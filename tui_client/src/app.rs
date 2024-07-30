use dashmap::DashMap;
use lazy_static::lazy_static;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    Frame, Terminal,
};
use reqwest::{header::HeaderMap, Response};
use serde::Deserialize;
use std::{
    io::{self, stdout},
    sync::{Arc, OnceLock, RwLock},
};
use tracing::{debug, info};
use tui_input::{backend::crossterm::EventHandler, Input};
pub type AppResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

use crate::{
    events::AppEventHandler,
    screens::{self, InputType},
    ConfigFile, Theme,
};

pub struct App {
    pub running: bool,
    pub config: ConfigFile,
    pub url: String,
    pub cursor: usize,
    pub current_screen: usize,
    pub screen_stack: Vec<screens::ScreenType>,
    pub state: ratatui::widgets::ListState,
    pub mode: Mode,
    pub input: Input,
    pub buffer: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct RepositoryList {
    pub repositories: Vec<Repo>,
}

impl<'a> Iterator for &'a RepositoryList {
    type Item = &'a Repo;
    fn next(&mut self) -> Option<Self::Item> {
        self.repositories.iter().next()
    }
}
static DEFAULT_THEME: Theme = Theme {
    fg: ratatui::style::Color::White,
    bg: ratatui::style::Color::DarkGray,
    highlight: ratatui::style::Color::LightYellow,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub name: String,
    pub is_public: bool,
    pub blob_count: i64,
    pub tag_count: i64,
    pub tags: Vec<String>,
    pub manifest_count: i64,
    pub file_path: String,
    pub disk_usage: usize,
    pub driver: String,
    pub num_layers: i64,
}
impl Repo {
    pub fn calculate_mb(&self) -> f64 {
        self.disk_usage as f64 / 1024.0
    }
}

lazy_static! {
    pub static ref GLOBAL_REPO_LIST: Arc<RwLock<RepositoryList>> =
        Arc::new(RwLock::new(RepositoryList::default()));
    pub static ref MANIFESTS: Arc<DashMap<usize, Vec<String>>> = Arc::new(DashMap::new());
    pub static ref CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    pub static ref HEADERS: OnceLock<HeaderMap> = OnceLock::new();
}

pub static DEFAULT_SCREENS: &[screens::ScreenType] =
    &[screens::ScreenType::Home, screens::ScreenType::Repos];

impl Default for App {
    fn default() -> Self {
        let config = ConfigFile::load();
        let url = config
            .url
            .as_ref()
            .expect("config file not properly loaded")
            .clone();
        info!("Using Harbor URL: {}", url);
        Self {
            url,
            config,
            running: true,
            cursor: 0,
            current_screen: 0,
            screen_stack: DEFAULT_SCREENS.to_vec(),
            state: ratatui::widgets::ListState::default(),
            mode: Mode::Normal,
            input: Input::default(),
            buffer: Vec::new(),
        }
    }
}

impl App {
    pub fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
    }
    pub fn insert_mode(&mut self) {
        self.mode = Mode::Insert;
    }
    fn shuffle_screen_left(&mut self) {
        self.current_screen = match self.current_screen.saturating_sub(1) {
            x if x >= self.screen_stack.len() => 0,
            x => x,
        }
    }

    pub fn get_bg(&self) -> &ratatui::style::Color {
        &self.config.theme.as_ref().unwrap_or(&DEFAULT_THEME).bg
    }

    pub fn get_fg(&self) -> &ratatui::style::Color {
        match self.mode {
            Mode::Normal => &self.config.theme.as_ref().unwrap_or(&DEFAULT_THEME).fg,
            Mode::Insert => {
                &self
                    .config
                    .theme
                    .as_ref()
                    .unwrap_or(&DEFAULT_THEME)
                    .highlight
            }
        }
    }

    fn shuffle_screen_right(&mut self) {
        self.current_screen = match self.current_screen.saturating_add(1) {
            x if x >= self.screen_stack.len() => self.screen_stack.len() - 1,
            x => x,
        }
    }

    pub fn handle_input(&mut self, kind: InputType) -> AppResult<()> {
        match kind {
            InputType::NewRepo => {
                let public = self.buffer.pop().unwrap_or("".to_string());
                let name = self.buffer.pop().unwrap_or("".to_string());
                let url = self.url.clone();
                tokio::spawn(async move {
                    let _ = create_repository(url, name, public.to_lowercase() == "y").await;
                })
            }
        };
        Ok(())
    }

    fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn cursor_down(&mut self) {
        let len = GLOBAL_REPO_LIST.read().unwrap().repositories.len();
        self.cursor = match self.cursor.saturating_add(1) {
            x if x >= len => len,
            x => x,
        };
    }

    fn render_screen(&mut self, frame: &mut Frame<'_>) -> AppResult<()> {
        screens::repos::repository_screen(frame, self);
        Ok(())
    }

    pub fn get_items(&self) -> Vec<ratatui::widgets::ListItem> {
        match self.screen_stack[self.current_screen] {
            screens::ScreenType::Home => vec![
                ratatui::widgets::ListItem::new("Repos"),
                ratatui::widgets::ListItem::new("Images"),
            ],
            screens::ScreenType::Repos => GLOBAL_REPO_LIST
                .read()
                .unwrap()
                .repositories
                .iter()
                .map(|r| ratatui::widgets::ListItem::new(r.name.clone()))
                .collect(),
            _ => vec![],
        }
    }

    fn quit(&mut self) {
        self.running = false;
    }
}

pub async fn create_repository(url: String, name: String, public: bool) -> AppResult<()> {
    let client = CLIENT.get().unwrap();
    let res = client
        .post(format!("{}/repositories/{}/{}", url, name, public))
        .send()
        .await?;
    if res.status().is_success() {
        let new = client.get(format!("{}/repositories", url)).send().await?;
        let repos: RepositoryList = new.json().await.expect("failed to parse repos");
        *GLOBAL_REPO_LIST.write().unwrap() = repos;
        info!("Repository created successfully");
        Ok(())
    } else {
        debug!("{:?}", res);
        info!("Failed to create repository");
        Err("Failed to create repository".into())
    }
}

pub async fn send_get_request(url: String) -> AppResult<Response> {
    let client = CLIENT.get().unwrap();
    let headers = HEADERS.get().unwrap();
    let req = client.get(url).headers(headers.to_owned()).build()?;
    info!("sending request {:?}", req);
    let res = client.execute(req).await?;
    if res.status().is_success() {
        Ok(res)
    } else {
        Err("Failed to fetch data".into())
    }
}

pub struct Tui {
    pub app: App,
    terminal: Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    pub events: AppEventHandler,
}

impl Tui {
    pub fn new(
        term: Terminal<CrosstermBackend<io::Stdout>>,
        events: AppEventHandler,
        app: App,
    ) -> Self {
        Self {
            app,
            terminal: term,
            events,
        }
    }

    pub fn init(&mut self) -> AppResult<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let _ = Self::reset();
            panic_hook(panic);
        }));

        Ok(())
    }

    pub fn handle_events(&mut self) -> io::Result<bool> {
        if let Event::Key(key) = event::read()? {
            let _ = self.handle_key(key).is_ok();
        }
        Ok(false)
    }

    pub fn draw(&mut self) -> AppResult<()> {
        let _ = self.terminal.draw(|frame| {
            self.app
                .render_screen(frame)
                .expect("failed to render screen")
        });
        Ok(())
    }

    pub async fn refresh_repositories(&mut self) -> AppResult<()> {
        self.get_repositories().await?;
        Ok(())
    }

    pub async fn get_manifests(&mut self) -> AppResult<()> {
        let client = CLIENT.get().unwrap();
        let repos = GLOBAL_REPO_LIST.read().unwrap().repositories.clone();
        for (idx, repo) in repos.iter().enumerate() {
            let res = client
                .get(format!(
                    "{}/{}/manifests/{}",
                    repo.name, self.app.url, repo.tags[idx]
                ))
                .send()
                .await?;
            let manifests: Vec<String> = res.json().await?;
            MANIFESTS.insert(idx, manifests);
        }
        Ok(())
    }

    pub async fn get_repositories(&mut self) -> AppResult<()> {
        let resp = send_get_request(format!("{}/repositories", self.app.url)).await?;
        let repos: RepositoryList = resp.json().await?;
        debug!("{:?}", repos.repositories);
        *GLOBAL_REPO_LIST.write().unwrap() = repos;
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppResult<()> {
        match self.app.mode {
            Mode::Normal => match key.code {
                KeyCode::Char(ch) => match ch {
                    'q' => self.app.running = false,
                    'j' => self.app.cursor_down(),
                    'k' => self.app.cursor_up(),
                    'h' => self.app.shuffle_screen_left(),
                    'l' => self.app.shuffle_screen_right(),
                    'i' => self.app.insert_mode(),
                    _ => {}
                },
                KeyCode::Left => self.app.shuffle_screen_left(),
                KeyCode::Right => self.app.shuffle_screen_right(),
                KeyCode::Up => self.app.cursor_up(),
                KeyCode::Down => self.app.cursor_down(),
                KeyCode::Esc => self.app.quit(),
                _ => {}
            },
            Mode::Insert => match key.code {
                KeyCode::Esc => self.app.normal_mode(),
                KeyCode::Enter => {
                    self.app.buffer.push(self.app.input.value().to_string());
                    self.app.input.reset();
                }
                _ => {
                    let _ = self.app.input.handle_event(&Event::Key(key));
                }
            },
        }
        Ok(())
    }

    fn reset() -> AppResult<()> {
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    pub fn exit(&mut self) -> AppResult<()> {
        let _ = disable_raw_mode();
        self.terminal.show_cursor()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }
}
