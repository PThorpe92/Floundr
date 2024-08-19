use axum::{
    middleware::from_fn,
    routing::{delete, get, head, patch, post, put},
    Extension, Router,
};
use clap::{Parser, Subcommand};
use floundr::{
    auth::{
        auth_middleware, auth_token_get, check_scope_middleware, get_auth_clients, login_user,
        register_user, Auth,
    },
    blobs::{
        check_blob, delete_blob, get_blob, handle_upload_blob, handle_upload_session_chunk,
        put_upload_blob, put_upload_session_blob,
    },
    content_discovery::{
        create_repository, delete_repository, get_tags_list, get_v2, list_repositories,
    },
    database::{self, initdb, migrate_fresh},
    manifests::{delete_manifest, get_manifest, push_manifest},
    set_env,
    storage_driver::{Backend, DriverType},
    users::{delete_user, generate_token, get_users},
};
use http::Request;
use sqlx::SqliteConnection;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Parser)]
#[command(name = "floundr")]
#[command(version = "0.0.1")]
#[command(about = "OCI container registry server", long_about = None)]
struct App {
    #[arg(long, short = 'p', default_value = "8080")]
    port: Option<usize>,
    #[arg(long = "storage-path")]
    storage_path: Option<PathBuf>,
    #[arg(
        long = "home-dir",
        help = "path to the floundr home directory (default is $XDG_DATA_HOME/floundr)"
    )]
    container_home_dir: Option<PathBuf>,
    #[arg(long = "db-path", short = 'd', help = "path to the sqlite database")]
    db_path: Option<String>,
    #[arg(long, default_value = "local", value_enum)]
    driver: DriverType,
    #[arg(long, default_value = "false", help = "Enable debug mode")]
    debug: bool,
    #[command(subcommand)]
    command: Option<Box<Command>>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Migrate the database to a fresh state")]
    MigrateFresh,

    #[command(about = "Create a new repository with the given name")]
    NewRepo {
        #[arg(help = "name of the new repository", required(true))]
        name: String,
        #[arg(
            long,
            default_value = "false",
            help = "whether the new repository is public"
        )]
        public: bool,
    },

    #[command(
        about = "Create a new user with the given email",
        arg_required_else_help(true)
    )]
    NewUser {
        #[arg(help = "new user email", required(true))]
        email: String,
        #[arg(long, requires = "email", help = "new user password", required(true))]
        password: String,
    },

    #[command(about = "Generate a new API key for a user with administrative privileges")]
    GenKey {
        email: String,
        #[arg(
            long,
            default_value = "key.txt",
            help = "Output file for the generated key"
        )]
        output_file: String,
    },
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let args = App::parse();
    let home = args
        .container_home_dir
        .as_ref()
        .cloned()
        .map_or_else(
            || {
                std::env::var("FLOUNDR_HOME").unwrap_or(
                    dirs::data_local_dir()
                        .expect("unable to get XDG_LOCAL_DIR")
                        .join("floundr")
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
    let db_url = args
        .db_path
        .as_ref()
        .cloned()
        .unwrap_or_else(|| std::env::var("DB_PATH").unwrap_or("db.sqlite3".to_string()));
    let pool = initdb(&db_url).await;
    let mut conn = pool.acquire().await.expect("unable to acquire connection");
    set_env();
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
        .route("/auth/login", post(login_user))
        .route("/auth/token", get(auth_token_get))
        .route("/auth/register", post(register_user))
        .route("/auth/clients", get(get_auth_clients))
        .route("/repositories", get(list_repositories))
        .route("/repositories/:name/:public", post(create_repository))
        .route("/repositories/:name", delete(delete_repository))
        .route("/users", get(get_users))
        .route("/users/:email", delete(delete_user))
        .route("/users/:email/tokens", post(generate_token))
        .route("/v2/", get(get_v2))
        .route("/v2/:name/blobs/:digest", put(put_upload_blob))
        .route("/v2/:name/blobs/:digest", get(get_blob))
        .route("/v2/:name/blobs/:digest", head(check_blob))
        .route("/v2/:name/blobs/uploads/", post(handle_upload_blob))
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            put(put_upload_session_blob),
        )
        .route(
            "/v2/:name/blobs/uploads/:session_id",
            patch(handle_upload_session_chunk),
        )
        .route("/v2/:name/blobs/:digest", delete(delete_blob))
        .route("/v2/:name/tags/list", get(get_tags_list))
        .route("/v2/:name/manifests/:reference", get(get_manifest))
        .route("/v2/:name/manifests/:reference", put(push_manifest))
        .route("/v2/:name/manifests/:reference", delete(delete_manifest))
        .layer(from_fn(check_scope_middleware))
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
    match args.command.as_deref() {
        Some(Command::MigrateFresh) => {
            migrate_fresh(conn, None, None)
                .await
                .expect("unable to migrate database");
            info!("Migrating the database to a fresh state...");
        }
        Some(Command::NewRepo { name, public }) => {
            let _ = storage.create_repository(conn, name, *public).await;
            println!("Created new repository: {} (public: {})", name, public);
            std::process::exit(0);
        }
        Some(Command::NewUser { email, password }) => {
            let _ = database::seed_default_user(
                conn,
                Some(email.to_owned()),
                Some(password.to_owned()),
            )
            .await;
            println!("Creating new user: {} with password: {}", email, password);
            std::process::exit(0);
        }
        Some(Command::GenKey { email, output_file }) => {
            let secret = database::generate_secret(conn, None, email)
                .await
                .expect("unable to generate secret");
            tokio::fs::write(output_file, secret).await.unwrap();
            info!(
                "Generated new API key for: {} and saving to: {}",
                email, output_file
            );
            std::process::exit(0);
        }
        None => {
            info!("No subcommand was used. Running the default behavior...");
        }
    }
}
