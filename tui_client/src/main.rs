use base64::{alphabet::URL_SAFE, engine::GeneralPurposeConfig, Engine};
use ratatui::prelude::*;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT},
    Client,
};
use std::io::{self};
use tracing::{info, Level};
use tui_client::{
    app::{App, Tui, CLIENT, HEADERS},
    events::AppEventHandler,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create("tui.log")?;
    // only log to file
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(file)
        .init();

    let _span = tracing::span!(tracing::Level::DEBUG, "main");
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    let app = App::default();
    let mut headers = HeaderMap::new();
    let mut value = String::new();
    base64::engine::GeneralPurpose::new(&URL_SAFE, GeneralPurposeConfig::default()).encode_string(
        format!(
            "{}:{}",
            app.config.email.as_ref().unwrap(),
            app.config.password.as_ref().unwrap()
        ),
        &mut value,
    );
    let token_value = HeaderValue::from_str(&format!("Basic {}", value))?;
    let user_agent = HeaderValue::from_str("harbor-tui")?;
    info!("user agent is {:?}", user_agent);
    info!("token value is {:?}", token_value);
    headers.append(USER_AGENT, user_agent);
    headers.append(AUTHORIZATION, token_value);
    let _ = HEADERS.set(headers.clone());
    info!("headers are {:?}", headers);
    let client = Client::builder().default_headers(headers).build()?;
    let _ = CLIENT.set(client);
    let mut tui = Tui::new(terminal, AppEventHandler::new(100), app);
    tui.init()?;
    if tui.get_repositories().await.is_err() {
        println!("Unable to fetch repositories, please set the HARBOR_URL env var, or the url in the config file to connect to the harbor registry");
    }
    while tui.app.running {
        let _ = tui.draw();
        let _ = tui.handle_events();
    }
    tui.exit()?;
    Ok(())
}
