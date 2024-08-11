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
    if app.config.secret.is_none() {
        login_basic(&app.url).await?;
    } else {
        login_bearer(&app.url, &mut headers, app.config.secret.as_ref().unwrap()).await?;
    }
    base64::engine::GeneralPurpose::new(&URL_SAFE, GeneralPurposeConfig::default()).encode_string(
        format!(
            "{}:{}",
            app.config.email.as_ref().unwrap(),
            app.config.password.as_ref().unwrap()
        ),
        &mut value,
    );
    let token_value = HeaderValue::from_str(&format!("Basic {}", value))?;
    let user_agent = HeaderValue::from_str("floundr-tui")?;
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
        println!("Unable to fetch repositories, please set the FLOUNDR_URL env var, or the url in the config file to connect to the registry");
    }
    while tui.app.running {
        let _ = tui.draw();
        let _ = tui.handle_events();
    }
    tui.exit()?;
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct TokenResponse {
    token: String,
}

async fn login_basic(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    let mut value = String::new();
    base64::engine::GeneralPurpose::new(&URL_SAFE, GeneralPurposeConfig::default())
        .encode_string("floundr_admin:admin", &mut value);
    let token_value = HeaderValue::from_str(&format!("Basic {}", value))?;
    let user_agent = HeaderValue::from_str("floundr-tui")?;
    headers.append(USER_AGENT, user_agent.clone());
    headers.append(AUTHORIZATION, token_value);
    let response = client
        .get(format!("{}/v2/auth/token", url))
        .headers(headers.clone())
        .send()
        .await?;
    let body = response.text().await?;
    let token: TokenResponse = serde_json::from_str(&body)?;
    let token_val = HeaderValue::from_str(&format!("Bearer {}", token.token))?;
    headers.clear();
    headers.insert(AUTHORIZATION, token_val);
    headers.insert(USER_AGENT, user_agent);
    let _ = HEADERS.set(headers);
    Ok(())
}

async fn login_bearer(
    url: &str,
    headers: &mut HeaderMap,
    secret: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let token_value = HeaderValue::from_str(&format!("Bearer {}", secret))?;
    let user_agent = HeaderValue::from_str("floundr-tui")?;
    let client = Client::default();
    headers.insert(USER_AGENT, user_agent.clone());
    headers.insert(AUTHORIZATION, token_value);
    let response = client
        .get(format!("{}/v2/auth/token", url))
        .headers(headers.clone())
        .send()
        .await?;
    let body: TokenResponse = response.json().await?;
    info!("using api key/bearer auth");
    headers.clear();
    let value = HeaderValue::from_str(&format!("Bearer {}", body.token))?;
    headers.insert(AUTHORIZATION, value);
    headers.insert(USER_AGENT, user_agent);
    let _ = HEADERS.set(headers.clone());
    Ok(())
}
