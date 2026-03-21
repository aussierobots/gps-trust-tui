use ratatui::layout::{Constraint, Layout, Rect};

/// Named regions for the main application layout.
pub struct MainLayout {
    pub status_bar: Rect,
    pub tool_list: Rect,
    pub detail_panel: Rect,
    pub progress_bar: Rect,
    pub footer: Rect,
}

/// Build the main application layout from the terminal area.
///
/// ```text
/// +--------------------------------------------------------------+
/// | Status Bar                                                    |
/// +----------------------+---------------------------------------+
/// |  Tool List (35%)     |  Detail / Form / Result (65%)          |
/// |                      |                                        |
/// +----------------------+---------------------------------------+
/// | Progress bar (conditional, 1 row)                             |
/// | Footer (key hints, 1 row)                                     |
/// +--------------------------------------------------------------+
/// ```
pub fn main_layout(area: Rect) -> MainLayout {
    let vertical = Layout::vertical([
        Constraint::Length(1), // status bar
        Constraint::Min(0),   // main content
        Constraint::Length(1), // progress bar
        Constraint::Length(1), // footer
    ])
    .split(area);

    let horizontal = Layout::horizontal([
        Constraint::Percentage(35), // tool list
        Constraint::Percentage(65), // detail panel
    ])
    .split(vertical[1]);

    MainLayout {
        status_bar: vertical[0],
        tool_list: horizontal[0],
        detail_panel: horizontal[1],
        progress_bar: vertical[2],
        footer: vertical[3],
    }
}
