use crate::{
    codes::{Code, ErrorResponse},
    database::DbConn,
    storage_driver::Backend,
};
use axum::{
    extract::{Path, Query, Request},
    http::{header::LOCATION, HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error};

/// GET /v2/:name/blobs/:digest
/// to pull a blob from the registry
pub async fn get_blob(
    Path((name, digest)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
    Extension(blob_storage): Extension<Arc<Backend>>,
) -> impl IntoResponse {
    debug!("GET /v2/{}/blobs/{}", name, digest);
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
    debug!("HEAD /v2/{}/blobs/{}", name, digest);
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
    storage: Extension<Arc<Backend>>,
) -> impl IntoResponse {
    debug!("DELETE /v2/{}/blobs/{}", name, digest);
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
pub async fn upload_blob(
    Path(path): Path<BlobUpload>,
    Query(digest): Query<QueryParams>,
    Extension(blob_storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    request: Request,
) -> impl IntoResponse {
    debug!("PUT /v2/{}/blobs/uploads/{}", path.name, path.session_id);
    match blob_storage
        .write_blob(
            &path.name,
            &path.session_id,
            &mut conn,
            request.into_body().into_data_stream(),
        )
        .await
    {
        Ok(result_digest) => {
            if let Some(sha) = digest.digest {
                let result_digest = format!("sha256:{}", result_digest);
                if !result_digest.eq(&sha) {
                    error!("{} did not match {}", result_digest, sha);
                    let code = crate::codes::Code::DigestInvalid;
                    return ErrorResponse::from_code(&code, "digest did not match content")
                        .into_response();
                }
            }
            let mut headers = HeaderMap::new();
            headers.append(
                "Location",
                format!("/v2/{}/blobs/{}", path.name, result_digest)
                    .parse()
                    .unwrap(), // TODO: handle error better
            );
            (StatusCode::CREATED, headers, "resource created").into_response()
        }
        Err(err) => {
            error!("error uploading blob: {:?}", err);
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
///
/// QUERY /v2/:name/blobs/uploads/?mount=<digest>&from=<other_name>
/// mount a blob from another repository
/// returns a 201 Created with Location header
/// that contains the digest of the mounted blob
/// <location>?digest=<digest>
/// spec: 436-460
pub async fn handle_upload_blob(
    Path(name): Path<BlobPath>,
    Query(digest): Query<QueryParams>,
    Extension(storage): Extension<Arc<Backend>>,
    DbConn(mut conn): DbConn,
    headers: HeaderMap,
    request: Request,
) -> impl IntoResponse {
    let name = name.name;
    debug!("Handling blob upload for {name}");
    debug!("query params: {:?}", digest);
    debug!("headers: {:?}", headers);
    if let Some(sha) = digest.digest {
        if let Some(mount) = digest.mount {
            match storage
                .mount_blob(&mut conn, &name, &mount, digest.from.as_deref())
                .await
            {
                Ok(_) => {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "Location",
                        format!("/v2/{}/blobs/{}", name, mount).parse().unwrap(),
                    );
                    headers.insert("Docker-Content-Digest", mount.parse().unwrap());

                    (StatusCode::CREATED, headers).into_response()
                }
                Err(err) => {
                    error!("error uploading blob: {:?}", err);
                    let code = crate::codes::Code::BlobUploadUnknown;
                    ErrorResponse::from_code(&code, "unable to mount blob").into_response()
                }
            }
        } else {
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
                    (StatusCode::CREATED, headers, "resource created").into_response()
                }
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
    } else {
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
}
