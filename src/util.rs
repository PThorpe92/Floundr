use crate::{auth::UserInfo, storage_driver::StorageError};
use crate::{Action, UserScope};
use base64::{alphabet::URL_SAFE, Engine};
use http::{header::CONTENT_RANGE, HeaderMap};
use sha2::{Digest, Sha256};

pub static OCI_CONTENT_HEADER: &str = "application/vnd.oci.image.index.v1+json";
pub static DOCKER_DIGEST: &str = "Docker-Content-Digest";
pub static MANIFEST_CONTENT_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";

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

pub async fn verify_login(
    pool: &mut sqlx::SqliteConnection,
    email: &str,
    password: &str,
) -> Result<UserInfo, String> {
    let user = sqlx::query!(
        "SELECT id, password, is_admin FROM users WHERE email = ?",
        email
    )
    .fetch_one(pool)
    .await
    .map_err(|_| String::from("Invalid login"))?;
    if bcrypt::verify(password, &user.password).map_err(|_| String::from("Invalid login"))? {
        Ok(UserInfo {
            id: user.id,
            email: email.to_string(),
            is_admin: user.is_admin,
        })
    } else {
        Err(String::from("Invalid login"))
    }
}

pub fn validate_registration(email: &str, psw: &str, confirm: &str) -> Result<(), String> {
    if !(psw.eq(confirm) && email.contains("@") && psw.len() >= 8 && email.contains(".")) {
        return Err("Invalid registration".to_string());
    }
    Ok(())
}

use serde::de::{self, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserializer, Serializer};
use std::fmt;

pub fn scopes_to_vec<S>(scopes: &UserScope, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(scopes.0.len()))?;
    for (repo, actions) in scopes.0.iter() {
        for action in actions {
            seq.serialize_element(&format!("repository:{}:{}", repo, action))?;
        }
    }
    seq.end()
}

pub fn vec_to_scopes<'de, D>(deserializer: D) -> Result<UserScope, D::Error>
where
    D: Deserializer<'de>,
{
    struct ScopesVisitor;

    impl<'de> Visitor<'de> for ScopesVisitor {
        type Value = UserScope;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of scope strings")
        }

        fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
        where
            V: de::SeqAccess<'de>,
        {
            let mut map = UserScope::default();

            while let Some(scope_str) = seq.next_element::<String>()? {
                let parts: Vec<&str> = scope_str.split(':').collect();
                if parts.len() == 3 {
                    map.0
                        .entry(parts[1].to_string())
                        .or_insert_with(Vec::new)
                        .push(match parts[2] {
                            "pull" => Action::Pull,
                            "push" => Action::Push,
                            "delete" => Action::Delete,
                            _ => return Err(de::Error::custom("Invalid scope format")),
                        });
                } else {
                    return Err(de::Error::custom("Invalid scope format"));
                }
            }

            Ok(map)
        }
    }

    deserializer.deserialize_seq(ScopesVisitor)
}
