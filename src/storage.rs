use crate::{
    storage_driver::StorageError,
    util::{calculate_digest, validate_digest},
};
use axum::body::BodyDataStream;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::{async_trait, BoxError};
use bytes::Bytes;
use futures::{Stream, StreamExt, TryStreamExt};
use shared::ImageManifest;
use sqlx::{query, SqliteConnection};
use std::io::{self};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::{fs::File, io::BufWriter};
use tokio_util::io::StreamReader;
use tracing::{debug, error, info};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct LocalStorageDriver {
    base_path: PathBuf,
}

#[async_trait]
impl<S> FromRequestParts<S> for LocalStorageDriver
where
    LocalStorageDriver: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let storage = LocalStorageDriver::from_ref(state);
        Ok(storage)
    }
}

fn visit(path: impl Into<PathBuf>) -> impl Stream<Item = io::Result<u64>> + Send + 'static {
    async fn one_level(path: PathBuf, to_visit: &mut Vec<PathBuf>) -> io::Result<Vec<u64>> {
        let mut dir = tokio::fs::read_dir(&path).await?;
        let mut files = Vec::new();

        while let Some(child) = dir.next_entry().await? {
            let metadata = child.metadata().await?;
            if metadata.is_dir() {
                to_visit.push(child.path());
            } else {
                let size = metadata.len();
                files.push(size);
            }
        }

        Ok(files)
    }
    futures::stream::unfold(vec![path.into()], |mut to_visit| async {
        let path = to_visit.pop()?;
        let file_stream = match one_level(path, &mut to_visit).await {
            Ok(files) => futures::stream::iter(files).map(Ok).left_stream(),
            Err(e) => futures::stream::once(async { Err(e) }).right_stream(),
        };

        Some((file_stream, to_visit))
    })
    .flatten()
}

impl LocalStorageDriver {
    pub fn new(base_path: &Path) -> Self {
        Self {
            base_path: PathBuf::from(base_path),
        }
    }

    pub async fn delete_repository(
        &self,
        name: &str,
        conn: &mut SqliteConnection,
    ) -> Result<(), StorageError> {
        let rows = query!("SELECT file_path FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE repositories.name = ?", name)
            .fetch_all(&mut *conn)
            .await?;
        for row in rows.iter() {
            tokio::fs::remove_file(&row.file_path).await?;
        }
        query!(
            "DELETE FROM blobs WHERE repository_id = (SELECT id from repositories WHERE name = ?)",
            name
        )
        .execute(&mut *conn)
        .await?;
        let rows = query!("SELECT file_path FROM manifests JOIN repositories ON manifests.repository_id = repositories.id WHERE repositories.name = ?", name)
            .fetch_all(&mut *conn)
            .await?;
        for row in rows.iter() {
            tokio::fs::remove_file(&row.file_path).await?;
        }
        query!("DELETE FROM manifests WHERE repository_id = (SELECT id from repositories WHERE name = ?)", name)
            .execute(&mut *conn)
            .await?;
        query!("DELETE FROM repositories WHERE name = ?", name)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    pub async fn get_dir_size(&self, path: impl Into<PathBuf>) -> u64 {
        visit(path)
            .fold(0u64, |acc, entry| async move {
                acc + entry.unwrap_or_else(|e| {
                    error!("error getting directory size: {e}");
                    0
                })
            })
            .await
    }
    async fn stream_to_file<S, E>(
        &self,
        path: &str,
        filename: &str,
        stream: S,
    ) -> Result<PathBuf, StorageError>
    where
        S: Stream<Item = Result<Bytes, E>>,
        E: Into<BoxError>,
    {
        async {
            let body_with_io_error =
                stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
            let body_reader = StreamReader::new(body_with_io_error);
            futures::pin_mut!(body_reader);
            let mut pathbuf = PathBuf::new();
            for new_path in path.split('/') {
                pathbuf = pathbuf.join(new_path);
            }
            if !self.base_path.join(path).exists() {
                tokio::fs::create_dir_all(self.base_path.join(path)).await?;
            }
            let path = self.base_path.join(path).join(filename);
            debug!("streaming to file: {:?}", path);
            let mut file = BufWriter::new(File::create(path.clone()).await?);

            tokio::io::copy(&mut body_reader, &mut file).await?;
            debug!("finished streaming to file completed: {:?}", path);
            Ok::<_, io::Error>(path)
        }
        .await
        .map_err(StorageError::IoError)
    }

    pub fn base_path(&self) -> &PathBuf {
        &self.base_path
    }

    pub async fn write_blob(
        &self,
        name: &str,
        session_id: &str,
        chunk: i64,
        pool: &mut SqliteConnection,
        data: BodyDataStream,
    ) -> Result<String, StorageError> {
        let rel_path = PathBuf::from(name)
            .join("blobs")
            .join(session_id)
            .to_string_lossy()
            .to_string();
        let path = self
            .stream_to_file(&rel_path, &format!("{}", chunk), data)
            .await?;
        let digest = calculate_digest(&tokio::fs::read(&path).await?);
        let file_path = self
            .base_path
            .join(rel_path)
            .join(format!("{chunk}"))
            .to_string_lossy()
            .to_string();
        let _ = query!("INSERT INTO blobs (repository_id, digest, file_path, upload_session_id) VALUES ((select id from repositories where name = ?), ?, ?, ?)", name, digest, file_path, session_id)
        .execute(pool)
        .await;
        Ok(digest)
    }

    pub async fn write_blob_without_session_id(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
        data: BodyDataStream,
    ) -> Result<String, StorageError> {
        let path = self.stream_to_file("blobs", digest, data).await?;
        validate_digest(&std::fs::read(&path)?, digest)?;
        let file_path = self
            .base_path
            .join(name)
            .join(digest)
            .to_string_lossy()
            .to_string();
        query!("INSERT INTO blobs (repository_id, digest, file_path) VALUES ((select id from repositories where name = ?), ?, ?)", name, digest, file_path)
        .execute(pool)
        .await?;
        Ok(digest.to_owned())
    }

    pub async fn read_blob(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
    ) -> Result<Vec<u8>, StorageError> {
        // Retrieve the file path from the database
        let row = query!("SELECT file_path FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", digest, name)
            .fetch_one(pool)
            .await?;

        let mut file = tokio::fs::File::open(row.file_path).await?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).await?;
        Ok(data)
    }

    pub async fn read_manifest(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let mut data = Vec::new();
        match File::open(path).await?.read_to_end(&mut data).await {
            Ok(_) => Ok(data),
            Err(e) => {
                error!("error reading manifest: {:?}", e);
                Err(StorageError::IoError(e))
            }
        }
    }

    pub async fn new_session(
        &self,
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<String, StorageError> {
        if query!(
            "SELECT COUNT(*) as count FROM repositories WHERE name = ?",
            name
        )
        .fetch_one(&mut *conn)
        .await?
        .count
            == 0
        {
            query!("INSERT INTO repositories (name) VALUES (?)", name)
                .execute(&mut *conn)
                .await?;
        }
        let session_id = Uuid::new_v4().to_string();
        info!("creating new session with id: {}", session_id);
        let new_dir = self.base_path.join(name).join("blobs").join(&session_id);
        debug!("creating new directory: {:?}", new_dir);
        if let Err(err) = std::fs::create_dir_all(&new_dir) {
            error!("Error creating directory: {:?}", err);
            return Err(StorageError::IoError(err));
        }
        query!("INSERT INTO uploads (repository_id, uuid) VALUES ((SELECT id FROM repositories WHERE name = ?), ?)", name, session_id)
            .execute(conn)
            .await?;
        Ok(session_id)
    }

    pub async fn combine_chunks(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        session_id: &str,
    ) -> Result<String, StorageError> {
        let rows = query!(
            "SELECT file_path, chunk_count FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE upload_session_id = ? AND repositories.name = ? ORDER BY chunk_count ASC",
            session_id, name
        )
        .fetch_all(&mut *pool)
        .await?;
        let mut data = Vec::new();
        for row in rows.iter() {
            let mut file = tokio::fs::File::open(&row.file_path).await?;
            let mut chunk_data = Vec::new();
            file.read_to_end(&mut chunk_data).await?;
            data.extend(chunk_data);
            sqlx::query!(
                "DELETE FROM blobs WHERE upload_session_id = ? AND chunk_count = ?",
                session_id,
                row.chunk_count
            )
            .execute(&mut *pool)
            .await?;
        }
        let digest = calculate_digest(&data);
        let file_path = self
            .base_path
            .join(name)
            .join("blobs")
            .join(&digest)
            .to_string_lossy()
            .to_string();
        tokio::fs::write(&file_path, &mut data).await?;
        let _ = query!("INSERT INTO blobs (repository_id, digest, file_path) VALUES ((select id from repositories where name = ?), ?, ?)", name, digest, file_path)
        .execute(pool)
        .await;
        Ok(digest)
    }

    pub async fn mount_blob(
        &self,
        pool: &mut SqliteConnection,
        target_name: &str,
        digest: &str,
        source_name: Option<&str>,
    ) -> Result<String, StorageError> {
        let row = if let Some(source_name) = source_name {
            sqlx::query!(
                "SELECT file_path FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?",
                digest, source_name
            )
            .fetch_one(&mut *pool)
            .await?.file_path
        } else {
            sqlx::query!("SELECT file_path FROM blobs WHERE digest = ?", digest)
                .fetch_one(&mut *pool)
                .await?
                .file_path
        };

        let target_exists = query!("SELECT COUNT(*) as count FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", digest, target_name)
            .fetch_optional(&mut *pool)
            .await?
            .is_some_and(|row| row.count > 0);
        if !target_exists {
            let target_repository_id =
                query!("SELECT id FROM repositories WHERE name = ?", target_name)
                    .fetch_one(&mut *pool)
                    .await?
                    .id;

            query!(
                "INSERT INTO blobs (repository_id, digest, file_path) VALUES (?, ?, ?)",
                target_repository_id,
                digest,
                row
            )
            .execute(pool)
            .await?;
        }
        Ok(row)
    }

    pub async fn write_manifest(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        reference: &str,
        data: BodyDataStream,
    ) -> Result<String, StorageError> {
        let path = self
            .stream_to_file(
                std::path::PathBuf::from(name)
                    .join("manifests")
                    .to_str()
                    .expect("invalid path"),
                reference,
                data,
            )
            .await?;
        info!("successfully wrote manifest to path: {:?}", path);
        let digest = calculate_digest(&tokio::fs::read(&path).await?);
        let img: ImageManifest = serde_json::from_str(&tokio::fs::read_to_string(&path).await?)
            .map_err(|_| {
                StorageError::IoError(std::io::Error::new(
                    io::ErrorKind::InvalidData,
                    "error deserializing into ImageManifest",
                ))
            })?;
        let cfg = img.config.unwrap_or_default();
        let file_path = path.to_string_lossy().to_string();
        let record = query!("INSERT INTO manifests (repository_id, digest, file_path, media_type, size, schema_version)
             VALUES ((select id from repositories where name = ?), ?, ?, ?, ?, ?)",
            name, digest, file_path, img.media_type, cfg.size, img.schema_version)
        .execute(&mut *pool)
        .await?;
        for layer in img.layers {
            query!("INSERT INTO manifest_layers (manifest_id, repository_id, digest, size, media_type) VALUES ((SELECT id from manifests WHERE digest = ?), (SELECT id from repositories where name = ?), ?, ?, ?)", digest, name, layer.digest, layer.size, layer.media_type).execute(&mut *pool).await?;
            query!(
                "UPDATE blobs SET ref_count = ref_count + 1 WHERE digest = ?",
                layer.digest
            )
            .execute(&mut *pool)
            .await?;
        }
        let id = record.last_insert_rowid();
        query!("INSERT OR REPLACE INTO tags (repository_id, tag, manifest_id) VALUES ((SELECT id from repositories where name = ?), ?, ?)", name, reference, id).execute(pool).await?;
        Ok(digest)
    }

    pub async fn delete_blob(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
    ) -> Result<(), StorageError> {
        let row = query!("SELECT file_path, blobs.id FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", digest, name)
            .fetch_one(&mut *pool)
            .await?;
        std::fs::remove_file(row.file_path)?;
        query!("DELETE FROM blobs WHERE id = ?", row.id)
            .execute(&mut *pool)
            .await?;
        Ok(())
    }

    pub async fn delete_manifest(&self, file_path: &str) -> Result<(), StorageError> {
        tokio::fs::remove_file(file_path).await?;
        Ok(())
    }

    pub async fn create_repository(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        is_pub: bool,
    ) -> Result<(), StorageError> {
        query!(
            "INSERT INTO repositories (name, is_public) VALUES (?, ?)",
            name,
            is_pub
        )
        .execute(pool)
        .await?;
        debug!("Created new repository: {}", name);
        let path = self.base_path.join(name);
        let paths = &[&path, &path.join("blobs"), &path.join("manifests")];
        for p in paths {
            match tokio::fs::create_dir_all(*p).await {
                Ok(_) => debug!("Created file path {:?} for test repository: {:?}", *p, name),
                Err(e) => debug!("Error creating new repository directory: {:?}", e),
            }
        }
        Ok(())
    }

    pub async fn run_garbage_collection(
        &self,
        pool: &mut SqliteConnection,
    ) -> Result<(), StorageError> {
        let rows = query!("SELECT digest, ref_count FROM blobs")
            .fetch_all(&mut *pool)
            .await?;
        for row in rows.iter() {
            if row.ref_count == 0 {
                let path = query!("SELECT file_path FROM blobs WHERE digest = ?", row.digest)
                    .fetch_one(&mut *pool)
                    .await?
                    .file_path;
                tokio::fs::remove_file(path).await?;
                query!("DELETE FROM blobs WHERE digest = ?", row.digest)
                    .execute(&mut *pool)
                    .await?;
            }
        }
        Ok(())
    }
}
