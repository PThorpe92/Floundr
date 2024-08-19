use std::sync::Arc;

use crate::{
    auth::Auth,
    codes::{Code, ErrorResponse},
    database::DbConn,
    storage_driver::{Backend, DriverType},
};
use axum::{
    extract::{Path, Query, Request},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::debug;
#[derive(Serialize, Deserialize, Debug)]
pub struct TagsListResponse {
    name: String,
    tags: Vec<String>,
}

pub fn bad_request(msg: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, String::from(msg))
}

#[derive(Deserialize, Debug, Clone)]
pub struct DockerLogin {
    pub service: Option<String>,
    pub client_id: Option<String>,
    pub scope: Option<String>,
    pub offline_token: Option<bool>,
    pub account: Option<String>,
    pub password: Option<String>,
}

/// GET /v2/
/// Return status code 200
/// Spec: 770
pub async fn get_v2(headers: HeaderMap, Query(params): Query<DockerLogin>) -> StatusCode {
    debug!(
        "GET /v2/ Request headers: {:?}\n URI: {:?}",
        headers, params,
    );
    debug!("GET /v2/");
    StatusCode::OK
}

impl TagsListResponse {
    pub fn new(name: &str, tags: &[String]) -> Self {
        Self {
            name: String::from(name),
            tags: tags.to_vec(),
        }
    }
}
/// Response:
///    {
///    "name": "<name>",
///    "tags": ["<tag1>", "<tag2>", "<tag3>"]
///    }
#[derive(Deserialize, Debug, Clone)]
pub struct TagsQueryParams {
    n: Option<usize>,
    last: Option<String>,
}
/// Endpoint: Listing Referrers
///
/// GET /v2/:name/tags/list
/// query_params: n=<int> & last=<tagname>
/// sort_by: lexicographically
/// `/v2/<name>/tags/list?n=<int>&last=<tagname>`
///
/// spec: 526 - 574
pub async fn get_tags_list(
    DbConn(mut conn): DbConn,
    Path(name): Path<String>,
    Query(params): Query<TagsQueryParams>,
) -> impl IntoResponse {
    let TagsQueryParams { n, last } = params;
    let mut query_string = r#"
        SELECT tags.tag
        FROM repositories r
        JOIN tags ON tags.repository_id = r.id
        WHERE r.name = ?
    "#
    .to_string();
    if last.is_some() {
        query_string.push_str(" AND tags.tag > ?");
    }
    query_string.push_str(" ORDER BY tags.tag COLLATE NOCASE");
    if n.is_some() {
        query_string.push_str(" LIMIT ?");
    }
    let mut query = sqlx::query(&query_string);
    if let Some(last_tag) = last {
        query = query.bind(last_tag);
    }
    if let Some(limit) = n {
        query = query.bind(limit as i64);
    }
    query = query.bind(&name);
    match query.fetch_all(&mut *conn).await {
        Ok(rows) => {
            let tags: Vec<String> = rows.into_iter().map(|row| row.get(0)).collect();
            let mut headers = HeaderMap::new();
            if let Some(limit) = n {
                if tags.len() == limit {
                    let next_tag = tags.last().unwrap();
                    let link = format!(
                        "</v2/{}/tags/list?n={}&last={}>; rel=\"next\"",
                        name, limit, next_tag
                    );
                    headers.insert("Link", HeaderValue::from_str(&link).unwrap());
                }
            }

            let response = TagsListResponse::new(&name, &tags);
            (headers, Json(response)).into_response()
        }
        Err(_) => ErrorResponse::from_code(&Code::NameUnknown, String::from("namespace not found"))
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
pub struct Repository {
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

#[derive(Debug, Serialize)]
pub struct RepoList {
    pub repositories: Vec<Repository>,
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

pub async fn list_repositories(
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
    req: Request,
) -> impl IntoResponse {
    let auth = req.extensions().get::<Auth>();
    let mut query = String::from(
        r"SELECT id, name, is_public, (SELECT COUNT(*) from blobs where blobs.repository_id = repositories.id) as blob_count,
(SELECT COUNT(*) from tags WHERE tags.repository_id = repositories.id) as tag_count, (SELECT COUNT(m.id) from manifests m WHERE m.repository_id = id) as manifest_count,
(SELECT COUNT(*) from manifest_layers ml JOIN manifests m ON ml.manifest_id = m.id WHERE m.repository_id = ml.id) as num_layers FROM repositories",
    );
    if auth.is_some_and(|a| !a.is_valid()) {
        // list only public repos
        query.push_str(" WHERE is_public = true");
    };
    let repos = sqlx::query(&query).fetch_all(&mut *conn).await.unwrap();
    let mut names = Vec::new();
    for repo in repos {
        let id = repo.get::<i64, _>("id");
        let row = sqlx::query!("SELECT tag from tags t WHERE t.repository_id = ?", id)
            .fetch_all(&mut *conn)
            .await
            .unwrap();
        let tags = row.iter().map(|t| t.tag.clone()).collect::<Vec<String>>();
        let name = repo.get::<String, _>("name");
        let is_public = repo.get::<bool, _>("is_public");
        let blob_count = repo.get::<i64, _>("blob_count");
        let tag_count = repo.get::<i64, _>("tag_count");
        let manifest_count = repo.get::<i64, _>("manifest_count");
        let num_layers = repo.get::<i64, _>("num_layers");
        let disk_usage = storage
            .get_dir_size(storage.base_path().join(repo.get::<String, _>("name")))
            .await;
        names.push(Repository {
            name: name.clone(),
            is_public,
            blob_count,
            tag_count,
            tags,
            file_path: format!("{}/{}", storage.base_path().to_string_lossy(), &name),
            manifest_count,
            disk_usage,
            num_layers,
            driver: storage.kind(),
        });
    }
    let response = serde_json::to_string(&RepoList {
        repositories: names,
    })
    .unwrap();
    (StatusCode::OK, response).into_response()
}

pub async fn delete_repository(
    Path(name): Path<String>,
    DbConn(mut conn): DbConn,
    Extension(storage): Extension<Arc<Backend>>,
) -> impl IntoResponse {
    let _ = storage.delete_repository(&name, &mut conn).await;
    (StatusCode::OK, "repository deleted").into_response()
}
