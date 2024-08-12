use crate::app::GLOBAL_REPO_LIST;

pub mod manifests;
pub mod repos;
pub mod users;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenType {
    Home,
    Users,
    Repos,
}

impl ScreenType {
    pub fn get_cursor_len(&self) -> usize {
        match self {
            Self::Home => 0,
            Self::Repos => GLOBAL_REPO_LIST.read().unwrap().repositories.len(),
            Self::Users => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputType {
    NewRepo,
    CreateApiKey,
    NewUser,
    DeleteUser,
    DeleteRepo,
}
