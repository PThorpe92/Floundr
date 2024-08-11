use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDateTime;

use crate::{
    auth::Auth,
    codes::{Code, ErrorResponse},
    database::DbConn,
};

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct User {
    id: String,
    is_admin: bool,
    email: String,
    #[serde(skip_serializing)]
    password: String,
    #[serde(skip_serializing)]
    created_at: NaiveDateTime,
}
impl User {
    fn validate(&self) -> bool {
        self.email.is_ascii()
            && self.password.len() > 8
            && self.password.chars().any(char::is_numeric)
    }
}

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
    (StatusCode::OK, Json(users)).into_response()
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

pub async fn create_user(Json(user): Json<User>, DbConn(mut conn): DbConn) -> impl IntoResponse {
    if !user.validate() {
        return (StatusCode::BAD_REQUEST, "Invalid user").into_response();
    }
    let _ = sqlx::query!(
        "INSERT INTO users (id, email, password) VALUES (?, ?, ?)",
        user.id,
        user.email,
        user.password,
    )
    .execute(&mut *conn)
    .await
    .expect("unable to create user");
    (StatusCode::CREATED, "").into_response()
}

pub async fn generate_token(
    Path(email): Path<String>,
    DbConn(mut conn): DbConn,
    req: Request,
) -> impl IntoResponse {
    if !check_auth(&req) {
        return ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized").into_response();
    }
    let token = crate::database::generate_secret(&mut conn, Some(&email))
        .await
        .unwrap();
    (
        StatusCode::OK,
        serde_json::to_string(&crate::auth::TokenResponse::new(&token)).unwrap(),
    )
        .into_response()
}

#[derive(serde::Serialize, Debug)]
struct Client {
    #[serde(skip)]
    id: i64,
    client_id: String,
    user_id: String,
    secret: String,
    created_at: NaiveDateTime,
}
pub async fn list_keys(DbConn(mut conn): DbConn, req: Request) -> impl IntoResponse {
    if !check_auth(&req) {
        return ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized").into_response();
    }
    let keys = sqlx::query_as!(Client, "SELECT * FROM clients")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch keys");
    (StatusCode::OK, serde_json::to_string(&keys).unwrap()).into_response()
}
