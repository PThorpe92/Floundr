use crate::{
    app::{App, Mode},
    screens::InputType,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{Frame, Style},
    style::{Color, Modifier, Stylize},
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table},
};

pub fn render_header(frame: &mut Frame, header: &str) {
    let size = frame.size();
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
    let size = frame.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Percentage(8),
                Constraint::Percentage(22),
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
    let menu_items = vec![
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
        ListItem::new("View Logs").style(if app.cursor == 2 {
            selected_style
        } else {
            Style::default().fg(Color::White)
        }),
    ];

    let menu_list = List::new(menu_items).block(
        Block::default()
            .borders(Borders::ALL)
            .fg(Color::Yellow)
            .bold()
            .border_style(Style::new().add_modifier(Modifier::REVERSED))
            .title("User Management Options")
            .style(Style::default().bg(*app.get_bg())),
    );

    frame.render_widget(menu_list, chunks[1]);

    let selected_option_text = match app.cursor {
        0 => "Create a new API Key",
        1 => "Manage existing users",
        2 => "View system logs",
        _ => "",
    };

    let selected_option_paragraph = Paragraph::new(selected_option_text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Option"),
        );

    frame.render_widget(selected_option_paragraph, chunks[2]);

    match app.cursor {
        0 => render_create_api_key(frame, app, chunks[2]),
        1 => render_manage_users(frame, app, chunks[2]),
        2 => render_view_logs(frame, app, chunks[2]),
        _ => {}
    }
}

fn render_create_api_key(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let create_api_key_text = if app.mode == Mode::Normal {
        "Press 'i' to create a new API key".to_string()
    } else {
        match app.buffer.len() {
            0 => format!("Enter API key name: {}", app.input.value()),
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
    let user_list_items = vec![
        ListItem::new("User 1"),
        ListItem::new("User 2"),
        ListItem::new("User 3"),
    ];

    let user_list = List::new(user_list_items)
        .block(Block::default().borders(Borders::ALL).title("Manage Users"));

    frame.render_widget(user_list, area);
}

fn render_view_logs(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    // Implement the view logs UI
    let logs = vec![
        ListItem::new("Log entry 1"),
        ListItem::new("Log entry 2"),
        ListItem::new("Log entry 3"),
    ];

    let log_list =
        List::new(logs).block(Block::default().borders(Borders::ALL).title("System Logs"));

    frame.render_widget(log_list, area);
}
