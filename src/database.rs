use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use sqlx::SqlitePool;
use tracing::debug;

pub static TABLES: [&str; 5] = ["repositories", "blobs", "tags", "uploads", "manifests"];

pub struct DbConn(pub sqlx::pool::PoolConnection<sqlx::Sqlite>);
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

pub async fn migrate_fresh(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    drop_tables(pool).await?;
    create_tables(pool).await?;
    Ok(())
}

pub async fn create_new_repo(
    name: &str,
    is_pub: bool,
    conn: &SqlitePool,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO repositories (name, is_public) VALUES (?, ?)",
        name,
        is_pub
    )
    .execute(conn)
    .await?;
    debug!("Created new repository: {}", name);
    Ok(())
}

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(include_str!("../migrations/01_createtables.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn drop_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for table in TABLES {
        sqlx::query(&format!("DROP TABLE {};", table))
            .execute(pool)
            .await?;
    }
    Ok(())
}
