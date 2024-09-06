use crate::{
    auth::{
        auth_middleware, auth_token_get, check_scope_middleware, get_auth_clients, login_user,
        register_user, Auth,
    },
    blobs::{
        check_blob, delete_blob, get_blob, handle_upload_blob, handle_upload_session_chunk,
        put_upload_blob, put_upload_session_blob,
    },
    content_discovery::{
        create_repository, delete_repository, get_tags_list, get_v2, list_repositories,
    },
    manifests::{delete_manifest, get_manifest, push_manifest},
    storage_driver::Backend,
    users::{delete_user, generate_token, get_users},
};
use axum::{
    extract::{Extension, Host},
    handler::HandlerWithoutStateExt,
    http::{StatusCode, Uri},
    middleware::from_fn,
    response::Redirect,
    routing::{delete, get, head, patch, post, put},
    BoxError, Router,
};
use http::Request;
use sqlx::SqlitePool;
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

#[derive(Clone, Copy)]
pub struct Ports(pub u16, pub u16);

#[allow(dead_code)]
pub async fn redirect_http_to_https(ports: Ports) {
    fn make_https(host: String, uri: axum::http::Uri, ports: Ports) -> Result<Uri, BoxError> {
        let mut parts = uri.into_parts();

        parts.scheme = Some(axum::http::uri::Scheme::HTTPS);

        if parts.path_and_query.is_none() {
            parts.path_and_query = Some("/".parse().unwrap());
        }

        let https_host = host.replace(&ports.0.to_string(), &ports.1.to_string());
        parts.authority = Some(https_host.parse()?);

        Ok(Uri::from_parts(parts)?)
    }

    let redirect = move |Host(host): Host, uri: Uri| async move {
        match make_https(host, uri, ports) {
            Ok(uri) => Ok(Redirect::permanent(&uri.to_string())),
            Err(error) => {
                tracing::warn!(%error, "failed to convert URI to HTTPS");
                Err(StatusCode::BAD_REQUEST)
            }
        }
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], ports.0));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, redirect.into_make_service())
        .await
        .unwrap();
}
#[derive(Debug)]
pub enum Endpoint {
    GetV2,
    HeadBlobs,
    GetBlobs,
    HeadManifests,
    GetManifests,
    PostBlobsUploads,
    PostBlobsUploadsWithDigest,
    PatchBlobsUploads,
    PutBlobsUploadsWithDigest,
    PutBlobsNoSession,
    PutManifests,
    GetTagsList,
    GetTagsListWithParams,
    DeleteManifests,
    DeleteBlobs,
    PostBlobsUploadsMount,
    GetReferrers,
    GetReferrersWithArtifactType,
    GetBlobsUploads,
}

impl Endpoint {
    pub fn to_handler(&self) -> axum::routing::MethodRouter<sqlx::Pool<sqlx::Sqlite>> {
        match self {
            Endpoint::GetV2 => get(get_v2),
            Endpoint::HeadBlobs => head(check_blob),
            Endpoint::GetBlobs => get(get_blob),
            Endpoint::HeadManifests => head(check_blob),
            Endpoint::GetManifests => get(get_manifest),
            Endpoint::PostBlobsUploads => post(handle_upload_blob),
            Endpoint::PostBlobsUploadsWithDigest => post(handle_upload_blob),
            Endpoint::PatchBlobsUploads => patch(handle_upload_session_chunk),
            Endpoint::PutBlobsUploadsWithDigest => put(put_upload_session_blob),
            Endpoint::PutBlobsNoSession => put(put_upload_blob),
            Endpoint::PutManifests => put(push_manifest),
            Endpoint::GetTagsList => get(get_tags_list),
            Endpoint::GetTagsListWithParams => get(get_tags_list),
            Endpoint::DeleteManifests => delete(delete_manifest),
            Endpoint::DeleteBlobs => delete(delete_blob),
            Endpoint::PostBlobsUploadsMount => post(handle_upload_blob),
            Endpoint::GetReferrers => get(get_v2),
            Endpoint::GetReferrersWithArtifactType => get(get_v2),
            Endpoint::GetBlobsUploads => get(get_v2),
        }
    }
}

pub fn register_routes(pool: SqlitePool, storage: Arc<Backend>) -> Router {
    Router::new()
        .route("/auth/login", post(login_user))
        .route("/auth/token", get(auth_token_get))
        .route("/auth/register", post(register_user))
        .route("/auth/clients", get(get_auth_clients))
        .route("/repositories", get(list_repositories))
        .route("/repositories/:name/:public", post(create_repository))
        .route("/repositories/:name", delete(delete_repository))
        .route("/users", get(get_users))
        .route("/users/:email", delete(delete_user))
        .route("/users/:email/tokens", post(generate_token))
        .route("/v2/", Endpoint::GetV2.to_handler())
        .route(
            "/v2/:name/blobs/:digest",
            Endpoint::PutBlobsNoSession.to_handler(),
        )
        .route("/v2/:name/blobs/:digest", Endpoint::GetBlobs.to_handler())
        .route("/v2/:name/blobs/:digest", Endpoint::HeadBlobs.to_handler())
        .route("/v2/:name/blobs/uploads/", post(handle_upload_blob))
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            Endpoint::PutBlobsUploadsWithDigest.to_handler(),
        )
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            Endpoint::PatchBlobsUploads.to_handler(),
        )
        .route(
            "/v2/:name/blobs/:digest",
            Endpoint::DeleteBlobs.to_handler(),
        )
        .route("/v2/:name/tags/list", Endpoint::GetTagsList.to_handler())
        .route(
            "/v2/:name/manifests/:reference",
            Endpoint::GetManifests.to_handler(),
        )
        .route(
            "/v2/:name/manifests/:reference",
            Endpoint::PutManifests.to_handler(),
        )
        .route(
            "/v2/:name/manifests/:reference",
            Endpoint::DeleteManifests.to_handler(),
        )
        .layer(from_fn(check_scope_middleware))
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            auth_middleware,
        ))
        .layer(Extension(storage))
        .layer(
            ServiceBuilder::new().layer(TraceLayer::new_for_http().make_span_with(
                |request: &Request<_>| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                },
            )),
        )
        .layer(Extension(axum::middleware::from_extractor::<Auth>()))
        .with_state(pool)
}
