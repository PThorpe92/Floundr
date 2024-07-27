pub mod repos;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenType {
    Home,
    Repos,
    Images,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputType {
    NewRepo,
}
