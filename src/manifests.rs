use crate::{
    codes::ErrorResponse,
    database::DbConn,
    storage::{validate_digest, StorageDriver},
};
use axum::{
    extract::{Multipart, Path},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RepoList {
    pub repositories: Vec<Repo>,
}

#[derive(Debug, Serialize)]
pub struct Repo {
    pub name: String,
    pub is_public: bool,
}

pub async fn list_repositories(DbConn(mut conn): DbConn) -> impl IntoResponse {
    let repositories = sqlx::query!("SELECT name, is_public FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    let mut names = Vec::new();
    for repo in repositories {
        names.push(Repo {
            name: repo.name,
            is_public: repo.is_public,
        })
    }
    let response = serde_json::to_string(&RepoList {
        repositories: names,
    })
    .unwrap();
    (StatusCode::OK, response).into_response()
}

pub async fn push_manifest(
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
    body: Multipart,
) -> impl IntoResponse {
    match storage
        .write_manifest(&mut conn, &name, &reference, body)
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
    Extension(blob_storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    match blob_storage.read_manifest(&mut conn, &name, &digest).await {
        Ok(data) => match validate_digest(&data, &digest) {
            Ok(()) => {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Content-Type",
                    "application/vnd.oci.image.manifest.v1+json"
                        .parse()
                        .unwrap(),
                );
                headers.insert("Docker-Content-Digest", digest.parse().unwrap());
                (StatusCode::OK, headers, data).into_response()
            }
            Err(_) => (StatusCode::BAD_REQUEST, "Digest validation failed").into_response(),
        },
        Err(_) => (StatusCode::NOT_FOUND, "Manifest not found").into_response(),
    }
}

pub async fn delete_manifest(
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    match storage.delete_manifest(&mut conn, &name, &reference).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::NOT_FOUND,
    }
}
