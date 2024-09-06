use crate::{
    events::AppEventHandler,
    requests::{
        create_new_api_key, create_new_user, create_repository, delete_repository, delete_user,
        get_all_users, get_manifests, get_repositories, get_tokens,
    },
    screens::{self, InputType, ScreenType},
    ConfigFile, Theme,
};
use dashmap::DashMap;
use lazy_static::lazy_static;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    widgets::ListItem,
    Frame, Terminal,
};
use reqwest::header::HeaderMap;
use serde::Deserialize;
use shared::UserResponse;
use shared::{AuthClient, ImageManifest, RegisterUserRequest, Repo};
use std::{
    io::{self, stdout},
    sync::{Arc, OnceLock, RwLock},
};
use tracing::{error, info};
use tui_input::{backend::crossterm::EventHandler, Input};

pub type AppResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct App {
    pub running: bool,
    pub config: ConfigFile,
    pub url: String,
    pub cursor: usize,
    pub current_screen: usize,
    pub screen_stack: Vec<ScreenType>,
    pub state: ratatui::widgets::ListState,
    pub scrollbar: ratatui::widgets::ScrollbarState,
    pub current_action: Option<InputType>,
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

#[allow(clippy::iter_next_slice)]
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

lazy_static! {
    pub static ref GLOBAL_REPO_LIST: Arc<RwLock<RepositoryList>> =
        Arc::new(RwLock::new(RepositoryList::default()));
    pub static ref MANIFESTS: Arc<DashMap<String, ImageManifest>> = Arc::new(DashMap::new());
    pub static ref USERS: Arc<RwLock<Vec<UserResponse>>> = Arc::new(RwLock::new(Vec::new()));
    pub static ref CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    pub static ref HEADERS: OnceLock<HeaderMap> = OnceLock::new();
    pub static ref ACTIVE_KEYS: Arc<RwLock<Vec<AuthClient>>> = Arc::new(RwLock::new(Vec::new()));
}

pub static DEFAULT_SCREENS: &[screens::ScreenType] = &[
    screens::ScreenType::Home,
    screens::ScreenType::Repos,
    screens::ScreenType::Users,
];

impl Default for App {
    fn default() -> Self {
        let config = ConfigFile::load();
        let url = config
            .url
            .as_ref()
            .expect("config file not properly loaded")
            .clone();
        info!("Using Floundr URL: {}", url);
        Self {
            url,
            config,
            running: true,
            cursor: 0,
            current_screen: 0,
            current_action: None,
            screen_stack: DEFAULT_SCREENS.to_vec(),
            state: ratatui::widgets::ListState::default(),
            scrollbar: ratatui::widgets::ScrollbarState::default(),
            mode: Mode::Normal,
            input: Input::default(),
            buffer: Vec::new(),
        }
    }
}

impl App {
    #[inline(always)]
    pub fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
    }
    #[inline(always)]
    pub fn insert_mode(&mut self) {
        self.mode = Mode::Insert;
    }
    #[inline(always)]
    fn reset_cursor(&mut self) {
        self.current_action = None;
        self.state.select(None);
        self.cursor = 0;
        self.scrollbar =
            ratatui::widgets::ScrollbarState::new(get_items(self.current_screen, None).len());
    }
    #[inline(always)]
    fn shuffle_screen_left(&mut self) {
        self.reset_cursor();
        self.current_screen = match self.current_screen.saturating_sub(1) {
            x if x >= self.screen_stack.len() => 0,
            x => x,
        }
    }
    #[inline(always)]
    pub fn set_action(&mut self, action: InputType) {
        self.mode = Mode::Insert;
        self.current_action = Some(action);
    }
    #[inline(always)]
    pub fn clear_action(&mut self) {
        self.current_action = None;
    }
    #[inline(always)]
    pub fn get_bg(&self) -> &ratatui::style::Color {
        &self.config.theme.as_ref().unwrap_or(&DEFAULT_THEME).bg
    }

    #[inline(always)]
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

    #[inline(always)]
    fn shuffle_screen_right(&mut self) {
        self.reset_cursor();
        self.current_screen = match self.current_screen.saturating_add(1) {
            x if x >= self.screen_stack.len() => self.screen_stack.len() - 1,
            x => x,
        };
    }

    pub fn handle_input(&mut self, kind: InputType) {
        self.clear_action();
        match kind {
            InputType::NewRepo => {
                let public = self.buffer.pop().unwrap_or("".to_string());
                let name = self.buffer.pop().unwrap_or("".to_string());
                let url = self.url.clone();
                tokio::spawn(async move {
                    let _ = create_repository(url, name, public.to_lowercase() == "y").await;
                });
            }
            InputType::CreateApiKey => {
                let url = self.url.clone();
                let name = self.buffer.pop().unwrap_or_default();
                tokio::spawn(async move {
                    let _ = create_new_api_key(url, name).await;
                });
            }
            InputType::DeleteUser => {
                let user = self.buffer.pop().unwrap_or_default();
                let url = self.url.clone();
                tokio::spawn(async move {
                    let _ = delete_user(url, user).await;
                });
            }
            InputType::DeleteRepo => {
                let repo = self.buffer.pop().unwrap_or_default();
                let url = self.url.clone();
                tokio::spawn(async move {
                    let _ = delete_repository(url, repo).await;
                });
            }
            InputType::NewUser => {
                match RegisterUserRequest::from_input_buff(
                    &self.buffer.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ) {
                    Ok(user) => {
                        self.buffer.clear();
                        let url = self.url.clone();
                        tokio::spawn(async move {
                            let _ = create_new_user(url, user).await;
                        });
                    }
                    Err(e) => {
                        error!("Error creating user: {:?}", e);
                    }
                }
            }
        };
    }

    fn cursor_up(&mut self) {
        self.scrollbar.prev();
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn cursor_down(&mut self) {
        self.scrollbar.next();
        let len = self.screen_stack[self.current_screen].get_cursor_len();
        if len == 0 {
            return;
        }
        self.cursor = match self.cursor.saturating_add(1) {
            x if x >= len => len - 1,
            x => x,
        };
    }

    fn render_screen(&mut self, frame: &mut Frame<'_>) -> AppResult<()> {
        match self.screen_stack[self.current_screen] {
            screens::ScreenType::Home => screens::repos::home_screen(frame, self),
            screens::ScreenType::Repos => screens::repos::repository_screen(frame, self),
            screens::ScreenType::Users => screens::users::user_management_screen(frame, self),
        }
        Ok(())
    }

    fn quit(&mut self) {
        self.running = false;
    }
}

pub fn get_items(curr: usize, selected: Option<usize>) -> Vec<ratatui::widgets::ListItem<'static>> {
    match DEFAULT_SCREENS[curr] {
        screens::ScreenType::Home => vec![
            ratatui::widgets::ListItem::new("Use Vim keybindings to navigate".to_string()),
            ratatui::widgets::ListItem::new("'h' & 'l' to move screens left/right ".to_string()),
        ],
        screens::ScreenType::Repos => GLOBAL_REPO_LIST
            .read()
            .unwrap()
            .repositories
            .iter()
            .map(|r| ratatui::widgets::ListItem::new(r.name.clone()))
            .collect::<Vec<ListItem>>(),
        screens::ScreenType::Users => match selected {
            Some(1) => USERS
                .read()
                .unwrap()
                .iter()
                .map(|u| {
                    ratatui::widgets::ListItem::new(
                        serde_json::to_string_pretty(u).unwrap_or_default(),
                    )
                })
                .collect::<Vec<ListItem>>(),
            Some(2) => ACTIVE_KEYS
                .read()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(idx, k)| {
                    ratatui::widgets::ListItem::new(format!(
                        "{}:\n\n{}",
                        idx + 1,
                        serde_json::to_string_pretty(k).unwrap_or_default(),
                    ))
                })
                .collect::<Vec<ListItem>>(),
            _ => vec![],
        },
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

    pub async fn fetch_data(&mut self) -> AppResult<()> {
        let url = self.app.url.clone();
        if let Err(err) = get_repositories(&url).await {
            error!("Unable to fetch repos: {:?}", err);
        }
        if let Err(err) = get_manifests(&url).await {
            error!("Unable to fetch manifests: {:?}", err);
        }
        if let Err(err) = get_all_users(&url).await {
            error!("Unable to fetch users: {:?}", err);
        }
        if let Err(err) = get_tokens(&url).await {
            error!("Unable to fetch active keys: {:?}", err);
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppResult<()> {
        match self.app.mode {
            Mode::Normal => match key.code {
                KeyCode::Char(ch) => match ch {
                    'q' => self.app.quit(),
                    'j' => self.app.cursor_down(),
                    'k' => self.app.cursor_up(),
                    'h' => self.app.shuffle_screen_left(),
                    'l' => self.app.shuffle_screen_right(),
                    'i' => self.app.insert_mode(),
                    'd' => match self.app.screen_stack[self.app.current_screen] {
                        ScreenType::Users => {
                            if self.app.state.selected().is_some() {
                                self.app.set_action(InputType::DeleteUser);
                            }
                        }
                        ScreenType::Repos => {
                            if self.app.state.selected().is_some() {
                                self.app.set_action(InputType::DeleteRepo);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                },
                KeyCode::Left => self.app.shuffle_screen_left(),
                KeyCode::Right => self.app.shuffle_screen_right(),
                KeyCode::Up => self.app.cursor_up(),
                KeyCode::Down => self.app.cursor_down(),
                KeyCode::Esc => {
                    self.app.reset_cursor();
                }
                KeyCode::Enter => self.app.state.select(Some(self.app.cursor)),
                _ => {}
            },
            Mode::Insert => match key.code {
                KeyCode::Esc => {
                    self.app.normal_mode();
                    self.app.reset_cursor();
                }
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
