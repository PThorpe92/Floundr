pub mod auth;
pub mod blobs;
pub mod codes;
pub mod content_discovery;
pub mod database;
pub mod endpoints;
pub mod manifests;
pub mod storage;
pub mod storage_driver;
pub mod users;
pub mod util;
use std::{collections::HashMap, str::FromStr};

use axum::extract::Request;
use http::Method;
use lazy_static::lazy_static;
use sqlx::SqliteConnection;
use tokio::sync::OnceCell;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

lazy_static! {
    pub static ref APP_URL: OnceCell<String> = OnceCell::new();
    pub static ref JWT_SECRET: OnceCell<String> = OnceCell::new();
}

pub fn set_env() {
    let level = match std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()) {
        s if s.eq_ignore_ascii_case("trace") => tracing::Level::TRACE,
        s if s.eq_ignore_ascii_case("debug") => tracing::Level::DEBUG,
        s if s.eq_ignore_ascii_case("info") => tracing::Level::INFO,
        s if s.eq_ignore_ascii_case("warn") => tracing::Level::WARN,
        s if s.eq_ignore_ascii_case("error") => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(level)
        .with_ansi(true)
        .pretty()
        .finish();
    subscriber.with(tracing_subscriber::fmt::layer()).init();
    let app_url = std::env::var("APP_URL").expect("APP_URL must be set");
    APP_URL.set(app_url).unwrap();
    let jwt_secret = std::env::var("JWT_SECRET_KEY").expect("JWT_SECRET_KEY must be set");
    JWT_SECRET.set(jwt_secret).unwrap();
}

#[derive(serde::Serialize, PartialEq, Eq, serde::Deserialize, Clone, Copy, Debug)]
pub enum Action {
    Pull,
    Push,
    Delete,
}

impl PartialOrd for Action {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Action {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Action::Pull, Action::Pull) => std::cmp::Ordering::Equal,
            (Action::Pull, Action::Push) => std::cmp::Ordering::Less,
            (Action::Pull, Action::Delete) => std::cmp::Ordering::Less,
            (Action::Push, Action::Pull) => std::cmp::Ordering::Greater,
            (Action::Push, Action::Push) => std::cmp::Ordering::Equal,
            (Action::Push, Action::Delete) => std::cmp::Ordering::Less,
            (Action::Delete, Action::Pull) => std::cmp::Ordering::Greater,
            (Action::Delete, Action::Push) => std::cmp::Ordering::Greater,
            (Action::Delete, Action::Delete) => std::cmp::Ordering::Equal,
        }
    }
}

impl Action {
    pub fn check_permission(&self, requested: Action) -> bool {
        match self {
            Action::Delete => true, // Delete can perform any action
            Action::Push => requested.eq(&Action::Pull) || requested.eq(&Action::Push),
            Action::Pull => requested.eq(&Action::Pull),
        }
    }

    pub fn from_request(req: &Request) -> Option<Self> {
        let path = req.uri().path();
        match *req.method() {
            Method::PUT | Method::POST | Method::PATCH => Some(Self::Push),
            Method::DELETE => Some(Self::Delete),
            _ => {
                if path.starts_with("/v2/") {
                    Some(Self::Pull)
                } else {
                    None
                }
            }
        }
    }
    pub fn to_vec(&self) -> Vec<Self> {
        match self {
            Action::Pull => vec![Action::Pull],
            Action::Push => vec![Action::Push, Action::Pull],
            Action::Delete => vec![Action::Delete, Action::Push, Action::Pull],
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Pull => write!(f, "pull"),
            Action::Push => write!(f, "push"),
            Action::Delete => write!(f, "delete"),
        }
    }
}
impl std::str::FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // grant the highest level of access.
            // delete can always push, push can always pull
            "pull" => Ok(Action::Pull),
            "push" => Ok(Action::Push),
            "delete" => Ok(Action::Delete),
            "push,pull" => Ok(Action::Push),
            "push,pull,delete" => Ok(Action::Delete),
            "*" => Ok(Action::Delete),
            _ => Err(format!("invalid action: {}", s)),
        }
    }
}
pub type Repo = String;
#[derive(serde::Serialize, Default, serde::Deserialize, Clone, Debug)]
pub struct UserScope(HashMap<Repo, Action>);

impl UserScope {
    pub fn is_allowed(&self, repo: &str, action: Action) -> bool {
        tracing::info!("checking scope: {} {}", repo, action);
        if repo == "*" {
            if self
                .0
                .values()
                .any(|available_action| !available_action.check_permission(action))
            {
                return false;
            }
        } else if let Some(available_action) = self.0.get(repo) {
            if !available_action.check_permission(action) {
                return false;
            }
        } else {
            return false;
        }
        true
    }
}

impl FromStr for UserScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut scopes = HashMap::new();
        for scope in s.split(' ') {
            let parts = scope.split(':').collect::<Vec<&str>>();
            if parts.len() != 3 {
                return Err(format!("invalid scope: {}", scope));
            }
            let repo = parts[1];
            let action = parts[2].parse::<Action>().unwrap();
            scopes
                .entry(repo.to_string())
                .and_modify(|existing_action| {
                    if action > *existing_action {
                        *existing_action = action;
                    }
                })
                .or_insert(action);
        }
        tracing::info!("parsed scopes: {:?}", scopes);
        Ok(UserScope(scopes))
    }
}

pub async fn get_user_scopes(conn: &mut SqliteConnection, user_id: &str) -> UserScope {
    let rows = sqlx::query!(
        r#"
        SELECT r.id, r.name, r.is_public, rs.push, rs.pull, rs.del
        FROM repositories r
        JOIN repository_scopes rs ON r.id = rs.repository_id
        WHERE rs.user_id = ?"#,
        user_id
    )
    .fetch_all(conn)
    .await
    .expect("unable to fetch user scopes");
    let mut scopes = UserScope::default();
    for row in rows {
        let highest_action = if row.del {
            Action::Delete
        } else if row.push {
            Action::Push
        } else {
            Action::Pull
        };
        scopes.0.insert(row.name, highest_action);
    }
    scopes
}

pub async fn get_admin_scopes(conn: &mut SqliteConnection) -> UserScope {
    let repos = database::get_repositories(conn, false).await;
    UserScope(
        repos
            .into_iter()
            .map(|row| (row, Action::Delete))
            .collect::<HashMap<Repo, _>>(),
    )
}

pub async fn default_public_scopes(conn: &mut SqliteConnection) -> UserScope {
    let repos = database::get_repositories(conn, true).await;
    let mut scopes = HashMap::new();
    for repo in repos {
        scopes.insert(repo, Action::Pull);
    }
    UserScope(scopes)
}
