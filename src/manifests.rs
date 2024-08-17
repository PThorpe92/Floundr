use crate::{
    codes::ErrorResponse,
    database::DbConn,
    storage_driver::Backend,
    util::{is_digest, strip_sha_header},
};
use axum::{
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use http::header::CONTENT_TYPE;
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
            headers.insert(DOCKER_DIGEST, format!("sha256:{}", digest).parse().unwrap());
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
    if !is_digest(&reference) {
        if let Ok(record) = sqlx::query!("SELECT digest, file_path FROM manifests m JOIN tags t ON t.manifest_id = m.id WHERE t.repository_id = (SELECT id FROM repositories WHERE name = ?) AND t.tag = ?", name_ref, ref_reference)
       .fetch_one(&mut *conn)
       .await {
       let digest = record.digest;
       let file_path = record.file_path;
        match blob_storage.read_manifest(&file_path).await {
            Ok(data) => {
                headers.insert(DOCKER_DIGEST,  format!("sha256:{}", digest).parse().unwrap());
                headers.insert(CONTENT_TYPE, MANIFEST_CONTENT_TYPE.parse().unwrap());
                let resp = (StatusCode::OK, headers, data).into_response();
                tracing::info!("response: {:?}", resp);
                return resp;
                }
          Err(_) => {
                error!("unable to find manifest with provided file_path and digest {} : {}", file_path, digest);
                let code = crate::codes::Code::ManifestUnknown;
                ErrorResponse::from_code(&code, "unable to find manifest for image").into_response()
          }
      };
    }
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
    mut conn: DbConn,
) -> impl IntoResponse {
    let reference = strip_sha_header(&reference);
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
