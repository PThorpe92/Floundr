use ratatui::prelude::*;
use std::io::{self};
use tracing::Level;
use tui_client::{app::Tui, events::AppEventHandler};

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
    let mut tui = Tui::new(terminal, AppEventHandler::new(100));
    tui.init()?;
    if tui.app.get_repositories().await.is_err() {
        println!("Unable to fetch repositories, please set the HARBOR_URL env var, or the url in the config file to connect to the harbor registry");
    }
    while tui.app.running {
        let _ = tui.draw();
        let _ = tui.handle_events();
    }
    tui.exit()?;
    Ok(())
}
