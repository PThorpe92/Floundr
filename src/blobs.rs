use crate::{
    codes::{Code, ErrorResponse},
    database::DbConn,
    storage_driver::{Backend, StorageError},
    util::{parse_content_length, parse_content_range, strip_sha_header},
};
use axum::{
    extract::{Path, Query, Request},
    http::{
        header::{CONTENT_LENGTH, LOCATION},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    Extension,
};
use http::header::RANGE;
use sqlx::SqliteConnection;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

/// GET | HEAD /v2/:name/blobs/:digest
/// to pull a blob from the registry
#[tracing::instrument(skip(blob_storage, conn))]
pub async fn get_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    Extension(blob_storage): Extension<Arc<Backend>>,
) -> impl IntoResponse {
    let digest = strip_sha_header(&digest);
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

#[tracing::instrument(skip(conn))]
pub async fn check_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    debug!("HEAD /v2/{}/blobs/{}", name, digest);
    let digest = strip_sha_header(&digest);
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
#[tracing::instrument(skip(storage, conn))]
pub async fn delete_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    storage: Extension<Arc<Backend>>,
) -> impl IntoResponse {
    debug!("DELETE /v2/{}/blobs/{}", name, digest);
    let digest = strip_sha_header(&digest);
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
    pub mount: Option<String>,
    pub from: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
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
#[tracing::instrument(skip(blob_storage))]
pub async fn put_upload_session_blob(
    Path((name, session_id)): Path<(String, String)>,
    Query(query): Query<QueryParams>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    request: Request,
) -> impl IntoResponse {
    // this could either be finishing an upload session (with a digest/body) or uploading an
    // entire chunk
    let range = parse_content_range(request.headers());
    let content_length = parse_content_length(request.headers());
    if query.digest.is_none() {
        return ErrorResponse::from_code(
            &crate::codes::Code::DigestInvalid,
            "digest required to close session",
        )
        .into_response();
    }
    if range == (0, 0) && content_length == 0 {
        // no content range header
        debug!("finishing upload session");
        return finish_upload_session(&name, &session_id, &mut conn, blob_storage).await;
    } else {
        let cloned = Arc::clone(&blob_storage);
        debug!("uploading chunk");
        match upload_chunk(&name, &session_id, cloned, &mut conn, request).await {
            Ok(result_digest) => {
                let digest = query.digest.unwrap();
                let result_digest = format!("sha256:{}", result_digest);
                if !result_digest.eq(&digest) {
                    error!("{} did not match {}", result_digest, digest);
                    let code = crate::codes::Code::DigestInvalid;
                    return ErrorResponse::from_code(&code, "digest did not match content")
                        .into_response();
                }
                return finish_upload_session(&name, &session_id, &mut conn, blob_storage).await;
            }
            Err(err) => {
                error!("error uploading blob: {:?}", err);
                let code = crate::codes::Code::BlobUploadUnknown;
                ErrorResponse::from_code(&code, "unable to upload blob").into_response()
            }
        }
    }
}

// POST /v2/:name/blobs/uploads/?digest=<digest>
// digest of entire blob chunks may be provided.
// this will finish the upload session after the last
// blob may or may not have been included/uploaded
#[tracing::instrument(skip(storage, conn))]
async fn finish_upload_session(
    name: &str,
    session_id: &str,
    conn: &mut SqliteConnection,
    storage: Arc<Backend>,
) -> Response {
    // we will have to combine any chunks that have been uploaded in this session
    // and then calculate the digest
    let digest = storage
        .combine_chunks(conn, name, session_id)
        .await
        .unwrap();
    let digest = format!("sha256:{}", digest);
    let location = format!("/v2/{}/blobs/{}", name, digest);
    let mut return_headers = HeaderMap::new();
    return_headers.insert(LOCATION, location.parse().unwrap());
    (StatusCode::CREATED, return_headers, "resource created").into_response()
}

/// PUT /v2/:name/blobs/:session_id?digest=<digest>
/// because this closes the session, we also need to combine the chunks after
pub async fn put_upload_blob(
    Path((name, session_id)): Path<(String, String)>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    let digest = req
        .headers()
        .get("digest")
        .map(|v| v.to_str().unwrap_or("sha256:").to_string())
        .unwrap_or("sha256:".to_string());
    let content_len = parse_content_length(req.headers());
    match storage
        .write_blob(
            &name,
            &session_id,
            content_len,
            &mut conn,
            req.into_body().into_data_stream(),
        )
        .await
    {
        Ok(_) => match storage.combine_chunks(&mut conn, &name, &session_id).await {
            Ok(combined_digest) => {
                if !combined_digest.eq(&digest) {
                    info!(
                        "combined chunks digest did not equal digest given:\n {} != {}",
                        combined_digest, digest
                    );
                    let code = crate::codes::Code::DigestInvalid;
                    return ErrorResponse::from_code(&code, "digest did not match content")
                        .into_response();
                }
                let mut headers = HeaderMap::new();
                headers.insert(
                    LOCATION,
                    format!("/v2/{}/blobs/{}", name, combined_digest)
                        .parse()
                        .unwrap(),
                );
                headers.insert("Docker-Content-Digest", combined_digest.parse().unwrap());
                (StatusCode::CREATED, headers, "resource created").into_response()
            }
            Err(err) => {
                error!("error combining chunks: {:?}", err);
                ErrorResponse::from_code(
                    &crate::codes::Code::BlobUploadUnknown,
                    String::from("unable to combine chunks"),
                )
                .into_response()
            }
        },
        Err(err) => {
            error!("error uploading blob: {:?}", err);
            ErrorResponse::from_code(
                &crate::codes::Code::BlobUploadUnknown,
                String::from("unable to upload blob"),
            )
            .into_response()
        }
    }
}

#[tracing::instrument(skip(storage, conn))]
async fn upload_chunk(
    name: &str,
    session_id: &str,
    storage: Arc<Backend>,
    conn: &mut SqliteConnection,
    req: Request,
) -> Result<String, StorageError> {
    tracing::info!("blobs.rs: upload_chunk... {name} : {session_id}");
    let headers = req.headers().clone();
    let range = parse_content_range(&headers);
    if let Ok(current_session) = sqlx::query!(
        "SELECT current_chunk FROM uploads where uuid = ?",
        session_id,
    )
    .fetch_one(&mut *conn)
    .await
    {
        let current_chunk = current_session.current_chunk;
        // ensure that we are not out of order
        if range.0 == current_chunk || range.0 == 0 {
            let content_len = headers
                .get(CONTENT_LENGTH)
                .map(|v| v.to_str().unwrap_or("0"))
                .unwrap_or("0")
                .parse::<i64>()
                .unwrap_or(0);
            let chunk = if range.0 == 0 { content_len } else { range.1 };
            storage
                .write_blob(
                    name,
                    session_id,
                    chunk,
                    &mut *conn,
                    req.into_body().into_data_stream(),
                )
                .await
        } else {
            Err(StorageError::OutOfOrder)
        }
    } else {
        Err(StorageError::OutOfOrder)
    }
}

// PATCH /v2/:name/blobs/uploads/:session_id
// requires Content-Length & Content-Range headers
#[tracing::instrument(skip(storage, conn))]
pub async fn handle_upload_session_chunk(
    Path((name, session_id)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    storage: Extension<Arc<Backend>>,
    request: Request,
) -> impl IntoResponse {
    let headers = request.headers().clone();
    let range = parse_content_range(&headers);
    let content_len = parse_content_length(&headers);
    match upload_chunk(&name, &session_id, storage.0, &mut conn, request).await {
        Ok(_) => {
            let next_chunk = if range.1 == 0 { content_len } else { range.1 };
            sqlx::query!(
                "UPDATE uploads SET current_chunk = ? WHERE uuid = ?",
                next_chunk,
                session_id
            )
            .execute(&mut *conn)
            .await
            .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                format!("/v2/{}/blobs/uploads/{}", name, session_id)
                    .parse()
                    .unwrap(),
            );
            headers.insert(RANGE, format!("0-{}", next_chunk).parse().unwrap());
            headers.insert(CONTENT_LENGTH, "0".parse().unwrap());
            headers.insert("Docker-Upload-UUID", session_id.parse().unwrap());
            let resp = (StatusCode::ACCEPTED, headers).into_response();
            info!("{:?}", resp);
            resp
        }
        Err(err) => {
            error!("error uploading blob: {:?}", err);
            let code = crate::codes::Code::BlobUploadUnknown;
            ErrorResponse::from_code(&code, "unable to upload blob").into_response()
        }
    }
}

/// POST /v2/:name/blobs/uploads/?digest=<digest>
/// if no digest is provided, create a new session and respond with a 202 Accepted
/// spec 289-322
///
/// QUERY /v2/:name/blobs/uploads/?mount=<digest>&from=<other_name>
/// mount a blob from another repository
/// returns a 201 Created with Location header
/// that contains the digest of the mounted blob
/// <location>?digest=<digest>
/// spec: 436-460
#[tracing::instrument(skip(storage, conn))]
pub async fn handle_upload_blob(
    Path(name): Path<String>,
    Query(digest): Query<QueryParams>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    request: Request,
) -> impl IntoResponse {
    if let Some(sha) = digest.digest {
        debug!("digest provided, uploading blob");
        match storage
            .write_blob_without_session_id(
                &mut conn,
                &name,
                &sha,
                request.into_body().into_data_stream(),
            )
            .await
        {
            Ok(result_digest) => {
                if !result_digest.eq(&sha) {
                    let code = crate::codes::Code::DigestInvalid;
                    return ErrorResponse::from_code(&code, "digest did not match content")
                        .into_response();
                }
                let mut headers = HeaderMap::new();
                headers.append(
                    LOCATION,
                    format!("/v2/{}/blobs/{}", name, sha).parse().unwrap(),
                );
                return (StatusCode::CREATED, headers, "resource created").into_response();
            }
            Err(err) => {
                error!("error uploading blob: {:?}", err);
                return ErrorResponse::from_code(
                    &crate::codes::Code::BlobUploadUnknown,
                    String::from("unable to upload blob"),
                )
                .into_response();
            }
        }
    }
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
        Err(err) => {
            error!("error uploading blob: {:?}", err);
            let code = crate::codes::Code::NameUnknown;
            ErrorResponse::from_code(&code, "respository name not found").into_response()
        }
    }
}
