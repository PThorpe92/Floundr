pub mod repos;
pub mod users;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenType {
    Home,
    Users,
    Repos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputType {
    NewRepo,
    CreateApiKey,
}
