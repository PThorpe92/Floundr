use crate::{codes::ErrorResponse, database::DbConn, storage_driver::Backend};
use axum::{
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use shared::{DOCKER_DIGEST, MANIFEST_CONTENT_TYPE};
use std::sync::Arc;
use tracing::{error, info};

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
            info!(
                "manifest written to storage with digest: {} for image: {}",
                digest, reference
            );
            headers.insert(
                "Location",
                format!("/v2/{}/manifests/{}", name, reference)
                    .parse()
                    .unwrap(),
            );
            headers.insert(DOCKER_DIGEST, digest.parse().unwrap());
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
    Path((name, reference)): Path<(String, String)>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    if let Ok(record) = sqlx::query!("SELECT file_path, digest, tags.tag FROM manifests JOIN tags on tags.manifest_id = manifests.id WHERE manifests.repository_id = (SELECT id FROM repositories WHERE name = ?) AND (digest = $2 OR tags.tag = $2)", name, reference)
          .fetch_one(&mut *conn)
          .await {
        info!("found manifest for image reference: {} with file path : {:?}", reference, record.file_path);
        headers.insert(DOCKER_DIGEST, record.digest.parse().unwrap());
        headers.insert(CONTENT_TYPE, MANIFEST_CONTENT_TYPE.parse().unwrap());
        match *req.method() {
            http::Method::HEAD => {
            return (StatusCode::OK, headers).into_response();
            }
            _ => {
            match blob_storage.read_manifest(&record.file_path).await {
                Ok(data) => {
                    info!("manifest read from storage for image: {}", reference);
                    headers.insert(CONTENT_LENGTH, data.len().into());
                    return (StatusCode::OK, headers, data).into_response();
                }
                Err(_) => {
                    error!("unable to find manifest with provided file_path and digest {} : {}", record.file_path, record.digest);
                    let code = crate::codes::Code::ManifestUnknown;
                    return ErrorResponse::from_code(&code, "unable to find manifest for image").into_response();
                }
            };
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
    mut conn: DbConn,
) -> impl IntoResponse {
    match conn.delete_manifest(&name, &reference).await {
        Ok(file_path) => {
            if let Err(err) = storage.delete_manifest(&file_path).await {
                error!(
                    "unable to delete manifest for image: {} \n {err}",
                    reference
                );
            }
            info!("deleted manifest for image: {}", reference);
            (StatusCode::NO_CONTENT).into_response()
        }
        Err(e) => {
            error!("unable to delete manifest for image: {} \n {e}", reference);
            ErrorResponse::from_code(
                &crate::codes::Code::ManifestUnknown,
                "unable to delete manifest for image",
            )
            .into_response()
        }
    }
}
