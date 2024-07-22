use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tracing::debug;

use crate::storage;

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

pub async fn initdb(path: &str) -> sqlx::Pool<sqlx::Sqlite> {
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect(path)
        .await
        .expect("unable to connect to sqlite db pool")
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

pub async fn migrate_fresh(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    drop_tables(pool).await?;
    create_tables(pool).await?;
    Ok(())
}

pub async fn create_new_repo(
    name: &str,
    is_pub: bool,
    conn: &SqlitePool,
    storage: &storage::StorageDriver,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO repositories (name, is_public) VALUES (?, ?)",
        name,
        is_pub
    )
    .execute(conn)
    .await?;
    debug!("Created new repository: {}", name);
    let path = storage.base_path.join(name);
    debug!("creating path: {:?}", path);
    match std::fs::create_dir_all(&path) {
        Ok(_) => debug!("Created new repository directory: {:?}", name),
        Err(e) => debug!("Error creating new repository directory: {:?}", e),
    }
    Ok(())
}

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"
CREATE TABLE IF NOT EXISTS repositories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    is_public BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS blobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    digest TEXT NOT NULL UNIQUE,
    file_path TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);
CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    tag TEXT NOT NULL,
    blob_id INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    FOREIGN KEY (blob_id) REFERENCES blobs(id)
);
CREATE TABLE IF NOT EXISTS manifests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    digest TEXT NOT NULL UNIQUE,
    media_type TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);
CREATE TABLE IF NOT EXISTS uploads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    uuid TEXT NOT NULL UNIQUE,
    blob_id INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    FOREIGN KEY (blob_id) REFERENCES blobs(id)
);
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS repository_permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id),
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);
CREATE TABLE IF NOT EXISTS tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    token TEXT NOT NULL UNIQUE,
    expires TIMESTAMP NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id)
);
CREATE TRIGGER IF NOT EXISTS add_user_to_repo_permissions
AFTER INSERT ON repositories
  FOR EACH ROW WHEN NEW.is_public = 1
  BEGIN
      INSERT INTO repository_permissions (user_id, repository_id)
      SELECT id, NEW.id FROM users;
  END;
INSERT INTO repositories (id, name, is_public) VALUES (1, 'public', 1);
INSERT INTO users (id, email, password) VALUES (1, 'preston@unlockedlabs.org', 'ChangeMe!');",
    )
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
