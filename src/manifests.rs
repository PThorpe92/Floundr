use crate::{
    auth::{check_auth, Auth},
    codes::ErrorResponse,
    database::DbConn,
    storage_driver::{Backend, DriverType},
    util::{get_dir_size, strip_sha_header, DOCKER_DIGEST, OCI_CONTENT_HEADER},
};
use axum::{
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use http::header::CONTENT_TYPE;
use serde::Serialize;
use std::sync::Arc;
use tracing::{debug, error, info};

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
    pub num_layers: i64,
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
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    pub schema_version: i32,
    pub media_type: Option<String>,
    pub config: Option<Descriptor>,
    pub layers: Vec<Descriptor>,
    pub annotations: Option<HashMap<String, String>>,
}
impl Default for ImageManifest {
    fn default() -> Self {
        ImageManifest {
            schema_version: 2,
            media_type: Some("application/vnd.oci.image.manifest.v1+json".to_string()),
            config: Some(Descriptor {
                media_type: Some("application/vnd.oci.image.config.v1+json".to_string()),
                size: 0,
                digest: "".to_string(),
            }),
            layers: Vec::new(),
            annotations: None,
        }
    }
}

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: Option<String>,
    pub size: i32,
    pub digest: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    pub architecture: String,
    pub os: String,
    pub created: Option<String>,
    pub author: Option<String>,
    pub config: Config,
    pub rootfs: RootFS,
    pub history: Vec<History>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub env: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RootFS {
    pub type_: String,
    pub diff_ids: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub author: Option<String>,
    pub comment: Option<String>,
}

pub async fn list_repositories(
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
    req: Request,
) -> impl IntoResponse {
    let auth = req.extensions().get::<Auth>();
    if auth.is_some_and(|a| !a.is_valid()) {
        // list only public repos
        return list_public_repos(DbConn(conn), Extension(storage))
            .await
            .into_response();
    };
    let repos = sqlx::query!(r"SELECT id, name, is_public, (SELECT COUNT(*) from blobs where blobs.repository_id = repositories.id) as blob_count,
(SELECT COUNT(*) from tags WHERE tags.repository_id = repositories.id) as tag_count, (SELECT COUNT(m.id) from manifests m WHERE m.repository_id = id) as manifest_count,
(SELECT COUNT(*) from manifest_layers ml JOIN manifests m ON ml.manifest_id = m.id WHERE m.repository_id = ml.id) as num_layers FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    let mut names = Vec::new();
    for repo in repos {
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
            num_layers: repo.num_layers,
            driver: storage.kind(),
        })
    }
    let response = serde_json::to_string(&RepoList {
        repositories: names,
    })
    .unwrap();
    (StatusCode::OK, response).into_response()
}

async fn list_public_repos(
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
) -> impl IntoResponse {
    info!("listing public repositories!!");
    let repos = sqlx::query!("SELECT id, name, is_public, (SELECT COUNT(*) from blobs where blobs.repository_id = repositories.id) as blob_count,
(SELECT COUNT(*) from tags WHERE tags.repository_id = repositories.id) as tag_count, (SELECT COUNT(m.id) from manifests m WHERE m.repository_id = id) as manifest_count,
(SELECT COUNT(*) from manifest_layers ml JOIN manifests m ON ml.manifest_id = m.id WHERE m.repository_id = ml.id) as num_layers FROM repositories WHERE is_public = true")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    let mut names = Vec::new();
    for repo in repos {
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
            num_layers: repo.num_layers,
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
    Extension(claims): Extension<Auth>,
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    body: Request,
) -> impl IntoResponse {
    check_auth(claims, &name, &mut conn).await.unwrap();
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
            // we are calculating sha digest of the manifest and returning that in the header
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
    Extension(claims): Extension<Auth>,
    Path((name, digest)): Path<(String, String)>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    check_auth(claims, &name, &mut conn).await.unwrap();
    let reference = strip_sha_header(&digest);
    // we need to figure out if the reference is a digest or a tag
    // if it is a tag, we need to look up the digest
    // if it is a digest, we can look up the manifest directly
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, OCI_CONTENT_HEADER.parse().unwrap());
    if let Ok(record) = sqlx::query!("SELECT digest, file_path FROM manifests m JOIN tags t ON t.manifest_id = m.id WHERE t.repository_id = (SELECT id FROM repositories WHERE name = ?) AND t.tag = ?", name, reference)
       .fetch_one(&mut *conn)
       .await {
       let digest = record.digest;
       let file_path = record.file_path;
        match blob_storage.read_manifest(&file_path).await {
            Ok(data) => {
            info!("manifest found for image: {} with digest: {} and path {}", name, digest, file_path);
            headers.insert(DOCKER_DIGEST, format!("sha256:{}", digest).parse().unwrap());
            match *req.method() {
                http::Method::HEAD => return (StatusCode::OK, headers).into_response(),
                http::Method::GET => return (StatusCode::OK, headers, data).into_response(),
                _ => unreachable!(),
            }
            }
            Err(_) => {
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
            }
        };
    };
    if let Ok(record) = sqlx::query!("SELECT file_path, digest FROM manifests WHERE repository_id = (SELECT id FROM repositories WHERE name = ?) AND digest = ?", name, reference)
          .fetch_one(&mut *conn)
          .await {
        info!("found manifest for image reference: {}", reference);
        match blob_storage.read_manifest(&record.file_path).await {
            Ok(data) => {
            headers.insert(DOCKER_DIGEST, format!("sha256:{}", record.digest).parse().unwrap());
            match *req.method()  {
                http::Method::HEAD => (StatusCode::OK, headers).into_response(),
                _ => (StatusCode::OK, headers, data).into_response(),
            }
            }
            Err(_) => {
            error!("unable to find manifest with provided file_path and digest {} : {}", record.file_path, record.digest);
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
            }
       };
    };
    ErrorResponse::from_code(
        &crate::codes::Code::ManifestUnknown,
        "unable to find manifest for image",
    )
    .into_response()
}

/// DELETE /v2/:name/manifests/:reference
/// digest or tag can be used as reference
/// spec: 688-715
pub async fn delete_manifest(
    Extension(claims): Extension<Auth>,
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    check_auth(claims, &name, &mut conn).await.unwrap();
    let reference = strip_sha_header(&reference);
    match storage.delete_manifest(&mut conn, &name, &reference).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::NOT_FOUND,
    }
}
