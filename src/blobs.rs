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

pub async fn delete_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    storage: Extension<Arc<StorageDriver>>,
) -> impl IntoResponse {
    let exists = sqlx::query!("SELECT COUNT(*) as count from blobs join repositories r on r.id = (select id from repositories where name = ?) AND digest = ?", name, digest)
       .fetch_one(&mut *conn)
       .await
       .unwrap()
       .count > 0;
    if exists {
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

/// POST /v2/:name/blobs/uploads/
/// to get a session ID for uploading a blob
///
/// returns a 202 Accepted with Location header
/// that contains the session ID
/// <location>?digest=<digest>
/// location of the subsequent PUT request upload
pub async fn get_session_id(
    Path(name): Path<String>,
    Extension(blob_storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let session_id = blob_storage.new_session(&mut conn, &name).await;
    match session_id {
        Ok(session_id) => {
            let mut headers = HeaderMap::new();
            headers.append(
                LOCATION,
                format!("/v2/{}/blobs/uploads/{}", name, session_id)
                    .parse()
                    .unwrap(), // TODO: handle error better
            );
            (StatusCode::ACCEPTED, headers, session_id).into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::NameUnknown;
            ErrorResponse::from_code(&code, "respository name not found").into_response()
        }
    }
}

#[derive(serde::Deserialize)]
pub struct QueryParams {
    pub digest: Option<String>,
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
    Path((name, session_id)): Path<(String, String)>,
    Query(digest): Query<QueryParams>,
    Extension(blob_storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
    body: Multipart,
) -> impl IntoResponse {
    let digest = match digest.digest {
        Some(d) => d,
        None => {
            let code = crate::codes::Code::DigestInvalid;
            return ErrorResponse::from_code(&code, "digest missing").into_response();
        }
    };
    match blob_storage
        .write_blob(&mut conn, &name, &session_id, body)
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
                LOCATION,
                format!("/v2/{}/blobs/{}", name, digest).parse().unwrap(), // TODO: handle error better
            );
            (StatusCode::CREATED, headers, "resource created").into_response()
        }
        Err(_) => {
            let code = crate::codes::Code::BlobUploadUnknown;
            ErrorResponse::from_code(&code, "unable to upload blob").into_response()
        }
    }
}

/// POST /v2/:name/blobs/uploads?digest=<digest>
/// upload a blob using a single POST without SESSION ID
/// spec 289-322
pub async fn single_post_upload(
    Path(name): Path<String>,
    Query(digest): Query<QueryParams>,
    Extension(storage): Extension<StorageDriver>,
    DbConn(mut conn): DbConn,
    body: Multipart,
) -> impl IntoResponse {
    let digest = match digest.digest {
        Some(d) => d,
        None => {
            let code = crate::codes::Code::DigestInvalid;
            return ErrorResponse::from_code(&code, "digest missing").into_response();
        }
    };
    match storage.write_blob(&mut conn, &name, "", body).await {
        Ok(result_digest) => {
            if result_digest != digest {
                return (StatusCode::BAD_REQUEST, "Digest mismatch").into_response();
            }
            let mut headers = HeaderMap::new();
            headers.append(
                "Location",
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

/// POST /v2/:name/blobs/uploads/?mount=<digest>&from=<other_name>
/// mount a blob from another repository
/// returns a 201 Created with Location header
/// that contains the digest of the mounted blob
/// <location>?digest=<digest>
/// spec: 436-460
pub async fn mount_blob(
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    Extension(blob_storage): Extension<StorageDriver>,
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
