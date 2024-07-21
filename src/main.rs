use axum::{
    http::StatusCode,
    routing::{delete, get, head, post, put},
    Router,
};
use clap::Parser;
use oci_rs::{
    blobs::{check_blob, delete_blob, get_blob, get_session_id, single_post_upload, upload_blob},
    content_discovery::get_tags_list,
    database::{create_new_repo, migrate_fresh},
    manifests::{get_manifest, list_repositories},
    storage,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::{net::SocketAddr, path::PathBuf, str::FromStr};
use tower_http::trace::TraceLayer;
use tracing::debug;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
#[command(name = "oci_rs")]
#[command(version = "0.0.1")]
#[command(about = "OCI compliant container registry server", long_about = None)]
struct App {
    #[arg(long, short = 'p', default_value = "8080")]
    port: Option<usize>,
    #[arg(long)]
    storage_path: Option<PathBuf>,
    #[arg(long)]
    container_home_dir: Option<PathBuf>,
    #[arg(long)]
    db_path: Option<PathBuf>,
    #[arg(long, default_value = "false")]
    migrate_fresh: bool,
    #[arg(long, help = "Create a new repository with a given name")]
    new_repo: Option<String>,
    #[arg(
        long,
        requires = "new_repo",
        help = "whether the new repository is public",
        default_value = "false"
    )]
    is_public: Option<bool>,
}

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(true)
        .with_line_number(true)
        .with_level(true)
        .finish();
    subscriber.init();
    let _ = dotenvy::dotenv().ok();
    let args = App::parse();
    let home = args.container_home_dir.as_ref().cloned().map_or_else(
        || std::env::var("CONTAINER_HOME_DIR").unwrap_or("".to_string()),
        |path| path.to_string_lossy().to_string(),
    );
    let storage =
        storage::StorageDriver::new(args.storage_path.as_ref().unwrap_or(&PathBuf::from(home)));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&std::env::var("DB_PATH").unwrap_or("oci_rs.db".to_string()))
        .await
        .expect("unable to connect to sqlite db pool");
    handle_args(&args, &pool).await;
    let app = app().with_state(pool).with_state(storage);
    let host = std::env::var("HOST").unwrap_or("127.0.0.1".to_string());
    let port = args.port.unwrap_or(8080);
    let addr = SocketAddr::from_str(&format!("{host}:{port}")).unwrap_or_else(|_| {
        eprintln!("Invalid address: {host}:{port}");
        std::process::exit(1);
    });
    println!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("unable to bind to port");

    axum::serve(listener, app.into_make_service())
        .await
        .expect("unable to start server");
}

fn app() -> Router<sqlx::SqlitePool> {
    Router::new()
        .layer(TraceLayer::new_for_http())
        .route("/v2/", get(get_v2))
        .route("/repositories", get(list_repositories))
        .route("/v2/:name/blobs/:digest", get(get_blob))
        .route("/v2/:name/blobs/:digest", head(check_blob))
        .route("/v2/:name/blobs/uploads", post(get_session_id))
        .route("/v2/:name/blobs/uploads/:digest", put(upload_blob))
        .route("/v2/:name/blobs/:digest", delete(delete_blob))
        .route("/v2/:name/blobs/uploads/:digest", post(single_post_upload))
        .route("/v2/:name/tags/list", get(get_tags_list))
        .route("/v2/:name/manifests/:reference", get(get_manifest))
}

async fn handle_args(args: &App, pool: &sqlx::SqlitePool) {
    if args.migrate_fresh {
        migrate_fresh(pool)
            .await
            .expect("unable to migrate database");
    }
    if let Some(name) = &args.new_repo {
        let _ = create_new_repo(name, args.is_public.unwrap_or(false), pool).await;
    }
}

/// GET /v2/
/// Return status code 200
/// Spec: 770
async fn get_v2() -> StatusCode {
    // TODO: return unauthorized if not authenticated
    debug!("GET /v2/");
    StatusCode::OK
}
