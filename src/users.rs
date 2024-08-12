use crate::{
    auth::Auth,
    codes::{Code, ErrorResponse},
    database::DbConn,
};
use axum::{
    debug_handler,
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDateTime;
use shared::User;
use shared::{RepoScope, UserResponse};
fn check_auth(req: &Request) -> bool {
    // here we ensure that we are dealing with the tui client
    let headers = req.headers().get("User-Agent");
    if headers.is_some_and(|h| h.to_str().unwrap().contains("floundr-tui")) {
        let auth = req.extensions().get::<Auth>();
        return auth.is_some() && auth.is_some_and(|a| a.is_valid());
    };
    false
}

pub async fn get_users(DbConn(mut conn): DbConn, req: Request) -> impl IntoResponse {
    if !check_auth(&req) {
        return ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized").into_response();
    }
    let users = sqlx::query_as!(User, "SELECT * FROM users")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch users");
    let repositories = sqlx::query!("SELECT * FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch repositories");
    let mut user_resp = vec![];
    for user in users.iter() {
        let mut user_scopes = vec![];
        for repo in repositories.iter() {
            let scopes = match user.is_admin {
                true => vec!["pull".to_string(), "push".to_string(), "delete".to_string()],
                false => {
                    let scopes = sqlx::query!("SELECT push, pull, del FROM repository_scopes WHERE user_id = ? AND repository_id = ?", user.id, repo.id)
                        .fetch_one(&mut *conn)
                        .await
                        .expect("unable to fetch permissions");
                    let mut permissions = vec![];
                    if scopes.push {
                        permissions.push("push".to_string());
                    }
                    if scopes.pull {
                        permissions.push("pull".to_string());
                    }
                    if scopes.del {
                        permissions.push("delete".to_string());
                    }
                    permissions
                }
            };
            user_scopes.push(RepoScope {
                repo: repo.name.clone(),
                scope: scopes,
            });
        }
        user_resp.push(UserResponse {
            user: user.clone(),
            scopes: user_scopes,
        });
    }
    (StatusCode::OK, Json(user_resp)).into_response()
}

pub async fn delete_user(
    Path(email): Path<String>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    if !check_auth(&req) {
        return ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized").into_response();
    }
    let _ = sqlx::query!("DELETE FROM users WHERE email = ?", email)
        .execute(&mut *conn)
        .await
        .expect("unable to delete user");
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn generate_token(
    Path(email): Path<String>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    if !check_auth(&req) {
        return ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized").into_response();
    }
    let token = crate::database::generate_secret(&mut conn, None, &email)
        .await
        .unwrap();
    (
        StatusCode::OK,
        serde_json::to_string(&crate::auth::TokenResponse::new(&token)).unwrap(),
    )
        .into_response()
}
