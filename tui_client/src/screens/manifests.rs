use crate::app::{App, MANIFESTS};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::users::render_header;

pub fn render_manifest_screen(frame: &mut Frame, manifest: &str, rect: &ratatui::layout::Rect) {
    render_header(frame, "Manifests");
    let pretty_json = serde_json::to_string_pretty(manifest).unwrap_or_else(|_| "{}".to_string());
    let json_paragraph = Paragraph::new(pretty_json)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Manifest Viewer")
                .title_alignment(ratatui::layout::Alignment::Center)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .alignment(ratatui::layout::Alignment::Center);

    frame.render_widget(json_paragraph, *rect);
}
