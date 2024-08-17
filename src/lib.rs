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

#[derive(serde::Serialize, PartialEq, Eq, serde::Deserialize, Clone, Debug)]
pub enum Action {
    Pull,
    Push,
    Delete,
}
impl Action {
    pub fn check_permission(&self, actions: &[Action]) -> bool {
        // self here is the users available scope
        // actions is the requested scope
        match self {
            Action::Pull => actions.contains(&Action::Pull),
            Action::Push => actions.contains(&Action::Push),
            Action::Delete => true,
        }
    }

    pub fn from_request(req: &Request) -> Option<Self> {
        let path = req.uri().path();
        match *req.method() {
            Method::PUT | Method::POST | Method::PATCH => Some(Self::Push),
            Method::DELETE => Some(Self::Delete),
            _ => {
                if path.contains("blobs") {
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
pub struct UserScope(HashMap<Repo, Vec<Action>>);

impl UserScope {
    pub fn is_allowed(&self, requested: &str) -> bool {
        // this is going to be called on the claims scope injected into the request by the
        // middleware before it's assigned a scoped token
        tracing::info!("checking scope: {}", requested);
        let requested = UserScope::from_str(requested).unwrap_or_default();
        for (repo, requested_actions) in requested.0.iter() {
            if repo == "*" {
                return self.0.values().next().is_some_and(|avail| {
                    avail
                        .iter()
                        .all(|ax| ax.check_permission(requested_actions))
                });
            }
            if self.0.get(repo).is_some_and(|avail| {
                avail
                    .iter()
                    .any(|ax| !ax.check_permission(requested_actions))
            }) {
                return false;
            }
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
            let actions = parts[2].parse::<Action>().unwrap();
            scopes
                .entry(repo.to_string())
                .or_insert_with(Vec::new)
                .extend_from_slice(&actions.to_vec());
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
        scopes.0.insert(
            row.name,
            Vec::from([
                if row.pull { Action::Pull } else { continue },
                if row.push { Action::Push } else { continue },
                if row.del { Action::Delete } else { continue },
            ]),
        );
    }
    scopes
}

pub async fn get_admin_scopes(conn: &mut SqliteConnection) -> UserScope {
    let repos = database::get_repositories(conn, false).await;
    UserScope(
        repos
            .into_iter()
            .map(|row| (row, Vec::from([Action::Pull, Action::Push, Action::Delete])))
            .collect::<HashMap<Repo, _>>(),
    )
}

pub async fn default_public_scopes(conn: &mut SqliteConnection) -> UserScope {
    let repos = database::get_repositories(conn, true).await;
    let mut scopes = HashMap::new();
    for repo in repos {
        scopes.insert(repo, vec![Action::Pull]);
    }
    UserScope(scopes)
}
