pub static OCI_CONTENT_HEADER: &str = "application/vnd.oci.image.index.v1+json";
pub static DOCKER_DIGEST: &str = "Docker-Content-Digest";
pub static MANIFEST_CONTENT_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";
use chrono::NaiveDateTime;
use serde::{self, Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub name: String,
    pub is_public: bool,
    pub blob_count: i64,
    pub tag_count: i64,
    pub tags: Vec<String>,
    pub manifest_count: i64,
    pub file_path: String,
    pub disk_usage: usize,
    pub driver: String,
    pub num_layers: i64,
}
impl Repo {
    pub fn calculate_mb(&self) -> f64 {
        // Convert bytes to MB
        (self.disk_usage as f64 / 1024.0) / 1024.0
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    pub schema_version: i32,
    pub media_type: Option<String>,
    pub config: Option<Descriptor>,
    pub layers: Vec<Descriptor>,
    pub annotations: Option<HashMap<String, String>>,
}
impl Default for ImageManifest {
    fn default() -> Self {
        ImageManifest {
            schema_version: 2,
            media_type: Some(MANIFEST_CONTENT_TYPE.to_string()),
            config: Some(Descriptor {
                media_type: Some("application/vnd.oci.image.config.v2+json".to_string()),
                size: 0,
                digest: "".to_string(),
            }),
            layers: Vec::new(),
            annotations: None,
        }
    }
}

#[derive(Deserialize, Serialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: Option<String>,
    pub size: i32,
    pub digest: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    pub architecture: String,
    pub os: String,
    pub created: Option<String>,
    pub author: Option<String>,
    pub config: Config,
    pub rootfs: RootFS,
    pub history: Vec<History>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub env: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RootFS {
    pub type_: String,
    pub diff_ids: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub author: Option<String>,
    pub comment: Option<String>,
}

#[derive(serde::Serialize, Default, Clone, serde::Deserialize, Debug)]
pub struct User {
    #[serde(skip_deserializing)]
    pub id: String,
    pub is_admin: bool,
    pub email: String,
    #[serde(skip_serializing, default = "String::new")]
    pub password: String,
    #[serde(skip)]
    pub created_at: NaiveDateTime,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
pub struct UserResponse {
    pub user: User,
    pub scopes: Vec<RepoScope>,
}

#[derive(serde::Serialize, Default, serde::Deserialize, Debug)]
pub struct RepoScope {
    pub repo: String,
    pub scope: Vec<String>,
}
impl User {
    pub fn new(email: &str, password: &str, admin: bool) -> Self {
        User {
            id: uuid::Uuid::new_v4().to_string(),
            is_admin: admin,
            email: email.to_string(),
            password: password.to_string(),
            created_at: chrono::Utc::now().naive_utc(),
        }
    }
    pub fn validate(&self) -> bool {
        self.email.is_ascii()
            && self.password.len() > 8
            && self.password.chars().any(char::is_numeric)
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AuthClient {
    #[serde(skip)]
    pub id: i64,
    pub client_id: String,
    pub secret: String,
    pub email: String,
    pub created_at: NaiveDateTime,
}

impl AuthClient {
    pub fn new(client_id: &str, secret: &str, email: &str) -> Self {
        AuthClient {
            id: 0,
            client_id: client_id.to_string(),
            secret: secret.to_string(),
            email: email.to_string(),
            created_at: chrono::Utc::now().naive_utc(),
        }
    }
}
impl Default for AuthClient {
    fn default() -> Self {
        AuthClient {
            id: 0,
            client_id: "".to_string(),
            secret: "".to_string(),
            email: "".to_string(),
            created_at: chrono::Utc::now().naive_utc(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RegisterUserRequest {
    pub email: String,
    pub password: String,
    pub confirm_password: String,
    pub is_admin: bool,
}
impl RegisterUserRequest {
    pub fn new(email: &str, password: &str, confirm_password: &str, is_admin: bool) -> Self {
        RegisterUserRequest {
            email: email.to_string(),
            password: password.to_string(),
            confirm_password: confirm_password.to_string(),
            is_admin,
        }
    }
    pub fn from_input_buff(buff: &[&str]) -> Result<Self, String> {
        // validate input. massword and
        if buff.len() != 4 {
            return Err("Invalid input".to_string());
        };
        let req = Self::new(buff[0], buff[1], buff[2], buff[3] == "y");
        match req.validate() {
            true => Ok(req),
            false => Err("Invalid input".to_string()),
        }
    }

    pub fn validate(&self) -> bool {
        self.email.is_ascii()
            && self.password.len() > 8
            && self.password.chars().any(char::is_numeric)
            && self.password == self.confirm_password
    }
}
