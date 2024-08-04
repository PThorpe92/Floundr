use axum::{
    routing::{delete, get, head, patch, post, put},
    Extension, Router,
};
use clap::Parser;
use harbor::{
    auth::{auth_middleware, login_user, oauth_token_get, register_user, Auth},
    blobs::{
        check_blob, delete_blob, get_blob, handle_upload_blob, single_upload_blob,
        upload_blob_session,
    },
    content_discovery::{create_repository, get_tags_list, get_v2, list_repositories},
    database::{self, initdb, migrate_fresh},
    manifests::{delete_manifest, get_manifest, push_manifest},
    storage_driver::{Backend, DriverType},
    users::{delete_user, generate_token, get_users},
};
use http::Request;
use sqlx::SqliteConnection;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;
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
    #[arg(long, help = "email for new user")]
    email: Option<String>,
    #[arg(long, requires = "email", help = "new user password")]
    password: Option<String>,
    #[arg(long, default_value = "local", value_enum)]
    driver: DriverType,
    #[arg(long, default_value = "false", help = "Enable debug mode")]
    debug: bool,
    #[arg(
        long,
        help = "generate new registry secret and write to file",
        default_missing_value = "secret.txt"
    )]
    secret: Option<String>,
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
    let home = args
        .container_home_dir
        .as_ref()
        .cloned()
        .map_or_else(
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
        )
        .parse::<PathBuf>()
        .expect("unable to parse home dir");
    let storage = Backend::new(
        DriverType::Local,
        args.storage_path.as_ref().unwrap_or(&home),
    );
    info!("storage path home: {:?}", storage.base_path());
    let email = args.email.clone();
    let password = args.password.clone();
    let pool = initdb(
        &std::path::Path::new(&home)
            .join("harbor.db")
            .to_string_lossy(),
        email.clone(),
        password.clone(),
    )
    .await;
    let mut conn = pool.acquire().await.expect("unable to acquire connection");
    let _ = handle_args(&args, &mut conn, &storage).await;
    let host = std::env::var("HOST").unwrap_or("127.0.0.1".to_string());
    let port = args.port.unwrap_or(8080);
    let addr = SocketAddr::from_str(&format!("{host}:{port}")).unwrap_or_else(|_| {
        eprintln!("Invalid address: {host}:{port}");
        std::process::exit(1);
    });
    info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("unable to bind to port");

    let routes = Router::new()
        .route("/v2/", get(get_v2))
        .route("/v2/auth/login", post(login_user))
        .route("/v2/auth/token", get(oauth_token_get))
        .route("/v2/auth/register", post(register_user))
        .route("/users", get(get_users))
        .route("/users/:email", delete(delete_user))
        .route("/users/:email/token", post(generate_token))
        .route("/repositories", get(list_repositories))
        .route("/repositories/:name/:public", post(create_repository))
        .route("/v2/:name/blobs/:digest", get(get_blob))
        .route("/v2/:name/blobs/:digest", head(check_blob))
        .route("/v2/:name/blobs/:digest", put(single_upload_blob))
        .route("/v2/:name/blobs/uploads/", post(handle_upload_blob))
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            put(upload_blob_session),
        )
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            patch(upload_blob_session),
        )
        .route("/v2/:name/blobs/:digest", delete(delete_blob))
        .route("/v2/:name/tags/list", get(get_tags_list))
        .route("/v2/:name/manifests/:reference", get(get_manifest))
        .route("/v2/:name/manifests/:reference", head(get_manifest))
        .route("/v2/:name/manifests/:reference", put(push_manifest))
        .route("/v2/:name/manifests/:reference", delete(delete_manifest))
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            auth_middleware,
        ))
        .layer(Extension(Arc::new(storage)))
        .layer(
            ServiceBuilder::new().layer(TraceLayer::new_for_http().make_span_with(
                |request: &Request<_>| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                        headers = ?request.headers(),
                    )
                },
            )),
        )
        .layer(Extension(axum::middleware::from_extractor::<Auth>()))
        .with_state(pool);

    axum::serve(listener, routes.into_make_service())
        .await
        .expect("unable to start server");
}

async fn handle_args(args: &App, conn: &mut SqliteConnection, storage: &Backend) {
    let email = args.email.clone();
    let password = args.password.clone();
    if args.migrate_fresh {
        migrate_fresh(conn, email.clone(), password.clone())
            .await
            .expect("unable to migrate database");
    }
    if let Some(name) = &args.new_repo {
        let _ = storage
            .create_repository(conn, name, args.public.unwrap_or(true))
            .await;
    }
    if args.email.as_ref().is_some() {
        let _ = database::seed_default_user(conn, email, password).await;
    }
    if let Some(ref file) = args.secret {
        let secret = database::generate_secret(conn, None)
            .await
            .expect("unable to generate secret");
        tokio::fs::write(file, secret).await.unwrap();
    }
}
