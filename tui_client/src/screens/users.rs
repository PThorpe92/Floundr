use crate::{
    app::{get_items, App, Mode, ACTIVE_KEYS, USERS},
    screens::InputType,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    prelude::{Frame, Style},
    style::{Color, Modifier, Stylize},
    widgets::{
        Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, StatefulWidget,
    },
};
use shared::UserResponse;

pub fn render_header(frame: &mut Frame, header: &str) {
    let size = frame.area();
    let header_chunk = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
        .split(size);

    let header = Paragraph::new(header)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title("Navigation"));

    frame.render_widget(header, header_chunk[0]);
}

pub fn user_management_screen(frame: &mut Frame, app: &mut App) {
    render_header(frame, "<-  Repositories |");
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Percentage(2),
                Constraint::Percentage(8),
                Constraint::Percentage(20),
                Constraint::Percentage(70),
            ]
            .as_ref(),
        )
        .spacing(1)
        .split(size);
    let selected_style = match app.cursor {
        0 => Style::default().fg(Color::Yellow).on_light_blue(),
        1 => Style::default().fg(Color::Yellow).on_light_blue(),
        2 => Style::default().fg(Color::Yellow).on_light_blue(),
        _ => Style::default().fg(Color::White),
    };

    let menu_items = match app.state.selected() {
        None => {
            vec![
                ListItem::new("Create API Key").style(if app.cursor == 0 {
                    selected_style
                } else {
                    Style::default().fg(Color::White)
                }),
                ListItem::new("Manage Users").style(if app.cursor == 1 {
                    selected_style
                } else {
                    Style::default().fg(Color::White)
                }),
                ListItem::new("View Active API Keyss").style(if app.cursor == 2 {
                    selected_style
                } else {
                    Style::default().fg(Color::White)
                }),
            ]
        }
        Some(cursor) => match cursor {
            0 => vec![
                ListItem::new("API keys hold all scopes and permissions\n"),
                ListItem::new("It is recommended to cycle them regularly\nAnd use bearer tokens when possible"),
                ListItem::new("Press 'esc' return"),
            ],
            1 => vec![
                ListItem::new("Press 'i' to create a new user"),
                ListItem::new("Press 'd' to delete a user"),
                ListItem::new("Press 'a' to add scope"),
                ListItem::new("Press 'esc' return"),
            ],
            _ => vec![
                ListItem::new("Press 'd' to delete a key"),
                ListItem::new("Press 'esc' to return"),
            ],
        },
    };

    let menu_list = List::new(menu_items).block(
        Block::default()
            .borders(Borders::ALL)
            .fg(Color::Yellow)
            .bold()
            .border_style(Style::new().add_modifier(match app.mode {
                Mode::Normal => Modifier::REVERSED,
                Mode::Insert => Modifier::DIM,
            }))
            .title("User Management Options")
            .style(Style::default().bg(*app.get_bg())),
    );
    frame.render_widget(menu_list, chunks[2]);
    let selected_option_text = match app.state.selected() {
        Some(cursor) => match cursor {
            0 => "Create a new API Key",
            1 => "Manage existing users",
            2 => "Manage Active Keys",
            _ => "",
        },
        None => "'j' / 'k' to navigate | 'enter' to select option | 'esc' to go back",
    };

    let selected_option_paragraph = Paragraph::new(selected_option_text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Option"),
        );

    frame.render_widget(selected_option_paragraph, chunks[3]);
    match app.state.selected() {
        Some(0) => render_create_api_key(frame, app, chunks[1]),
        Some(1) => {
            let prompt = match app.current_action {
                None => match app.mode {
                    Mode::Normal => String::default(),
                    Mode::Insert => match app.buffer.len() {
                        0 => format!("Enter new user email: {}", app.input.value()),
                        1 => format!(
                            "Enter new user password: {}",
                            "*".repeat(app.input.value().len())
                        ),
                        2 => format!(
                            "Confirm new user password: {}",
                            "*".repeat(app.input.value().len())
                        ),
                        3 => format!("Is user an admin? (y/n): {}", app.input.value()),
                        _ => {
                            app.normal_mode();
                            let _ = app.handle_input(InputType::NewUser);
                            String::from("User created")
                        }
                    },
                },
                Some(_) => match app.mode {
                    Mode::Normal => String::default(),
                    Mode::Insert => match app.buffer.len() {
                        0 => format!("Enter user email to delete: {}", app.input.value()),
                        _ => {
                            app.normal_mode();
                            let _ = app.handle_input(InputType::DeleteUser);
                            String::from("User deleted")
                        }
                    },
                },
            };
            render_input_box(frame, chunks[1], prompt);
            render_manage_users(frame, app, chunks[3]);
        }
        Some(2) => {
            let items = get_items(app.current_screen, app.state.selected()).clone();
            render_active_keys(frame, items, app, chunks[3]);
            if app.current_action.is_some() {
                // deleting key
                let prompt = match app.mode {
                    Mode::Normal => String::default(),
                    Mode::Insert => match app.buffer.len() {
                        0 => format!("Enter # key to delete: {}", app.input.value()),
                        _ => {
                            app.normal_mode();
                            let _ = app.handle_input(InputType::DeleteUser);
                            String::from("Key deleted")
                        }
                    },
                };
                render_input_box(frame, chunks[1], prompt);
            }
        }
        _ => {}
    }
}

fn render_active_keys(
    frame: &mut Frame,
    items: Vec<ListItem>,
    app: &mut App,
    area: ratatui::layout::Rect,
) {
    if get_items(app.current_screen, app.state.selected()).is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Active Keys | 'j' / 'k' to navigate")
        .style(Style::default().fg(Color::LightYellow));
    let widget = List::new(items)
        .block(block)
        .style(Style::default().fg(Color::LightYellow));
    let scroll = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .style(
            Style::default()
                .fg(Color::LightYellow)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .track_symbol(Some("▓"))
        .thumb_symbol("▒")
        .begin_symbol(Some("*"))
        .end_symbol(Some("*"));
    widget.render(
        area.inner(Margin::new(1, 1)),
        frame.buffer_mut(),
        &mut app.state,
    );
    scroll.render(area, frame.buffer_mut(), &mut app.scrollbar);
}

fn render_create_api_key(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let create_api_key_text = if app.mode == Mode::Normal {
        "Press 'i' to create a new API key".to_string()
    } else {
        match app.buffer.len() {
            0 => format!("Enter email address of user:\n {}", app.input.value()),
            1 => {
                app.normal_mode();
                let _ = app.handle_input(InputType::CreateApiKey);
                String::from("API key created")
            }
            _ => String::new(),
        }
    };
    let create_api_key_paragraph = Paragraph::new(create_api_key_text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create API Key"),
        );

    frame.render_widget(create_api_key_paragraph, area);
}

fn render_manage_users(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    // Implement the user management UI
    let user_list = USERS.read().expect("Failed to read users");
    let pretty_str = serde_json::to_string_pretty::<UserResponse>(
        user_list
            .get(app.cursor)
            .or_else(|| user_list.first())
            .unwrap_or(&UserResponse::default()),
    )
    .unwrap_or_default();
    let user_list = Paragraph::new(pretty_str).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Manage Users | 'j' / 'k' to navigate")
            .style(Style::default().fg(Color::LightYellow)),
    );
    frame.render_widget(user_list, area.inner(ratatui::layout::Margin::new(1, 1)));
}

pub fn render_input_box(frame: &mut Frame, area: ratatui::layout::Rect, prompt: String) {
    let input_box = Paragraph::new(prompt)
        .style(Style::default().bg(Color::DarkGray).fg(Color::LightYellow))
        .block(Block::default().borders(Borders::ALL).bold().title("Input"));
    frame.render_widget(input_box, area);
}
