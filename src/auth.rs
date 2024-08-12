use std::fmt::Formatter;

use super::UserScope;
use crate::{
    codes::{Code, ErrorResponse},
    content_discovery::DockerLogin,
    database::DbConn,
    get_admin_scopes, get_user_scopes,
    util::{base64_decode, validate_registration, verify_login},
    Action,
};
use axum::{
    extract::{Query, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use http::{
    header::{SET_COOKIE, WWW_AUTHENTICATE},
    HeaderMap,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use shared::{AuthClient, RegisterUserRequest};
use sqlx::{query, SqliteConnection};
use tracing::info;

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

#[derive(Serialize, Debug, Deserialize, Clone)]
pub struct Claims {
    sub: String,
    exp: usize,
    is_admin: bool,
    #[serde(
        serialize_with = "crate::util::scopes_to_vec",
        deserialize_with = "crate::util::vec_to_scopes"
    )]
    scopes: UserScope,
}

impl Claims {
    pub fn is_valid(&self) -> bool {
        self.exp > chrono::offset::Local::now().timestamp_millis() as usize
    }
    pub fn set(&mut self, info: &UserInfo) {
        self.sub = info.id.to_string();
        self.is_admin = info.is_admin;
    }
    pub fn set_sub(&mut self, sub: String) {
        self.sub = sub
    }
    pub fn set_scope(&mut self, scope: UserScope) {
        self.scopes = scope
    }
    pub fn set_admin(&mut self, is_admin: bool) {
        self.is_admin = is_admin
    }
    pub fn is_admin(&self) -> bool {
        self.is_admin
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
            is_admin: false,
            scopes: UserScope::default(),
        }
    }
}

#[derive(Serialize, Clone, Default, Deserialize, Debug)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub is_admin: bool,
}

#[tracing::instrument(skip(conn))]
pub async fn auth_middleware(
    DbConn(mut conn): DbConn,
    mut req: Request,
    next: Next,
) -> Result<Response, Response> {
    let headers = req.headers().clone();
    let mut resp_headers = HeaderMap::new();
    let requested_scope = get_requested_scope(&req);
    let app_url =
        std::env::var("APP_URL").map_err(|_| (StatusCode::UNAUTHORIZED).into_response())?;
    resp_headers.insert(
        WWW_AUTHENTICATE,
        format!(
            "Bearer realm=\"{}/v2/auth/token\",service=\"floundr\",scope=\"{}\"",
            app_url, requested_scope
        )
        .parse()
        .unwrap(),
    );
    match check_headers(&headers, &mut conn).await {
        Ok(auth) => {
            req.extensions_mut().insert(auth);
            return Ok(next.run(req).await);
        }
        Err(err) => {
            tracing::error!("failed to validate auth header: {}", err);
            if is_public_route(req.uri().path()) {
                req.extensions_mut().insert(Auth::default());
                return Ok(next.run(req).await);
            };
            if is_pub_repo(req.uri().path(), &mut conn).await {
                req.extensions_mut().insert(Auth::default());
                return Ok(next.run(req).await);
            }
            return Err((StatusCode::UNAUTHORIZED, resp_headers).into_response());
        }
    }
}

async fn is_pub_repo(path: &str, conn: &mut SqliteConnection) -> bool {
    match path.split("/").nth(2) {
        Some(repo) => sqlx::query!("SELECT is_public from repositories WHERE name = ?", repo)
            .fetch_one(&mut *conn)
            .await
            .map(|r| r.is_public)
            .unwrap_or(false),
        None => false,
    }
}

fn get_requested_scope(req: &Request) -> String {
    let query = req.uri().query().unwrap_or_default();
    let scopes = query
        .split('&')
        .filter_map(|q| {
            if let Some(scope_param) = q.strip_prefix("scope=") {
                let parts: Vec<&str> = scope_param.split(':').collect();
                if parts.len() < 3 {
                    return None;
                }
                let repo = parts[1].to_string();
                let actions_str = parts[2];
                let actions: Vec<String> = actions_str
                    .split(',')
                    .map(|action| action.to_string())
                    .collect();
                Some((repo, actions))
            } else {
                None
            }
        })
        .collect::<Vec<(String, Vec<String>)>>();
    let mut requested = String::new();
    for (repo, actions) in scopes {
        for action in actions {
            requested.push_str(&format!("repository:{}:{} ", repo, action));
        }
    }
    if requested.is_empty() {
        requested.push_str("repository:*:*");
    } else {
        requested = requested.trim_end().to_string();
    }
    requested
}

async fn check_headers(headers: &HeaderMap, conn: &mut SqliteConnection) -> Result<Auth, String> {
    let auth_header = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .ok_or_else(|| String::from("Missing authorization header"))?;

    if auth_header.to_lowercase().starts_with("bearer ") {
        let token = auth_header
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| String::from("Invalid bearer token format"))?;
        return validate_bearer(token, conn).await;
    }

    if auth_header.to_lowercase().starts_with("basic ") {
        let token = auth_header
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| String::from("Invalid basic auth format"))?;
        if let Ok(claims) = validate_basic_auth(token, conn).await {
            return Ok(Auth {
                claims: Some(claims),
            });
        }
    }
    Err(String::from("invalid auth header"))
}

#[tracing::instrument]
pub async fn check_scope_middleware(req: Request, next: Next) -> Result<Response, Response> {
    let auth = req
        .extensions()
        .get::<Auth>()
        .ok_or((StatusCode::UNAUTHORIZED, auth_response_headers()).into_response())?;
    if let Some(claims) = &auth.claims {
        if claims.is_admin() {
            info!("user is administrator: {}", claims.sub);
            return Ok(next.run(req).await);
        } else {
            match Action::from_request(&req) {
                Some(required_scope) => {
                    let repo_name = req.uri().path().split('/').nth(2).unwrap_or_default();
                    if let Some(scopes) = claims.scopes.0.iter().find(|(r, _)| r.eq(&repo_name)) {
                        if scopes.1.contains(&required_scope) {
                            info!("user has required scope: {}", required_scope);
                            return Ok(next.run(req).await);
                        }
                    }
                }
                None => {
                    info!("no scope required for this request");
                    return Ok(next.run(req).await);
                }
            }
        }
    }
    return Err((
        StatusCode::UNAUTHORIZED,
        auth_response_headers(),
        "requested unauthorized scope",
    )
        .into_response());
}

fn auth_response_headers() -> HeaderMap {
    let mut resp_headers = HeaderMap::new();
    let app_url = std::env::var("APP_URL").expect("APP_URL env var needs to be set");
    resp_headers.insert(
        WWW_AUTHENTICATE,
        format!(
            "Bearer realm=\"{}/v2/auth/token\",service=\"floundr\",scope=\"repository:*:*\"",
            app_url
        )
        .parse()
        .unwrap(),
    );
    resp_headers
}

async fn validate_basic_auth(token: &str, conn: &mut SqliteConnection) -> Result<Claims, String> {
    let decoded = base64_decode(token)?;
    let parts: Vec<&str> = decoded.split(":").collect();
    let user = parts[0];
    let password = parts.get(1).unwrap_or(&"");
    let user_info = verify_login(conn, user, password)
        .await
        .map_err(|e| e.to_string())?;
    let mut claims = Claims::default();
    if user_info.is_admin {
        let scopes = get_admin_scopes(conn).await;
        claims.set_admin(true);
        claims.set_scope(scopes);
    } else {
        let scopes = get_user_scopes(conn, &user_info.id).await;
        claims.set(&user_info);
        claims.set_scope(scopes);
    }
    tracing::info!("user scopes attached: {:?}", claims.scopes);
    Ok(claims)
}

#[tracing::instrument(skip(conn))]
async fn validate_bearer(token: &str, conn: &mut SqliteConnection) -> Result<Auth, String> {
    // check if it's an assigned API key (uuid v4)
    // these carry all scopes for each repository
    info!("validating bearer token: {}", token);
    if let Ok(row) = query!("SELECT client_id FROM clients WHERE secret = ?", token)
        .fetch_one(&mut *conn)
        .await
    {
        let scopes = get_admin_scopes(conn).await;
        let mut claims = Claims::default();
        claims.set_sub(row.client_id);
        claims.set_admin(true);
        claims.scopes = scopes;
        return Ok(Auth {
            claims: Some(claims),
        });
    }
    let secret = std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY env var needs to be set");
    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| e.to_string())?
    .claims;
    Ok(Auth {
        claims: Some(claims),
    })
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

#[tracing::instrument(skip(conn))]
pub async fn auth_token_get(
    DbConn(mut conn): DbConn,
    Query(params): Query<DockerLogin>,
    headers: HeaderMap,
    req: Request,
) -> impl IntoResponse {
    let scope = params.scope.unwrap_or_default();
    if let Ok(auth) = check_headers(&headers, &mut conn).await {
        if let Some(ref claims) = auth.claims {
            if claims.is_valid() && claims.scopes.is_allowed(&scope) {
                return (
                    StatusCode::OK,
                    serde_json::to_string(&TokenResponse {
                        token: auth.claims.unwrap().update_jwt(),
                    })
                    .unwrap(),
                )
                    .into_response();
            }
        }
    }
    (
        StatusCode::UNAUTHORIZED,
        ErrorResponse::from_code(&Code::Unauthorized, "Unauthorized"),
    )
        .into_response()
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
            Ok(info) => {
                let mut claims = Claims::default();
                claims.set(&info);
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
        Ok(info) => {
            let mut claims = Claims::default();
            claims.set(&info);
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

pub async fn get_auth_clients(DbConn(mut conn): DbConn) -> impl IntoResponse {
    if let Ok(clients) = sqlx::query_as!(
        AuthClient,
        "SELECT clients.id, client_id, secret, clients.created_at, u.email FROM clients JOIN users u ON user_id = u.id"
    )
    .fetch_all(&mut *conn)
    .await
    {
        return (StatusCode::OK, serde_json::to_string(&clients).unwrap()).into_response();
    }
    (StatusCode::NOT_FOUND, "no auth clients were found").into_response()
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
                "INSERT INTO users (id, email, password, is_admin) VALUES (?, ?, ?, ?)",
                user_id,
                req.email,
                hashed,
                req.is_admin,
            )
            .execute(&mut *conn)
            .await;
            (StatusCode::CREATED).into_response()
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
    pub fn new(user_id: &str) -> Self {
        let expiration = chrono::offset::Local::now()
            .checked_add_days(chrono::Days::new(1))
            .unwrap();
        Claims {
            sub: user_id.to_string(),
            exp: expiration.timestamp_millis() as usize,
            scopes: UserScope::default(),
            is_admin: false,
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
            is_admin: self.is_admin,
            scopes: self.scopes.clone(),
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
        Some(UserInfo {
            id: claims.sub,
            email: "".to_string(),
            is_admin: claims.is_admin,
        })
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
