use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
use ratatui::Frame;
use turul_mcp_protocol::tasks::TaskStatus;

use crate::mcp::types::ActiveTask;

/// Render the task status view into the given area.
///
/// For working tasks this shows a progress bar and status message.
/// For terminal states it shows the final status with appropriate styling.
pub fn render_task_view(frame: &mut Frame, area: Rect, task: &ActiveTask) {
    let status = task.task.status;

    // Split area: header (3 lines), gauge (3 lines), rest for detail.
    let chunks = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(3), // header block
        ratatui::layout::Constraint::Length(3), // progress gauge
        ratatui::layout::Constraint::Min(0),    // detail / hint
    ])
    .split(area);

    // -- Header --
    let (status_label, status_color) = status_display(status);
    let header_lines = vec![Line::from(vec![
        Span::styled(
            format!(" Task: {} ", task.task.task_id),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Status: "),
        Span::styled(status_label, Style::default().fg(status_color)),
    ])];

    let header = Paragraph::new(header_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Task ")
            .border_style(Style::default().fg(status_color)),
    );
    frame.render_widget(header, chunks[0]);

    // -- Progress gauge (only meaningful while Working) --
    match status {
        TaskStatus::Working => {
            let ratio = match (task.progress, task.total) {
                (Some(p), Some(t)) if t > 0.0 => (p / t).clamp(0.0, 1.0),
                _ => 0.0,
            };
            let label = task
                .message
                .as_deref()
                .unwrap_or("Working...");

            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .ratio(ratio)
                .label(label.to_string());

            frame.render_widget(gauge, chunks[1]);

            // Hint: cancel
            let hint = Paragraph::new(vec![Line::from(vec![Span::styled(
                "  [x] Cancel task",
                Style::default().fg(Color::DarkGray),
            )])])
            .wrap(Wrap { trim: false });
            frame.render_widget(hint, chunks[2]);
        }
        TaskStatus::Completed => {
            let detail = Paragraph::new(vec![
                Line::from(Span::styled(
                    "  Task completed successfully.",
                    Style::default().fg(Color::Green),
                )),
                status_message_line(&task.task.status_message),
            ])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
            frame.render_widget(detail, chunks[1]);
        }
        TaskStatus::Failed => {
            let msg = task
                .task
                .status_message
                .as_deref()
                .unwrap_or("(no details)");
            let detail = Paragraph::new(vec![Line::from(Span::styled(
                format!("  Task failed: {}", msg),
                Style::default().fg(Color::Red),
            ))])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
            frame.render_widget(detail, chunks[1]);
        }
        TaskStatus::Cancelled => {
            let detail = Paragraph::new(vec![
                Line::from(Span::styled(
                    "  Task was cancelled.",
                    Style::default().fg(Color::Yellow),
                )),
                status_message_line(&task.task.status_message),
            ])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
            frame.render_widget(detail, chunks[1]);
        }
        TaskStatus::InputRequired => {
            let detail = Paragraph::new(vec![Line::from(Span::styled(
                "  Input required -- not supported in TUI v1",
                Style::default().fg(Color::Yellow),
            ))])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
            frame.render_widget(detail, chunks[1]);
        }
    }
}

/// Map a `TaskStatus` to a display label and colour.
fn status_display(status: TaskStatus) -> (&'static str, Color) {
    match status {
        TaskStatus::Working => ("Working", Color::Cyan),
        TaskStatus::Completed => ("Completed", Color::Green),
        TaskStatus::Failed => ("Failed", Color::Red),
        TaskStatus::Cancelled => ("Cancelled", Color::Yellow),
        TaskStatus::InputRequired => ("Input Required", Color::Yellow),
    }
}

/// Build an optional status-message line in dim style.
fn status_message_line(message: &Option<String>) -> Line<'static> {
    match message {
        Some(msg) => Line::from(Span::styled(
            format!("  {}", msg),
            Style::default().fg(Color::DarkGray),
        )),
        None => Line::from(""),
    }
}
