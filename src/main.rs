use axum::{
    http::StatusCode,
    routing::{delete, get, head, patch, post, put},
    Extension, Router,
};
use clap::Parser;
use http::Request;
use oci_rs::{
    blobs::{check_blob, delete_blob, get_blob, handle_upload_blob, upload_blob},
    content_discovery::get_tags_list,
    database::{initdb, migrate_fresh},
    manifests::{
        create_repository, delete_manifest, get_manifest, list_repositories, push_manifest,
    },
    storage_driver::{Backend, DriverType},
};
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::{debug, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    db_path: Option<String>,
    #[arg(long, default_value = "false")]
    migrate_fresh: bool,
    #[arg(long, help = "Create a new repository with a given name")]
    new_repo: Option<String>,
    #[arg(
        long,
        requires = "new_repo",
        help = "whether the new repository is public",
        default_missing_value = "true"
    )]
    public: Option<bool>,
    #[arg(long, default_value = "local", value_enum)]
    driver: DriverType,
}

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(true)
        .pretty()
        .finish();
    subscriber.with(tracing_subscriber::fmt::layer()).init();
    let _ = dotenvy::dotenv().ok();
    let args = App::parse();
    let home = args.container_home_dir.as_ref().cloned().map_or_else(
        || {
            std::env::var("HARBOR_HOME").unwrap_or(
                dirs::data_local_dir()
                    .expect("unable to get XDG_LOCAL_DIR")
                    .join("harbor")
                    .to_string_lossy()
                    .to_string(),
            )
        },
        |path| path.to_string_lossy().to_string(),
    );

    let storage = Backend::new(
        DriverType::Local,
        args.storage_path
            .unwrap_or_else(|| std::path::Path::new(&home).into()),
    );
    info!("storage path home: {:?}", storage.base_path());
    let pool = initdb(
        &std::path::Path::new(&home)
            .join("harbor.db")
            .to_string_lossy(),
    )
    .await;
    let mut conn = pool.acquire().await.expect("unable to acquire connection");
    if args.migrate_fresh {
        migrate_fresh(&mut conn)
            .await
            .expect("unable to migrate database");
    }
    if let Some(name) = &args.new_repo {
        let _ = storage
            .create_repository(&mut conn, name, args.public.unwrap_or(true))
            .await;
    }

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

    let app = Router::new()
        .route("/v2/", get(get_v2))
        .layer(
            ServiceBuilder::new().layer(TraceLayer::new_for_http().make_span_with(
                |request: &Request<_>| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                        status_code = tracing::field::Empty,
                    )
                },
            )),
        )
        .route("/repositories", get(list_repositories))
        .route("/repositories/:name/:public", post(create_repository))
        .route("/v2/:name/blobs/:digest", get(get_blob))
        .route("/v2/:name/blobs/:digest", head(check_blob))
        .route("/v2/:name/blobs/uploads/", post(handle_upload_blob))
        .route("/v2/:name/blobs/uploads/:session_id", put(upload_blob))
        .route("/v2/:name/blobs/uploads/:session_id", patch(upload_blob))
        .route("/v2/:name/blobs/:digest", delete(delete_blob))
        .route("/v2/:name/tags/list", get(get_tags_list))
        .route("/v2/:name/manifests/:reference", get(get_manifest))
        .route("/v2/:name/manifests/:reference", put(push_manifest))
        .route("/v2/:name/manifests/:reference", delete(delete_manifest))
        .layer(Extension(Arc::new(storage)))
        .with_state(pool);

    axum::serve(listener, app.into_make_service())
        .await
        .expect("unable to start server");
}

/// GET /v2/
/// Return status code 200
/// Spec: 770
async fn get_v2() -> StatusCode {
    // TODO: return unauthorized if not authenticated
    debug!("GET /v2/");
    StatusCode::OK
}
