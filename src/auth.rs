use std::fmt::Formatter;

use axum::{
    extract::{Query, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{alphabet::URL_SAFE, Engine};
use http::{
    header::{SET_COOKIE, WWW_AUTHENTICATE},
    HeaderMap,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::{query, SqliteConnection};
use tracing::{debug, info};
use uuid::Uuid;

use crate::{
    codes::{Code, ErrorResponse},
    content_discovery::DockerLogin,
    database::DbConn,
    util::{base64_decode, validate_registration, verify_login},
};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Auth {
    pub claims: Option<Claims>,
}
impl Auth {
    pub fn is_valid(&self) -> bool {
        self.claims.as_ref().is_some_and(|c| c.is_valid())
    }
    pub fn get_user_info(&self) -> Option<UserInfo> {
        self.claims.as_ref().and_then(|c| c.get_user_info())
    }
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Claims {
    sub: String,
    exp: usize,
}

impl Claims {
    pub fn is_valid(&self) -> bool {
        self.exp > chrono::offset::Local::now().timestamp_millis() as usize
    }
    pub fn set_sub(&mut self, sub: String) {
        self.sub = sub;
    }
}

impl Default for Claims {
    fn default() -> Self {
        Self {
            sub: "".to_string(),
            exp: chrono::offset::Utc::now()
                .checked_add_days(chrono::Days::new(1))
                .unwrap()
                .timestamp_millis() as usize,
        }
    }
}
#[derive(Serialize, Clone, Default, Deserialize, Debug)]
pub struct UserInfo {
    pub email: String,
    pub user_id: String,
}

pub async fn auth_middleware(
    DbConn(mut conn): DbConn,
    mut req: Request,
    next: Next,
) -> Result<Response, Response> {
    let headers = req.headers().clone();
    let span = tracing::info_span!("auth_middleware");
    span.record("headers", format!("{:?}", headers));
    span.record("uri", req.uri().to_string());
    let mut resp_headers = HeaderMap::new();
    let app_url = std::env::var("APP_URL").unwrap_or("http://127.0.0.1:8080".to_string());
    resp_headers.insert(
        WWW_AUTHENTICATE,
        format!(
            "Bearer realm=\"{}/v2/auth/token\",service=\"harbor\",scope=\"repository:*:*\"",
            app_url
        )
        .parse()
        .unwrap(),
    );
    if let Some(auth_header) = check_headers(&headers) {
        match auth_header {
            header if header.to_lowercase().contains("bearer") => {
                let token = header
                    .split("Bearer ")
                    .last()
                    .unwrap_or(header.split("bearer ").last().unwrap());
                if let Ok(claims) = validate_bearer(token, &mut conn).await {
                    req.extensions_mut().insert(Auth {
                        claims: Some(claims),
                    });
                    return Ok(next.run(req).await);
                }
            }
            header if header.contains("Basic") => {
                let token = header
                    .split("Basic ")
                    .last()
                    .unwrap_or(header.split("bearer ").last().unwrap());
                if let Ok(claims) = validate_basic_auth(token, &mut conn).await {
                    req.extensions_mut().insert(Auth {
                        claims: Some(claims),
                    });
                    return Ok(next.run(req).await);
                }
            }
            _ => {
                return Err((StatusCode::UNAUTHORIZED, resp_headers).into_response());
            }
        }
    } else {
        if is_public_route(req.uri().path()) {
            req.extensions_mut().insert(Auth::default());
            return Ok(next.run(req).await);
        };
        let repo_name = req.uri().path().split("/").nth(2).unwrap_or_default();
        if sqlx::query!(
            "SELECT is_public from repositories WHERE name = ?",
            repo_name
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, resp_headers.clone()).into_response())?
        .is_public
        {
            req.extensions_mut().insert(Auth::default());
            return Ok(next.run(req).await);
        } else {
            return Err((StatusCode::UNAUTHORIZED, resp_headers).into_response());
        }
    }
    Ok(next.run(req).await)
}

fn check_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

async fn validate_basic_auth(token: &str, conn: &mut SqliteConnection) -> Result<Claims, String> {
    let decoded = base64::engine::GeneralPurpose::new(
        &URL_SAFE,
        base64::engine::GeneralPurposeConfig::default(),
    )
    .decode(token)
    .unwrap();
    let decoded = String::from_utf8(decoded).unwrap();
    let parts: Vec<&str> = decoded.split(":").collect();
    let user = parts[0];
    let password = parts.get(1).unwrap_or(&"");
    let user_info = verify_login(conn, user, password)
        .await
        .map_err(|e| e.to_string())?;
    let mut claims = Claims::default();
    claims.set_sub(serde_json::to_string(&user_info).unwrap());
    Ok(claims)
}

async fn validate_bearer(token: &str, conn: &mut SqliteConnection) -> Result<Claims, String> {
    if let Ok(client_id) = query!("SELECT client_id from clients WHERE secret = ?", token)
        .fetch_one(&mut *conn)
        .await
    {
        let mut claims = Claims::default();
        claims.set_sub(client_id.client_id);
        return Ok(claims);
    }
    let secret = std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY env var needs to be set");
    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims);
    claims.map_err(|s| s.to_string())
}

fn is_public_route(path: &str) -> bool {
    let routes = ["/repositories", "/v2/auth/token", "/v2/auth/login"];
    routes.iter().any(|r| path.eq(*r))
}

#[derive(Serialize, Debug)]
pub struct TokenResponse {
    token: String,
}
impl TokenResponse {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
        }
    }
}
pub async fn oauth_token_get(
    DbConn(mut conn): DbConn,
    Query(params): Query<DockerLogin>,
    headers: HeaderMap,
    req: Request,
) -> impl IntoResponse {
    if let Some(auth) = req.extensions().get::<Auth>() {
        if let Some(ref claims) = auth.claims {
            return (
                StatusCode::OK,
                serde_json::to_string(&TokenResponse {
                    token: claims.to_string(),
                })
                .unwrap(),
            )
                .into_response();
        }
    }
    if let Some(auth_header) = headers.get("authorization") {
        let token = auth_header
            .to_str()
            .unwrap()
            .split("Basic ")
            .last()
            .unwrap();
        let decoded = base64_decode(token).unwrap_or_else(|e| e);
        let parts: Vec<&str> = decoded.split(":").collect();
        let user = parts[0];
        let password = parts.get(1).unwrap_or(&"");
        match verify_login(&mut conn, user, password).await {
            Ok(user_id) => {
                let mut claims = Claims::default();
                claims.set_sub(serde_json::to_string(&user_id).unwrap());
                let token = claims.to_string();
                return (
                    StatusCode::OK,
                    serde_json::to_string(&TokenResponse { token }).unwrap(),
                )
                    .into_response();
            }
            Err(_) => return (StatusCode::UNAUTHORIZED).into_response(),
        };
    }
    let token = Uuid::new_v4().to_string();
    let user = params.account.unwrap();
    let client = params.client_id.unwrap();
    if let Err(e) = sqlx::query!(
        "INSERT INTO tokens (token, account, client_id) VALUES (?, ?, ?)",
        token,
        user,
        client,
    )
    .execute(&mut *conn)
    .await
    {
        tracing::error!("failed to insert token: {}", e);
    }
    let response = TokenResponse { token };
    let body = serde_json::to_string(&response).unwrap();
    debug!("oauth_token_get: {:?}", body);
    (StatusCode::OK, body).into_response()
}

#[derive(Deserialize, Debug)]
pub struct LoginRequest {
    email: String,
    password: String,
}
pub async fn login_user(
    DbConn(mut conn): DbConn,
    Query(params): Query<DockerLogin>,
    Json(req): Json<Option<LoginRequest>>,
) -> impl IntoResponse {
    info!("login user: {:?}", params);
    info!("login user: {:?}", req);
    if req.is_none() {
        let user = params.account.unwrap();
        let password = params.password.unwrap();
        match verify_login(&mut conn, &user, &password).await {
            Ok(user_id) => {
                let mut claims = Claims::default();
                claims.set_sub(serde_json::to_string(&user_id).unwrap());
                let token_resp = serde_json::to_string(&TokenResponse {
                    token: claims.to_string(),
                })
                .unwrap();
                (StatusCode::OK, token_resp).into_response()
            }
            Err(_) => {
                tracing::error!("failed to verify password {} : {}", &user, &password);
                (
                    StatusCode::UNAUTHORIZED,
                    ErrorResponse::from_code(&Code::NameUnknown, String::from("invalid login")),
                )
                    .into_response()
            }
        };
    }
    let req = req.unwrap();
    info!("login user: {:?}", req);
    match verify_login(&mut conn, &req.email, &req.password).await {
        Ok(user_id) => {
            let mut claims = Claims::default();
            claims.set_sub(serde_json::to_string(&user_id).unwrap());
            let token_resp = serde_json::to_string(&TokenResponse {
                token: claims.to_string(),
            })
            .unwrap();
            (StatusCode::OK, token_resp).into_response()
        }
        Err(_) => {
            tracing::error!(
                "failed to verify password {} : {}",
                &req.email,
                &req.password
            );
            (
                StatusCode::UNAUTHORIZED,
                ErrorResponse::from_code(&Code::NameUnknown, String::from("invalid login")),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct RegisterUserRequest {
    email: String,
    password: String,
    confirm_password: String,
}

pub async fn register_user(
    DbConn(mut conn): DbConn,
    Json(req): Json<RegisterUserRequest>,
) -> impl IntoResponse {
    match validate_registration(&req.email, &req.password, &req.confirm_password) {
        Ok(_) => {
            let hashed = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST).unwrap();
            let user_id = uuid::Uuid::new_v4().to_string();
            let _ = sqlx::query!(
                r#"
            INSERT INTO users (id, email, password)
            VALUES (?, ?, ?)
        "#,
                user_id,
                req.email,
                hashed
            )
            .execute(&mut *conn)
            .await;
            let claims = Claims::new(&req.email, &user_id).to_string();
            let mut header_map = HeaderMap::new();
            header_map.insert(
                SET_COOKIE,
                format!("Authorization: bearer {}", claims).parse().unwrap(),
            );
            (StatusCode::OK, header_map).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

pub async fn change_password(
    DbConn(mut conn): DbConn,
    Json(req): Json<RegisterUserRequest>,
) -> impl IntoResponse {
    match validate_registration(&req.email, &req.password, &req.confirm_password) {
        Ok(_) => {
            let hashed = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST).unwrap();
            let _ = sqlx::query!(
                "UPDATE users SET password = ? WHERE email = ?",
                req.email,
                hashed
            )
            .execute(&mut *conn)
            .await;
            (StatusCode::OK, "").into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

pub async fn check_auth(
    claims: Auth,
    name: &str,
    conn: &mut SqliteConnection,
) -> Result<(), StatusCode> {
    if query!("SELECT is_public from repositories WHERE name = ?", name)
        .fetch_one(&mut *conn)
        .await
        .is_ok_and(|row| row.is_public)
    {
        return Ok(());
    } else if !claims.claims.is_some_and(|c| c.is_valid()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[warn(clippy::recursive_format_impl)]
impl std::fmt::Display for Claims {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let secret =
            std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY env var needs to be set");
        let token = encode(
            &Header::default(),
            self,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("failed to encode jwt");
        write!(f, "{}", token)
    }
}

impl Claims {
    pub fn new(email: &str, user_id: &str) -> Self {
        let expiration = chrono::offset::Local::now()
            .checked_add_days(chrono::Days::new(1))
            .unwrap();
        let user_info = serde_json::to_string(&UserInfo {
            email: email.to_owned(),
            user_id: user_id.to_owned(),
        })
        .unwrap_or_default();

        Claims {
            sub: user_info,
            exp: expiration.timestamp_millis() as usize,
        }
    }

    pub fn update_jwt(&self) -> String {
        let secret =
            std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY env var needs to be set");
        let expiration = chrono::offset::Local::now()
            .checked_add_days(chrono::Days::new(1))
            .expect("date failed to add 1 day");
        let claims = Claims {
            sub: self.sub.to_owned(),
            exp: expiration.timestamp_millis() as usize,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("failed to encode jwt");
        token
    }

    pub fn get_user_info(&self) -> Option<UserInfo> {
        let secret =
            std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY env var needs to be set");
        let claims = decode::<Claims>(
            &self.sub,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .map(|data| data.claims)
        .ok()?;
        serde_json::from_str(&claims.sub).ok()
    }

    pub fn validate_jwt(token: &str) -> Result<Self, String> {
        if let Ok(claims) = decode::<Claims>(
            token,
            &DecodingKey::from_secret(
                std::env::var("JWT_SECRET_KEY")
                    .expect("failed to get secret key")
                    .as_bytes(),
            ),
            &Validation::default(),
        )
        .map(|data| data.claims)
        {
            if claims.is_valid() {
                return Ok(claims);
            }
        }
        Err("Token expired".to_string())
    }
}
