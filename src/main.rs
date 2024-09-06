use axum_server::tls_rustls::RustlsConfig;
use clap::{Parser, Subcommand};
use floundr::{
    database::{self, initdb, migrate_fresh},
    endpoints::{redirect_http_to_https, register_routes, Ports},
    set_env,
    storage_driver::{Backend, DriverType},
};
use sqlx::SqliteConnection;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};
use tracing::info;

#[derive(Parser)]
#[command(name = "floundr")]
#[command(version = "0.0.1")]
#[command(about = "OCI container registry server", long_about = None)]
struct App {
    #[arg(long, short = 'p', default_value = "8080")]
    port: Option<u16>,
    #[arg(long = "storage-path")]
    storage_path: Option<PathBuf>,
    #[arg(
        long = "home-dir",
        help = "path to the floundr home directory (default is $XDG_DATA_HOME/floundr)"
    )]
    container_home_dir: Option<PathBuf>,
    #[arg(long = "ssl", default_value = "false", help = "enable https")]
    ssl: bool,
    #[arg(
        long = "cert-path",
        help = "path to the certificate file",
        requires = "ssl"
    )]
    cert_path: Option<String>,
    #[arg(
        long = "key-path",
        help = "path to the private key file",
        requires = "cert_path"
    )]
    key_path: Option<String>,
    #[arg(
        long = "https-port",
        default_value = "443",
        help = "port to serve tls on"
    )]
    https_port: Option<u16>,
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

    let routes = register_routes(pool, Arc::new(storage));
    let ports = Ports(args.port.unwrap_or(8080), args.https_port.unwrap_or(443));

    if args.ssl {
        let addr = SocketAddr::from_str(&format!("{host}:{}", ports.1)).unwrap_or_else(|_| {
            eprintln!("Invalid address: {host}:{}", ports.1);
            std::process::exit(1);
        });
        let cert = match args.cert_path {
            Some(cert_path) => PathBuf::from(cert_path),
            None => PathBuf::from("./config/floundr-key.pem"),
        };
        let key = match args.key_path {
            Some(key_path) => PathBuf::from(key_path),
            None => PathBuf::from("./config/floundr-key.pem"),
        };
        let config = RustlsConfig::from_pem_file(cert, key)
            .await
            .expect("unable to find tls certificates");

        tokio::spawn(redirect_http_to_https(ports));

        axum_server::bind_rustls(addr, config)
            .serve(routes.into_make_service())
            .await
            .expect("unable to start server");
        info!("Listening on {}", addr);
    } else {
        let addr = SocketAddr::from_str(&format!("{host}:{}", ports.0)).unwrap_or_else(|_| {
            eprintln!("Invalid address: {host}:{}", ports.0);
            std::process::exit(1);
        });
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("unable to bind to port");
        axum::serve(listener, routes.into_make_service())
            .await
            .expect("unable to start server");
        info!("Listening on {}", addr);
    }
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
