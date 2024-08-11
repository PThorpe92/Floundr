pub mod app;
pub mod events;

pub mod screens;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ConfigFile {
    pub url: Option<String>,
    pub database_url: Option<String>,
    pub secret: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub theme: Option<Theme>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Theme {
    pub fg: ratatui::style::Color,
    pub bg: ratatui::style::Color,
    pub highlight: ratatui::style::Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: ratatui::style::Color::White,
            bg: ratatui::style::Color::DarkGray,
            highlight: ratatui::style::Color::LightYellow,
        }
    }
}

impl Default for ConfigFile {
    fn default() -> Self {
        let url = std::env::var("OCI_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
        Self {
            url: Some(url),
            database_url: None,
            email: Some(String::from("floundr_admin")),
            password: Some(String::from("admin")),
            theme: Some(Theme::default()),
            secret: None,
        }
    }
}

impl ConfigFile {
    pub fn load() -> Self {
        let dir = std::env::var("FLOUNDR_HOME").unwrap_or(
            dirs::data_local_dir()
                .expect("unable to find XDG_DATA_HOME, please set FLOUNDR_HOME env variable")
                .join("floundr")
                .to_string_lossy()
                .to_string(),
        );
        let config_path = std::path::Path::new(&dir);
        if !config_path.exists() {
            std::fs::create_dir_all(config_path).expect("unable to create config directory");
            let file = std::fs::File::create(config_path.join("floundr_tui.yml"))
                .expect("unable to create config file");
            let config = Self::default();
            serde_yaml::to_writer(file, &config).expect("unable to write default config");
            config
        } else if let Ok(file) = std::fs::File::open(config_path.join("floundr_tui.yml")) {
            serde_yaml::from_reader(file).expect("unable to parse config file")
        } else {
            let file = std::fs::File::create(config_path.join("floundr_tui.yml"))
                .expect("unable to create config file");
            let config = Self::default();
            serde_yaml::to_writer(file, &config).expect("unable to write default config");
            config
        }
    }
}
