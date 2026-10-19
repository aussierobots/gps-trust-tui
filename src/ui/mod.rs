pub mod layout;
pub mod login_view;
pub mod result_view;
pub mod status_bar;
pub mod tool_browser;
pub mod tool_form;

use ratatui::layout::{Constraint, Layout};
use ratatui::Frame;

use crate::app::{App, PanelFocus};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let regions = layout::main_layout(area);

    status_bar::render_status_bar(frame, regions.status_bar, app);
    tool_browser::render_tool_list(frame, regions.tool_list, app);
    render_right_panel(frame, regions.detail_panel, app);
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
        tool_form::render_tool_form(
            frame,
            chunks[0],
            app.form_state.as_ref().unwrap(),
            &app.id_cache,
        );
        result_view::render_result_view(
            frame,
            chunks[1],
            app.result_state.as_ref().unwrap(),
            result_focused,
        );
    } else if has_form {
        tool_form::render_tool_form(frame, area, app.form_state.as_ref().unwrap(), &app.id_cache);
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
