use std::sync::Arc;

use crate::{
    codes::ErrorResponse,
    database::DbConn,
    storage_driver::{Backend, DriverType},
    util::{get_dir_size, DOCKER_DIGEST, OCI_CONTENT_HEADER},
};
use axum::{
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use http::header::CONTENT_TYPE;
use serde::Serialize;
use tracing::debug;

#[derive(Debug, Serialize)]
pub struct RepoList {
    pub repositories: Vec<Repo>,
}

#[derive(Debug, Serialize)]
pub struct Repo {
    pub name: String,
    pub is_public: bool,
    pub blob_count: i64,
    pub tag_count: i64,
    pub tags: Vec<String>,
    pub manifest_count: i64,
    pub file_path: String,
    pub disk_usage: u64,
    pub driver: DriverType,
}

#[derive(Debug, Serialize)]
pub struct NewRepoQuery {
    pub is_public: Option<String>,
}

pub async fn create_repository(
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
    Path((name, public)): Path<(String, String)>,
) -> impl IntoResponse {
    debug!("POST /repositories/{}", name);
    match storage
        .create_repository(&mut conn, &name, public.to_lowercase().eq("true"))
        .await
    {
        Ok(_) => (StatusCode::CREATED, "repository created").into_response(),
        Err(_) => (StatusCode::BAD_REQUEST, "invalid request").into_response(),
    }
}

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Debug)]
pub struct ImageManifest {
    pub schema_version: i32,
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug)]
pub struct Descriptor {
    pub media_type: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Deserialize, Debug)]
pub struct ImageConfig {
    pub architecture: String,
    pub os: String,
    pub created: Option<String>,
    pub author: Option<String>,
    pub config: Config,
    pub rootfs: RootFS,
    pub history: Vec<History>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub env: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug)]
pub struct RootFS {
    pub type_: String,
    pub diff_ids: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct History {
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub author: Option<String>,
    pub comment: Option<String>,
}
pub async fn list_repositories(
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
) -> impl IntoResponse {
    debug!("GET /repositories \n listing repositories");
    let repositories = sqlx::query!("SELECT id, name, is_public, (SELECT COUNT(*) from blobs where blobs.repository_id = repositories.id) as blob_count, (SELECT COUNT(*) from tags WHERE tags.repository_id = repositories.id) as tag_count, (SELECT COUNT(m.id) from manifests m WHERE m.repository_id = id) as manifest_count FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    let mut names = Vec::new();
    for repo in repositories {
        let row = sqlx::query!("SELECT tag from tags t WHERE t.repository_id = ?", repo.id)
            .fetch_all(&mut *conn)
            .await
            .unwrap();
        let tags = row.iter().map(|t| t.tag.clone()).collect::<Vec<String>>();
        let disk_usage = get_dir_size(storage.base_path().join(&repo.name)).await;
        names.push(Repo {
            name: repo.name.clone(),
            is_public: repo.is_public,
            blob_count: repo.blob_count,
            tag_count: repo.tag_count,
            tags,
            file_path: format!("{}/{}", storage.base_path().to_string_lossy(), repo.name),
            manifest_count: repo.manifest_count,
            disk_usage,
            driver: storage.kind(),
        })
    }
    let response = serde_json::to_string(&RepoList {
        repositories: names,
    })
    .unwrap();
    (StatusCode::OK, response).into_response()
}

/// PUT /v2/:name/manifests/:reference
pub async fn push_manifest(
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    body: Request,
) -> impl IntoResponse {
    debug!(
        "PUT: uploading manifest for {} : reference: {}",
        name, reference
    );
    match storage
        .write_manifest(
            &mut conn,
            &name,
            &reference,
            body.into_body().into_data_stream(),
        )
        .await
    {
        Ok(digest) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Location",
                format!("/v2/{}/manifests/{}", name, digest)
                    .parse()
                    .unwrap(),
            );
            headers.insert("Docker-Content-Digest", digest.parse().unwrap());
            (StatusCode::CREATED, headers).into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to upload manifest").into_response()
        }
    }
}

/// To pull an image from the registry, the client must send a GET request to the `/v2/<name>/manifests/<reference>`
/// endpoint. The server must return the manifest of the image specified by the name and reference.

/// GET /v2/:name/manifests/:reference
/// spec: 145-184
pub async fn get_manifest(
    Path((name, digest)): Path<(String, String)>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    match blob_storage.read_manifest(&mut conn, &name, &digest).await {
        Ok(data) => {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, OCI_CONTENT_HEADER.parse().unwrap());
            headers.insert(DOCKER_DIGEST, digest.parse().unwrap());
            (StatusCode::OK, headers, data).into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
        }
    }
}

pub async fn delete_manifest(
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    match storage.delete_manifest(&mut conn, &name, &reference).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::NOT_FOUND,
    }
}
