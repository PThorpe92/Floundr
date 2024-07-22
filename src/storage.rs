use axum::async_trait;
use axum::extract::{FromRef, FromRequestParts, Multipart};
use axum::http::request::Parts;
use axum::http::StatusCode;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::{query, SqliteConnection};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct StorageDriver {
    pub base_path: PathBuf,
}

pub fn calculate_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn validate_digest(data: &[u8], digest: &str) -> Result<(), Box<dyn std::error::Error>> {
    let calculated_digest = calculate_digest(data);
    if calculated_digest != digest {
        return Err("Digest mismatch".into());
    }
    Ok(())
}

#[async_trait]
impl<S> FromRequestParts<S> for StorageDriver
where
    StorageDriver: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let storage = StorageDriver::from_ref(state);
        Ok(storage)
    }
}

impl StorageDriver {
    pub fn new(base_path: &Path) -> Self {
        Self {
            base_path: PathBuf::from(base_path),
        }
    }

    pub async fn write_blob(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        session_id: &str,
        data: &mut Multipart,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let path = self.base_path.join(name).join("blobs").join(session_id);
        let mut file = File::create(&path)?;
        while let Some(mut field) = data.next_field().await? {
            while let Some(chunk) = field.next().await {
                file.write_all(&chunk?)?;
            }
        }
        let digest = calculate_digest(&std::fs::read(&path)?);
        let file_path = self
            .base_path
            .join(name)
            .join("blobs")
            .join(&digest)
            .to_string_lossy()
            .to_string();
        std::fs::rename(path, &file_path)?;
        query!("INSERT INTO blobs (repository_id, digest, file_path) VALUES ((select id from repositories where name = ?), ?, ?)", name, digest, file_path)
        .execute(pool)
        .await?;
        Ok(digest)
    }

    pub async fn write_blob_without_session_id(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
        mut data: Multipart,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.base_path.join(name).join("blobs").join(digest);
        let mut file = File::create(&path)?;
        while let Some(mut field) = data.next_field().await? {
            while let Some(chunk) = field.next().await {
                file.write_all(&chunk?)?;
            }
        }
        validate_digest(&std::fs::read(&path)?, digest)?;
        let file_path = self
            .base_path
            .join(name)
            .join("blobs")
            .join(digest)
            .to_string_lossy()
            .to_string();
        query!("INSERT INTO blobs (repository_id, digest, file_path) VALUES ((select id from repositories where name = ?), ?, ?)", name, digest, file_path)
        .execute(pool)
        .await?;
        Ok(())
    }
    pub async fn read_blob(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Retrieve the file path from the database
        let row = query!("SELECT file_path FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", digest, name)
            .fetch_one(pool)
            .await?;

        let mut file = File::open(row.file_path)?;
        let mut data = Vec::new();
        validate_digest(&data, digest)?;

        file.read_to_end(&mut data)?;
        Ok(data)
    }

    pub async fn read_manifest(
        &self,
        conn: &mut SqliteConnection,
        name: &str,
        digest: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let row = sqlx::query!("SELECT file_path FROM manifests m JOIN repositories r on m.repository_id = r.id WHERE m.digest = ? AND r.name = ?", digest, name)
            .fetch_one(conn)
            .await?;

        let mut file = File::open(row.file_path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }

    pub async fn new_session(
        &self,
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let session_id = Uuid::new_v4().to_string();
        info!("creating new session with id: {}", session_id);
        query!("INSERT INTO uploads (repository_id, uuid) VALUES ((SELECT id FROM repositories WHERE name = ?), ?)", name, session_id)
            .execute(conn)
            .await?;
        let new_dir = self.base_path.join(name).join(&session_id);
        debug!("creating new directory: {:?}", new_dir);
        match std::fs::create_dir_all(&new_dir) {
            Ok(_) => {
                debug!("Created new session directory: {:?}", session_id);
                Ok(session_id)
            }
            Err(e) => {
                error!("Error creating directory: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub async fn mount_blob(
        &self,
        pool: &mut SqliteConnection,
        target_name: &str,
        digest: &str,
        source_name: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
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
        mut data: Multipart,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let path = self.base_path.join(name).join(reference);
        let mut file = File::create(&path)?;
        while let Some(mut field) = data.next_field().await? {
            while let Some(chunk) = field.next().await {
                file.write_all(&chunk?)?;
            }
        }
        let digest = calculate_digest(&std::fs::read(&path)?);
        let file_path = self
            .base_path
            .join(name)
            .join(&digest)
            .to_string_lossy()
            .to_string();
        std::fs::rename(path, &file_path)?;
        query!("INSERT INTO manifests (repository_id, digest, file_path) VALUES ((select id from repositories where name = ?), ?, ?)", name, digest, file_path)
        .execute(pool)
        .await?;
        Ok(digest)
    }

    pub async fn delete_blob(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        digest: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let row = query!("SELECT file_path, blobs.id FROM blobs JOIN repositories ON blobs.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", digest, name)
            .fetch_one(&mut *pool)
            .await?;
        std::fs::remove_file(row.file_path)?;
        query!("DELETE FROM blobs WHERE id = ?", row.id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn delete_manifest(
        &self,
        pool: &mut SqliteConnection,
        name: &str,
        reference: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // reference can be either a tag OR a digest
        // check if its a tag
        let row = if let Ok(record) = query!("SELECT b.digest FROM tags JOIN repositories r ON tags.repository_id = r.id JOIN blobs b on b.id = tags.blob_id WHERE tag = ? AND r.name = ?", reference, name)
            .fetch_one(&mut *pool)
            .await {
            // now we have the digest, delete the manifest
            let row = query!("SELECT file_path, manifests.id as m_id FROM manifests JOIN repositories ON manifests.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", record.digest, name)
                .fetch_one(&mut *pool)
                .await?;
            (row.file_path, row.m_id)
        } else {
            // reference is a digest
            let row = query!("SELECT file_path, manifests.id as m_id FROM manifests JOIN repositories ON manifests.repository_id = repositories.id WHERE digest = ? AND repositories.name = ?", reference, name)
            .fetch_one(&mut *pool)
            .await?;
            (row.file_path, row.m_id)
        };
        std::fs::remove_file(row.0)?;
        query!("DELETE FROM manifests WHERE id = ?", row.1)
            .execute(pool)
            .await?;
        Ok(())
    }
}
