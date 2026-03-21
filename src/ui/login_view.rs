use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Authentication status for the login view.
#[derive(Debug, Clone)]
pub enum LoginStatus {
    Authenticating,
    OAuthBrowser,
    Success(String),
    Error(String),
}

/// Render the login status widget into the given area.
pub fn render_login(frame: &mut Frame, area: Rect, status: &LoginStatus) {
    let (text, style) = match status {
        LoginStatus::Authenticating => (
            "Authenticating...".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        LoginStatus::OAuthBrowser => (
            "Opening browser for OAuth login...".to_string(),
            Style::default().fg(Color::Cyan),
        ),
        LoginStatus::Success(name) => (
            format!("Authenticated as {name}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        LoginStatus::Error(msg) => (
            format!("Auth error: {msg}"),
            Style::default().fg(Color::Red),
        ),
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(text, style)))
        .block(Block::default().borders(Borders::ALL).title("Login"));
    frame.render_widget(paragraph, area);
}
