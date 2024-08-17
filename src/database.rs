use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use sqlx::{query, sqlite::SqlitePoolOptions, Acquire, Executor, SqliteConnection, SqlitePool};
use tracing::{error, info};

use crate::Repo;

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
    println!("connecting to sqlite db at: {}", path);
    if !std::path::PathBuf::from(path).exists() {
        tokio::fs::File::create(path)
            .await
            .expect("unable to create sqlite db");
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(path)
        .await
        .expect("unable to connect to sqlite db pool");
    let mut conn = pool.acquire().await.expect("unable to acquire connection");
    migrate(&mut conn, None, None)
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
        "INSERT INTO users (id, email, password, is_admin) VALUES (?, ?, ?, 1)",
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
    client_id: Option<String>,
    email: &str,
) -> Result<String, sqlx::Error> {
    let secret = uuid::Uuid::new_v4().to_string();
    let id = client_id.unwrap_or(uuid::Uuid::new_v4().to_string());
    query!(
        "INSERT INTO clients (client_id, secret, user_id) VALUES (?, ?, (SELECT id from users where email = ?))",
        id,
        secret,
        email
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

pub async fn get_repositories(conn: &mut SqliteConnection, pub_only: bool) -> Vec<Repo> {
    let repos = sqlx::query!("SELECT id, name, is_public FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch public repositories");
    if pub_only {
        return repos
            .iter()
            .filter_map(|row| {
                if row.is_public {
                    Some(row.name.parse().unwrap())
                } else {
                    None
                }
            })
            .collect();
    }
    repos
        .iter()
        .filter_map(|row| row.name.parse().ok())
        .collect()
}
impl DbConn {
    pub async fn delete_manifest(
        &mut self,
        name: &str,
        reference: &str,
    ) -> Result<String, sqlx::Error> {
        let mut tx = self.begin().await?;
        match sqlx::query!(
            "SELECT m.file_path, m.id, m.digest FROM manifests m
        JOIN repositories r ON m.repository_id = r.id
        LEFT JOIN tags t ON m.id = t.manifest_id
        WHERE (m.digest = $1 OR t.tag = $1) AND r.name = $2",
            reference,
            name
        )
        .fetch_one(&mut *tx)
        .await
        {
            Ok(found) => {
                info!("found manifest with ref: {}", reference);
                sqlx::query!(
                    "DELETE FROM manifest_layers WHERE manifest_id = ?",
                    found.id
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!("DELETE FROM tags WHERE manifest_id = ?", found.id)
                    .execute(&mut *tx)
                    .await?;
                if let Ok(layers) = sqlx::query!(
                    "SELECT digest FROM manifest_layers WHERE manifest_id = ?",
                    found.id
                )
                .fetch_all(&mut *tx)
                .await
                {
                    for layer in layers {
                        if let Err(err) = sqlx::query!(
                            "UPDATE blobs SET ref_count = ref_count - 1 WHERE digest = ?",
                            layer.digest
                        )
                        .execute(&mut *tx)
                        .await
                        {
                            error!("unable to update blob ref_count: {}", err);
                        }
                        if let Err(err) = sqlx::query!(
                            "DELETE FROM blobs WHERE digest = ? AND ref_count <= 0",
                            layer.digest
                        )
                        .execute(&mut *tx)
                        .await
                        {
                            error!("unable to delete blob: {}", err);
                        }
                    }
                }
                sqlx::query!("DELETE FROM manifests WHERE id = ?", found.id)
                    .execute(&mut *tx)
                    .await?;
                Ok(found.file_path)
            }
            Err(err) => {
                error!("unable to find manifest with reference: {}", reference);
                Err(err)
            }
        }
    }
}
