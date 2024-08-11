use crate::{
    codes::ErrorResponse,
    database::DbConn,
    storage_driver::Backend,
    util::{strip_sha_header, DOCKER_DIGEST, MANIFEST_CONTENT_TYPE, OCI_CONTENT_HEADER},
};
use axum::{
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use http::header::{ACCEPT, CONTENT_TYPE};
use std::sync::Arc;
use tracing::{error, info};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug)]
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

#[derive(Deserialize, Serialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: Option<String>,
    pub size: i32,
    pub digest: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
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

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub env: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RootFS {
    pub type_: String,
    pub diff_ids: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub author: Option<String>,
    pub comment: Option<String>,
}

/// PUT /v2/:name/manifests/:reference
pub async fn push_manifest(
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    body: Request,
) -> impl IntoResponse {
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
            info!(
                "manifest written to storage with digest: {} for image: {}",
                digest, reference
            );
            headers.insert(
                "Location",
                format!("/v2/{}/manifests/{}", name, digest)
                    .parse()
                    .unwrap(),
            );
            headers.insert(
                "Docker-Content-Digest",
                format!("sha256:{}", digest).parse().unwrap(),
            );
            (StatusCode::CREATED, headers).into_response()
        }
        Err(err) => {
            error!("Error writing manifest: {:?}", err);
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to upload manifest").into_response()
        }
    }
}

/// To pull an image from the registry, the client must send a GET request to the `/v2/<name>/manifests/<reference>`
/// endpoint. The server must return the manifest of the image specified by the name and reference.
/// GET /v2/:name/manifests/:reference
/// spec: 145-184
#[tracing::instrument(skip(conn, blob_storage))]
pub async fn get_manifest(
    Path((name, digest)): Path<(String, String)>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    let reference = strip_sha_header(&digest);
    // we need to figure out if the reference is a digest or a tag
    // if it is a tag, we need to look up the digest
    // if it is a digest, we can look up the manifest directly
    let mut headers = HeaderMap::new();
    let name_ref = &name;
    let ref_reference = &reference;
    if let Ok(record) = sqlx::query!("SELECT digest, file_path FROM manifests m JOIN tags t ON t.manifest_id = m.id WHERE t.repository_id = (SELECT id FROM repositories WHERE name = ?) AND t.tag = ?", name_ref, ref_reference)
       .fetch_one(&mut *conn)
       .await {
       let digest = record.digest;
       let file_path = record.file_path;
        match blob_storage.read_manifest(&file_path).await {
            Ok(data) => {
            headers.insert(DOCKER_DIGEST,  digest.parse().unwrap());
            headers.insert(CONTENT_TYPE, MANIFEST_CONTENT_TYPE.parse().unwrap());
                    match *req.method() {
                        http::Method::HEAD => { return (StatusCode::OK, headers).into_response(); }
                        http::Method::GET =>  {
                            let resp = (StatusCode::OK, headers, data).into_response();
                            tracing::info!("response: {:?}", resp);
                            return resp;
                            }
                    _ => unreachable!(),
                     }
                    }
          Err(_) => {
                error!("unable to find manifest with provided file_path and digest {} : {}", file_path, digest);
                let code = crate::codes::Code::ManifestUnknown;
                ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
          }
      };
    }
    if let Ok(record) = sqlx::query!("SELECT file_path, digest FROM manifests WHERE repository_id = (SELECT id FROM repositories WHERE name = ?) AND digest = ?", name, reference)
          .fetch_one(&mut *conn)
          .await {
        info!("found manifest for image reference: {}", reference);
        match blob_storage.read_manifest(&record.file_path).await {
            Ok(data) => {
            headers.insert(DOCKER_DIGEST, format!("sha256:{}", record.digest).parse().unwrap());
            headers.insert(CONTENT_TYPE, MANIFEST_CONTENT_TYPE.parse().unwrap());
                match *req.method()  {
                    http::Method::HEAD => return (StatusCode::OK, headers).into_response(),
                    _ => return (StatusCode::OK, headers, data).into_response(),
                }
            }
            Err(_) => {
            error!("unable to find manifest with provided file_path and digest {} : {}", record.file_path, record.digest);
            let code = crate::codes::Code::ManifestUnknown;
            ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
            }
       };
    }
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
    Path((name, reference)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let reference = strip_sha_header(&reference);
    match storage.delete_manifest(&mut conn, &name, &reference).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::NOT_FOUND,
    }
}
