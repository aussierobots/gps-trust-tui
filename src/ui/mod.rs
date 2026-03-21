pub mod layout;
pub mod login_view;
pub mod result_view;
pub mod status_bar;
pub mod task_view;
pub mod tool_browser;
pub mod tool_form;

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, PanelFocus};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let regions = layout::main_layout(area);

    status_bar::render_status_bar(frame, regions.status_bar, app);
    tool_browser::render_tool_list(frame, regions.tool_list, app);
    render_right_panel(frame, regions.detail_panel, app);
    render_progress_bar(frame, regions.progress_bar, app);
    status_bar::render_footer(frame, regions.footer, app);
}

fn render_right_panel(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let has_form = app.form_state.is_some();
    // Only consider result "ready" if it has actual content (not just placeholder)
    let has_result = app
        .result_state
        .as_ref()
        .map(|rs| rs.result.is_some() || rs.error_display.is_some())
        .unwrap_or(false);
    let result_focused = app.focus == PanelFocus::Result;

    if has_form && has_result {
        let chunks = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);
        tool_form::render_tool_form(frame, chunks[0], app.form_state.as_ref().unwrap());
        result_view::render_result_view(
            frame,
            chunks[1],
            app.result_state.as_ref().unwrap(),
            result_focused,
        );
    } else if has_form {
        tool_form::render_tool_form(frame, area, app.form_state.as_ref().unwrap());
    } else if has_result {
        let chunks = Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);
        tool_browser::render_tool_detail(frame, chunks[0], app);
        result_view::render_result_view(
            frame,
            chunks[1],
            app.result_state.as_ref().unwrap(),
            result_focused,
        );
    } else {
        tool_browser::render_tool_detail(frame, area, app);
    }
}

fn render_progress_bar(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let Some(ref task) = app.active_task else {
        let empty = Paragraph::new("").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, area);
        return;
    };

    let mut spans: Vec<Span> = Vec::new();

    spans.push(Span::styled(
        format!(" {} ", task.tool_name),
        Style::default().fg(Color::Yellow),
    ));

    if let Some(progress) = task.progress {
        let total = task.total.unwrap_or(100.0);
        let pct = if total > 0.0 {
            (progress / total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bar_width = (area.width as usize).saturating_sub(30).max(10);
        let filled = (pct * bar_width as f64) as usize;
        let empty_count = bar_width - filled;

        spans.push(Span::raw("["));
        spans.push(Span::styled(
            "\u{2588}".repeat(filled),
            Style::default().fg(Color::Green),
        ));
        spans.push(Span::styled(
            "\u{2591}".repeat(empty_count),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::raw(format!("] {:.0}%", pct * 100.0)));
    } else {
        spans.push(Span::styled("running...", Style::default().fg(Color::Yellow)));
    }

    if let Some(ref msg) = task.message {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(msg.clone(), Style::default().fg(Color::DarkGray)));
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled("x:cancel", Style::default().fg(Color::DarkGray)));

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(bar, area);
}
