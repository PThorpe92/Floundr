use crate::{
    app::{App, Mode, GLOBAL_REPO_LIST},
    screens::InputType,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    prelude::{Frame, Style},
    style::Stylize,
    widgets::Row,
};
use ratatui::{
    style::Color,
    widgets::{Block, Borders, List, ListItem, Paragraph, Table},
};

use super::users::render_header;

pub fn repository_screen(frame: &mut Frame, app: &mut App) {
    render_header(frame, "<-   Home    |    Users   ->");
    let size = frame.size();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(5)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .spacing(1)
        .split(size);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(5)].as_ref())
        .split(chunks[0]);
    let items: Vec<ListItem> = match GLOBAL_REPO_LIST.read() {
        Ok(r) => r
            .repositories
            .iter()
            .enumerate()
            .map(|(i, repo)| {
                let style = if i == app.cursor {
                    Style::default()
                        .fg(ratatui::style::Color::Yellow)
                        .on_light_blue()
                        .italic()
                        .bold()
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(format!("\n{}\n", repo.name)).style(style)
            })
            .collect(),
        Err(_) => vec![],
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title_top("Repositories"),
    );

    frame.render_widget(list, left_chunks[0]);

    let create_repo_text = if app.mode == Mode::Normal {
        "Press 'i' to create a new repository".to_string()
    } else {
        match app.buffer.len() {
            0 => format!("Enter repository name: {}", app.input.value()),
            1 => format!("Public? (y/n): {}", app.input.value()),
            _ => {
                app.normal_mode();
                let _ = app.handle_input(InputType::NewRepo);
                String::from("Repo created")
            }
        }
    };

    let create_repo = Paragraph::new(create_repo_text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(Block::default().borders(Borders::ALL));

    frame.render_widget(create_repo, left_chunks[1]);
    if let Ok(rep) = GLOBAL_REPO_LIST.read() {
        if let Some(repo) = rep.repositories.get(app.cursor) {
            render_repo_details(frame, chunks.to_vec(), repo);
        }
    }
}

fn render_repo_details(
    frame: &mut Frame,
    chunks: Vec<ratatui::layout::Rect>,
    repo: &crate::app::Repo,
) {
    let details_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(0)].as_ref())
        .split(chunks[1]);

    let text = format!(
        "Name: {}\nPublic: {}\nFile Path: {}\nDisk Usage: ~{}MB\nTotal Layers: {}\nDriver: {}",
        repo.name,
        repo.is_public,
        repo.file_path,
        repo.calculate_mb().round(),
        repo.num_layers,
        repo.driver,
    );

    let basic_info = Paragraph::new(text)
        .style(Style::default().bg(Color::Black).fg(Color::Yellow))
        .block(
            Block::default()
                .title("Basic Information")
                .borders(Borders::ALL),
        );

    frame.render_widget(basic_info, details_chunks[0]);

    let stats_table = Table::new(
        vec![
            Row::new(vec!["Total Blobs".to_string(), repo.blob_count.to_string()]),
            Row::new(vec!["Total Tags".to_string(), repo.tag_count.to_string()]),
            Row::new(vec![
                "Manifests".to_string(),
                repo.manifest_count.to_string(),
            ]),
        ],
        &[Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(Block::default().title("Statistics").borders(Borders::ALL));

    frame.render_widget(stats_table, details_chunks[1]);

    let tags_list: Vec<ListItem> = repo
        .tags
        .iter()
        .map(|tag| ListItem::new(tag.clone()))
        .collect();
    let tags = List::new(tags_list).block(Block::default().title("Tags").borders(Borders::ALL));

    let bottom_half_details = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(details_chunks[1]);
    frame.render_widget(tags, bottom_half_details[1]);
}

pub fn home_screen(frame: &mut Frame, app: &mut App) {
    render_header(frame, "|   Repositories  ->");
    let size = frame.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Percentage(8),
                Constraint::Percentage(12),
                Constraint::Percentage(70),
            ]
            .as_ref(),
        )
        .split(size);
    let items = app.get_items();
    let selected_style = Style::default()
        .bg(ratatui::style::Color::Black)
        .fg(ratatui::style::Color::Yellow);
    let normal_style = Style::default()
        .bg(ratatui::style::Color::Black)
        .fg(ratatui::style::Color::White);
    let items = items.into_iter().enumerate().map(|(i, item)| {
        let style = if Some(i) == app.state.selected() {
            selected_style
        } else {
            normal_style
        };
        item.style(style).to_owned()
    });
    frame.render_widget(
        ratatui::widgets::Block::bordered()
            .border_type(ratatui::widgets::BorderType::Thick)
            .title("Floundr OCI Distrobution Registry")
            .title_style(Style::default().fg(Color::Yellow)),
        chunks[2],
    );
    frame.render_widget(
        ratatui::widgets::List::new(items).block(
            ratatui::widgets::Block::default()
                .title("Home")
                .borders(ratatui::widgets::Borders::ALL),
        ),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(ASCII_ART)
            .style(Style::default().fg(Color::LightYellow))
            .alignment(ratatui::layout::Alignment::Center),
        chunks[2].inner(Margin {
            horizontal: match chunks[2].width {
                5..=20 => 2,
                21..=40 => 4,
                41..=60 => 6,
                61..=80 => 8,
                _ => 10,
            },
            vertical: 5,
        }),
    );
}
#[rustfmt::skip]
pub static ASCII_ART: &str = 
r#" 

  █████▒██▓     ▒█████   █    ██  ███▄    █ ▓█████▄  ██▀███  
▓██   ▒▓██▒    ▒██▒  ██▒ ██  ▓██▒ ██ ▀█   █ ▒██▀ ██▌▓██ ▒ ██▒
▒████ ░▒██░    ▒██░  ██▒▓██  ▒██░▓██  ▀█ ██▒░██   █▌▓██ ░▄█ ▒
░▓█▒  ░▒██░    ▒██   ██░▓▓█  ░██░▓██▒  ▐▌██▒░▓█▄   ▌▒██▀▀█▄  
░▒█░   ░██████▒░ ████▓▒░▒▒█████▓ ▒██░   ▓██░░▒████▓ ░██▓ ▒██▒
 ▒ ░   ░ ▒░▓  ░░ ▒░▒░▒░ ░▒▓▒ ▒ ▒ ░ ▒░   ▒ ▒  ▒▒▓  ▒ ░ ▒▓ ░▒▓░
 ░     ░ ░ ▒  ░  ░ ▒ ▒░ ░░▒░ ░ ░ ░ ░░   ░ ▒░ ░ ▒  ▒   ░▒ ░ ▒░
 ░ ░     ░ ░   ░ ░ ░ ▒   ░░░ ░ ░    ░   ░ ░  ░ ░  ░   ░░   ░ 
           ░  ░    ░ ░     ░              ░    ░       ░     
                                             ░"#;
