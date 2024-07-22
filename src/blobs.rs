use crate::{
    codes::{Code, ErrorResponse},
    database::DbConn,
    storage::StorageDriver,
};
use axum::{
    extract::{Multipart, Path, Query},
    http::{header::LOCATION, HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// GET /v2/:name/blobs/:digest
/// to pull a blob from the registry
pub async fn get_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    Extension(blob_storage): Extension<Arc<StorageDriver>>,
) -> impl IntoResponse {
    match blob_storage.read_blob(&mut conn, &name, &digest).await {
        Ok(data) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Docker-Content-Digest",
                format!("sha256:{}", digest).parse().unwrap(),
            );
            (headers, data).into_response()
        }
        Err(_) => ErrorResponse::from_code(&Code::BlobUnknown, String::from("blob not found"))
            .into_response(),
    }
}

pub async fn check_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let exists = sqlx::query!("SELECT COUNT(*) as count from blobs join repositories r on r.id = (select id from repositories where name = ?) AND digest = ?", name, digest)
       .fetch_one(&mut *conn)
       .await
       .unwrap()
       .count > 0;
    if exists {
        (StatusCode::OK, "true").into_response()
    } else {
        ErrorResponse::from_code(&Code::BlobUnknown, String::from("blob not found")).into_response()
    }
}

/// DELETE /v2/:name/blobs/:digest
/// to delete a blob from the registry
/// spec: 705-712
pub async fn delete_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    storage: Extension<Arc<StorageDriver>>,
) -> impl IntoResponse {
    if sqlx::query!("SELECT COUNT(*) as count from blobs join repositories r on r.id = (select id from repositories where name = ?) AND digest = ?", name, digest)
       .fetch_one(&mut *conn)
       .await
       .is_ok_and(|row| row.count > 0) {
        storage
            .delete_blob(&mut conn, &name, &digest)
            .await
            .map_err(|_| {
                ErrorResponse::from_code(
                    &Code::BlobUnknown,
                    String::from("unable to find and delete blob"),
                )
                .into_response()
            })?;
        Ok((StatusCode::ACCEPTED, "deleted successfully").into_response())
    } else {
    Err(
        ErrorResponse::from_code(&Code::BlobUnknown, String::from("blob not found"))
            .into_response(),
    )
    }
}

#[derive(serde::Deserialize, Debug)]
pub struct QueryParams {
    pub digest: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct BlobUpload {
    pub name: String,
    pub session_id: String,
}

/// PUT /v2/:name/blobs/uploads/:session_id
/// to upload a blob
/// returns a 201 Created with Location header
/// that contains the digest of the uploaded blob
/// <location>?digest=<digest>
/// spec: 250-285
/// HEADERS:
///
/// Content-Length: <length>
/// Content-Type: application/octet-stream
pub async fn upload_blob(
    Path(path): Path<BlobUpload>,
    Query(digest): Query<QueryParams>,
    Extension(blob_storage): Extension<Arc<StorageDriver>>,
    DbConn(mut conn): DbConn,
    headers: HeaderMap,
    body: Option<Multipart>,
) -> impl IntoResponse {
    debug!("PUT /v2/{}/blobs/uploads/{}", path.name, path.session_id);
    debug!("headers: {:?}", headers);
    let digest = match digest.digest {
        Some(d) => d,
        None => {
            let code = crate::codes::Code::DigestInvalid;
            return ErrorResponse::from_code(&code, "digest missing").into_response();
        }
    };
    if !headers.contains_key("Content-Length") {
        let code = crate::codes::Code::BlobUploadUnknown;
        return ErrorResponse::from_code(&code, "missing content length").into_response();
    }
    if body.is_none() {
        let code = crate::codes::Code::BlobUploadUnknown;
        return ErrorResponse::from_code(&code, "missing content").into_response();
    }
    match blob_storage
        .write_blob(&mut conn, &path.name, &path.session_id, &mut body.unwrap())
        .await
    {
        Ok(result_digest) => {
            if !result_digest.eq(&digest) {
                let code = crate::codes::Code::DigestInvalid;
                return ErrorResponse::from_code(&code, "digest did not match content")
                    .into_response();
            }
            let mut headers = HeaderMap::new();
            headers.append(
                "Location",
                format!("/v2/{}/blobs/{}", path.name, digest)
                    .parse()
                    .unwrap(), // TODO: handle error better
            );
            (StatusCode::CREATED, headers, "resource created").into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::BlobUploadUnknown;
            ErrorResponse::from_code(&code, "unable to upload blob").into_response()
        }
    }
}
#[derive(serde::Deserialize)]
pub struct BlobPath {
    pub name: String,
}
/// POST /v2/:name/blobs/uploads/?digest=<digest>
/// if no digest is provided, create a new session and respond with a 202 Accepted
/// spec 289-322
pub async fn handle_upload_blob(
    Path(name): Path<BlobPath>,
    Query(digest): Query<QueryParams>,
    Extension(storage): Extension<Arc<StorageDriver>>,
    DbConn(mut conn): DbConn,
    headers: HeaderMap,
    mut body: Option<Multipart>,
) -> impl IntoResponse {
    let name = name.name;
    debug!("Handling blob upload for {name}");
    debug!("query params: {:?}", digest);
    debug!("headers: {:?}", headers);
    match digest.digest {
        None => {
            debug!("no digest, creating new uuid/session");
            let session_id = storage.new_session(&mut conn, &name).await;
            match session_id {
                Ok(session_id) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        String::from("LOCATION"),
                        format!("/v2/{name}/blobs/uploads/{session_id}"),
                    );
                    let headers: HeaderMap = HeaderMap::try_from(&headers).unwrap();
                    debug!("returning 202 with headers: {:?}", headers);
                    let response = (StatusCode::ACCEPTED, headers).into_response();
                    debug!("response: {:?}", response);
                    response
                }
                Err(_) => {
                    let code = crate::codes::Code::NameUnknown;
                    ErrorResponse::from_code(&code, "respository name not found").into_response()
                }
            }
        }
        Some(digest) => {
            debug!("digest provided, uploading blob");
            match storage
                .write_blob(&mut conn, &name, &digest, body.as_mut().unwrap())
                .await
            {
                Ok(result_digest) => {
                    if result_digest != digest {
                        return (StatusCode::BAD_REQUEST, "Digest mismatch").into_response();
                    }
                    let mut headers = HeaderMap::new();
                    headers.append(
                        LOCATION,
                        format!("/v2/{}/blobs/{}", name, digest).parse().unwrap(),
                    );
                    (StatusCode::CREATED, headers, "resource created").into_response()
                }
                Err(_) => ErrorResponse::from_code(
                    &crate::codes::Code::BlobUploadUnknown,
                    String::from("unable to upload blob"),
                )
                .into_response(),
            }
        }
    }
}

/// POST /v2/:name/blobs/uploads/?mount=<digest>&from=<other_name>
/// mount a blob from another repository
/// returns a 201 Created with Location header
/// that contains the digest of the mounted blob
/// <location>?digest=<digest>
/// spec: 436-460
pub async fn mount_blob(
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    Extension(blob_storage): Extension<Arc<StorageDriver>>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let digest = match params.get("mount") {
        Some(d) => d,
        None => {
            return (StatusCode::BAD_REQUEST, "Missing 'mount' parameter").into_response();
        }
    };
    let from = params.get("from").map(|s| s.as_str());
    match blob_storage
        .mount_blob(&mut conn, &name, digest, from)
        .await
    {
        Ok(_) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Location",
                format!("/v2/{}/blobs/{}", name, digest).parse().unwrap(),
            );
            headers.insert("Docker-Content-Digest", digest.parse().unwrap());

            (StatusCode::CREATED, headers).into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::BlobUploadUnknown;
            ErrorResponse::from_code(&code, "unable to mount blob").into_response()
        }
    }
}
