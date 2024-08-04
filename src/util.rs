use crate::{auth::UserInfo, storage_driver::StorageError};
use base64::{alphabet::URL_SAFE, Engine};
use futures::{Stream, StreamExt};
use http::{header::CONTENT_RANGE, HeaderMap};
use sha2::{Digest, Sha256};
use std::{io, path::PathBuf};
use tracing::error;

pub static OCI_CONTENT_HEADER: &str = "application/vnd.oci.image.index.v1+json";
pub static DOCKER_DIGEST: &str = "Docker-Content-Digest";

pub fn calculate_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn validate_digest(data: &[u8], digest: &str) -> Result<(), StorageError> {
    let calculated_digest = calculate_digest(data);
    if calculated_digest != digest {
        return Err(StorageError::DigestError);
    }
    Ok(())
}

pub fn path_is_valid(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut components = path.components().peekable();

    if let Some(first) = components.peek() {
        if !matches!(first, std::path::Component::Normal(_)) {
            return false;
        }
    }
    components.count() == 1
}

pub fn strip_sha_header(digest: &str) -> String {
    if digest.starts_with("sha256:") {
        digest.split(':').nth(1).unwrap().to_string()
    } else {
        digest.to_string()
    }
}

pub fn base64_decode(data: &str) -> Result<String, String> {
    let decoded = base64::engine::GeneralPurpose::new(
        &URL_SAFE,
        base64::engine::GeneralPurposeConfig::default(),
    )
    .decode(data)
    .map_err(|_| String::from("Invalid base64"))?;
    String::from_utf8(decoded).map_err(|_| String::from("Invalid base64"))
}

pub fn parse_content_length(headers: &HeaderMap) -> i64 {
    headers
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
}

pub fn parse_content_range(range: &HeaderMap) -> (i64, i64) {
    if let Some(range) = range.get(CONTENT_RANGE) {
        let range = range.to_str().unwrap_or("0-0");
        let parts: Vec<&str> = range.split('-').collect();
        let begin = parts.first().unwrap_or(&"0").parse::<i64>().unwrap_or(0);
        let end = parts.get(1).unwrap_or(&"0").parse::<i64>().unwrap_or(0);
        (begin, end)
    } else {
        (0, 0)
    }
}

fn visit(path: impl Into<PathBuf>) -> impl Stream<Item = io::Result<u64>> + Send + 'static {
    async fn one_level(path: PathBuf, to_visit: &mut Vec<PathBuf>) -> io::Result<Vec<u64>> {
        let mut dir = tokio::fs::read_dir(&path).await?;
        let mut files = Vec::new();

        while let Some(child) = dir.next_entry().await? {
            let metadata = child.metadata().await?;
            if metadata.is_dir() {
                to_visit.push(child.path());
            } else {
                let size = metadata.len();
                files.push(size);
            }
        }

        Ok(files)
    }
    futures::stream::unfold(vec![path.into()], |mut to_visit| async {
        let path = to_visit.pop()?;
        let file_stream = match one_level(path, &mut to_visit).await {
            Ok(files) => futures::stream::iter(files).map(Ok).left_stream(),
            Err(e) => futures::stream::once(async { Err(e) }).right_stream(),
        };

        Some((file_stream, to_visit))
    })
    .flatten()
}

pub async fn get_dir_size(path: PathBuf) -> u64 {
    visit(path)
        .fold(0u64, |acc, entry| async move {
            acc + entry.unwrap_or_else(|e| {
                error!("error getting directory size: {e}");
                0
            })
        })
        .await
}

pub async fn verify_login(
    pool: &mut sqlx::SqliteConnection,
    email: &str,
    password: &str,
) -> Result<UserInfo, String> {
    let user = sqlx::query!("SELECT id, password FROM users WHERE email = ?", email)
        .fetch_one(pool)
        .await
        .map_err(|_| String::from("Invalid login"))?;
    if bcrypt::verify(password, &user.password).map_err(|_| String::from("Invalid login"))? {
        Ok(UserInfo {
            user_id: user.id,
            email: email.to_string(),
        })
    } else {
        Err(String::from("Invalid login").into())
    }
}

pub fn validate_registration(email: &str, psw: &str, confirm: &str) -> Result<(), String> {
    if !(psw.eq(confirm) && email.contains("@") && psw.len() >= 8 && email.contains(".")) {
        return Err("Invalid registration".to_string());
    }
    Ok(())
}
