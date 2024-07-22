use crate::{
    codes::{Code, ErrorResponse},
    database::DbConn,
};
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
#[derive(Serialize, Deserialize, Debug)]
pub struct TagsListResponse {
    name: String,
    tags: Vec<String>,
}

pub fn bad_request(msg: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, String::from(msg))
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

/// DELETE `/v2/<name>/manifests/<tag>`
pub async fn delete_tag(
    Path((name, tag)): Path<(String, String)>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    match sqlx::query!("DELETE FROM tags WHERE tag = ? AND repository_id = (SELECT id FROM repositories WHERE name = ?)",
        tag,
        name
    )
    .execute(&mut *conn)
    .await {
        Ok(_) => (StatusCode::ACCEPTED, "tag deleted successfully").into_response(),
        Err(_) => ErrorResponse::from_code(&Code::ManifestUnknown, "tag not found").into_response(),
    }
}
