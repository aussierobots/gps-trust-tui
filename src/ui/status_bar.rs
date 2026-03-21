use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ConnectionState, InputMode, PanelFocus};
use crate::mcp::types::ServerIdentity;

/// Render the status bar (top row).
pub fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans: Vec<Span> = Vec::new();

    // App name
    spans.push(Span::styled(
        " gt-ui",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("  "));

    // Identity (entity name, not raw account_id)
    let identity = app.identity_display();
    let identity_style = if app.auth_session.is_some() {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    spans.push(Span::styled(identity, identity_style));
    spans.push(Span::raw("  "));

    // Server connection: [User:ok] [Agent:ok]
    for server in &[ServerIdentity::User, ServerIdentity::Agent] {
        let (label, color) = server_status_label(app, *server);
        spans.push(Span::raw("["));
        spans.push(Span::styled(
            server.label(),
            Style::default().fg(Color::White),
        ));
        spans.push(Span::raw(":"));
        spans.push(Span::styled(label, Style::default().fg(color)));
        spans.push(Span::raw("] "));
    }

    // Tool count
    spans.push(Span::styled(
        format!("{} tools", app.visible_tool_count()),
        Style::default().fg(Color::Gray),
    ));

    // Active task indicator
    if let Some(ref task) = app.active_task {
        spans.push(Span::raw("  "));
        let progress_text = match (task.progress, task.total) {
            (Some(p), Some(t)) => format!("{}: {:.0}/{:.0}", task.tool_name, p, t),
            (Some(p), None) => format!("{}: {:.0}", task.tool_name, p),
            _ => format!("{}...", task.tool_name),
        };
        spans.push(Span::styled(
            progress_text,
            Style::default().fg(Color::Yellow),
        ));
    }

    // Mode badge
    let mode_text = match app.input_mode {
        InputMode::Normal => "",
        InputMode::Filter => " FILTER ",
        InputMode::FormEdit => " EDIT ",
        InputMode::Login => " LOGIN ",
    };
    if !mode_text.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            mode_text,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(bar, area);
}

/// Render the footer bar (bottom row) with context-sensitive key hints.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hints = key_hints(app);
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(hints, Style::default().fg(Color::DarkGray)),
    ]);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(bar, area);
}

fn server_status_label(app: &App, server: ServerIdentity) -> (&'static str, Color) {
    match app.server_state.get(&server) {
        Some(ConnectionState::Connected) => ("ok", Color::Green),
        Some(ConnectionState::Connecting) => ("..", Color::Yellow),
        Some(ConnectionState::Error) => ("err", Color::Red),
        _ => ("--", Color::Gray),
    }
}

fn key_hints(app: &App) -> String {
    match app.input_mode {
        InputMode::Filter => "Type to filter | Enter: accept | Esc: clear".to_string(),
        InputMode::FormEdit => "Type to edit | Enter: done | Esc: cancel".to_string(),
        InputMode::Login => "Authenticating...".to_string(),
        InputMode::Normal => match app.focus {
            PanelFocus::ToolList => {
                "j/k: navigate | Enter: open | /: filter | Tab: next pane | q: quit".to_string()
            }
            PanelFocus::Detail => {
                "Enter: edit params | e: execute | Tab: next pane | Esc: back | q: quit"
                    .to_string()
            }
            PanelFocus::Form => {
                "j/k: navigate | Enter: edit field | Space: toggle | e: execute | Esc: back"
                    .to_string()
            }
            PanelFocus::Result => {
                "j/k: scroll | 1/2: switch tab | Esc: close | Tab: next pane | q: quit".to_string()
            }
        },
    }
}
