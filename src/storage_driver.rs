use axum::async_trait;
use axum::body::BodyDataStream;
use axum::extract::{FromRef, FromRequestParts};
use clap::ValueEnum;
use http::request::Parts;
use http::StatusCode;
use sqlx::SqliteConnection;
use std::path::PathBuf;

use crate::storage::LocalStorageDriver;

#[async_trait]
impl<S> FromRequestParts<S> for Backend
where
    Backend: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let storage = Backend::from_ref(state);
        Ok(storage)
    }
}

#[derive(Debug)]
pub enum StorageError {
    IoError(std::io::Error),
    SqlxError(sqlx::Error),
    DigestError,
    InvalidLogin,
    OutOfOrder,
}
impl std::error::Error for StorageError {}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO Error: {}", e),
            Self::SqlxError(e) => write!(f, "SQLx Error: {}", e),
            Self::DigestError => write!(f, "Digest mismatch"),
            Self::InvalidLogin => write!(f, "Login failed"),
            Self::OutOfOrder => write!(f, "Chunk out of order"),
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}
impl From<sqlx::Error> for StorageError {
    fn from(e: sqlx::Error) -> Self {
        Self::SqlxError(e)
    }
}
#[derive(Debug, serde::Serialize, Clone)]
pub enum DriverType {
    Local,
    S3,
}
impl ValueEnum for DriverType {
    fn from_str(input: &str, ignore_case: bool) -> Result<Self, String> {
        if ignore_case {
            match input.to_lowercase().as_str() {
                "local" => Ok(Self::Local),
                "s3" => Ok(Self::S3),
                _ => Err(format!("invalid storage driver: {}", input)),
            }
        } else {
            match input {
                "local" => Ok(Self::Local),
                "s3" => Ok(Self::S3),
                _ => Err(format!("invalid storage driver: {}", input)),
            }
        }
    }
    fn value_variants<'a>() -> &'a [Self] {
        [Self::Local, Self::S3].as_ref()
    }
    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::S3 => Some("s3".into()),
            Self::Local => Some("local".into()),
        }
    }
}

pub enum Backend {
    Local(LocalStorageDriver),
    // S3(S3StorageDriver),
}
// all this just because we can't use trait objects async
macro_rules! backend_methods {
    ($enum_name:ident, $($variant:ident),+) => {
        impl $enum_name {
            pub fn kind(&self) -> DriverType {
                match self {
                    $(Self::$variant(_) => DriverType::$variant,)+
                }
            }
            pub fn base_path(&self) -> &PathBuf {
                match self {
                    $(Self::$variant(driver) => driver.base_path(),)+
                }
            }

            pub async fn write_blob(
                &self,
                name: &str,
                session_id: &str,
                chunk: i64,
                pool: &mut SqliteConnection,
                data: BodyDataStream,
            ) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.write_blob(name, session_id, chunk, pool, data).await,)+
                }
            }

            pub async fn write_blob_without_session_id(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                digest: &str,
                data: BodyDataStream,
            ) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.write_blob_without_session_id(pool, name, digest, data).await,)+
                }
            }

            pub async fn read_blob(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                digest: &str,
            ) -> Result<Vec<u8>, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.read_blob(pool, name, digest).await,)+
                }
            }

            pub async fn read_manifest(
                &self,
                path: &str,
            ) -> Result<Vec<u8>, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.read_manifest(path).await,)+
                }
            }

            pub async fn new_session(
                &self,
                conn: &mut SqliteConnection,
                name: &str,
            ) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.new_session(conn, name).await,)+
                }
            }

            pub async fn mount_blob(
                &self,
                pool: &mut SqliteConnection,
                target_name: &str,
                digest: &str,
                source_name: Option<&str>,
            ) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.mount_blob(pool, target_name, digest, source_name).await,)+
                }
            }

            pub async fn write_manifest(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                reference: &str,
                data: BodyDataStream,
            ) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.write_manifest(pool, name, reference, data).await,)+
                }
            }

            pub async fn delete_blob(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                digest: &str,
            ) -> Result<(), StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.delete_blob(pool, name, digest).await,)+
                }
            }

            pub async fn delete_manifest(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                reference: &str,
            ) -> Result<(), StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.delete_manifest(pool, name, reference).await,)+
                }
            }

            pub async fn create_repository(
                &self,
                pool: &mut SqliteConnection,
                name: &str,
                is_public: bool,
            ) -> Result<(), StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.create_repository(pool, name, is_public).await,)+
                }
            }
            pub async fn combine_chunks(
            &self, pool: &mut SqliteConnection, name: &str, session_id: &str) -> Result<String, StorageError> {
                match self {
                    $(Self::$variant(driver) => driver.combine_chunks(pool, name, session_id).await,)+
            }
           }
        }
    };
}

backend_methods!(Backend, Local /*, S3 */);
impl Backend {
    pub fn new(driver: DriverType, base_path: &std::path::Path) -> Self {
        match driver {
            DriverType::Local => Self::Local(LocalStorageDriver::new(base_path)),
            DriverType::S3 => todo!(),
        }
    }
}
