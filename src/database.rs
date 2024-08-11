use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use sqlx::{query, sqlite::SqlitePoolOptions, Acquire, Executor, SqliteConnection, SqlitePool};
use tracing::info;

pub static TABLES: [&str; 8] = [
    "repositories",
    "blobs",
    "tags",
    "uploads",
    "manifests",
    "repository_permissions",
    "users",
    "tokens",
];

pub struct DbConn(pub sqlx::pool::PoolConnection<sqlx::Sqlite>);

pub async fn initdb(
    path: &str,
    email: Option<String>,
    password: Option<String>,
) -> sqlx::Pool<sqlx::Sqlite> {
    info!("connecting to sqlite db at: {}", path);
    if !std::path::Path::new(&path).exists() {
        tokio::fs::File::create_new(&path)
            .await
            .expect("unable to create sqlite db");
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(path)
        .await
        .expect("unable to connect to sqlite db pool");
    let mut conn = pool.acquire().await.expect("unable to acquire connection");
    migrate(&mut conn, email, password)
        .await
        .expect("unable to migrate db");
    if query!("SELECT COUNT(*) as client_count from clients")
        .fetch_one(&mut *conn)
        .await
        .expect("unable to fetch client count")
        .client_count
        .eq(&0)
    {
        seed_default_client(&mut conn)
            .await
            .expect("unable to seed default client");
    }
    pool
}

pub async fn seed_default_user(
    pool: &mut SqliteConnection,
    email: Option<String>,
    psw: Option<String>,
) -> Result<(), sqlx::Error> {
    let uuid = uuid::Uuid::new_v4().to_string();
    let psw = bcrypt::hash(psw.unwrap_or("admin".to_string()), bcrypt::DEFAULT_COST)
        .expect("unable to hash default password");
    let email = email.unwrap_or("floundr_admin".to_string());
    let _ = query!(
        "INSERT INTO users (id, email, password) VALUES (?, ?, ?)",
        uuid,
        email,
        psw
    )
    .execute(&mut *pool)
    .await?;
    Ok(())
}

pub async fn seed_default_client(pool: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let secret = uuid::Uuid::new_v4().to_string();
    let id = "floundr_tui";
    query!(
        "INSERT INTO clients (client_id, secret, user_id) VALUES (?, ?, (SELECT id FROM users WHERE email = 'floundr_admin'))",
        id,
        secret
    )
    .execute(&mut *pool)
    .await?;
    Ok(())
}

pub async fn generate_secret(
    pool: &mut SqliteConnection,
    client_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let secret = uuid::Uuid::new_v4().to_string();
    let id = client_id.unwrap_or("floundr_tui");
    query!(
        "INSERT INTO clients (client_id, secret) VALUES (?, ?)",
        id,
        secret
    )
    .execute(&mut *pool)
    .await?;
    Ok(secret)
}

impl std::ops::Deref for DbConn {
    type Target = sqlx::pool::PoolConnection<sqlx::Sqlite>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DbConn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for DbConn
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = SqlitePool::from_ref(state);
        let conn = pool.acquire().await.map_err(internal_error)?;
        Ok(Self(conn))
    }
}

pub fn internal_error<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

pub fn not_found_error<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::NOT_FOUND, err.to_string())
}

pub async fn migrate_fresh(
    pool: &mut SqliteConnection,
    email: Option<String>,
    psw: Option<String>,
) -> Result<(), sqlx::Error> {
    drop_tables(pool).await?;
    migrate(pool, email, psw).await?;
    Ok(())
}

pub async fn migrate(
    pool: &mut SqliteConnection,
    email: Option<String>,
    psw: Option<String>,
) -> Result<(), sqlx::Error> {
    let conn = pool.acquire().await?;
    sqlx::query(&tokio::fs::read_to_string("migrations/01_createtables.sql").await?)
        .execute(&mut *conn)
        .await?;
    if sqlx::query!("SELECT COUNT(*) as user_count from users")
        .fetch_one(&mut *conn)
        .await?
        .user_count
        .eq(&0)
    {
        seed_default_user(&mut *conn, email, psw).await?;
    }
    Ok(())
}

pub async fn drop_tables(pool: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for table in TABLES {
        tx.execute(sqlx::query(&format!("DROP TABLE {};", table)))
            .await?;
    }
    Ok(())
}
