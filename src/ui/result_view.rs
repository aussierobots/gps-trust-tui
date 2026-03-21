use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use turul_mcp_protocol::content::ContentBlock;
use turul_mcp_protocol::content::ResourceContents;
use turul_mcp_protocol::tools::CallToolResult;



/// Which tab is active in the result view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultTab {
    /// Friendly key-value rendering.
    #[default]
    Structured,
    /// Raw JSON / content blocks.
    Raw,
}

/// Tracks scroll state and content for the result view.
#[derive(Debug, Clone, Default)]
pub struct ResultState {
    pub result: Option<CallToolResult>,
    pub scroll_offset: u16,
    pub error_display: Option<String>,
    pub active_tab: ResultTab,
    pub tool_name: Option<String>,
}

impl ResultState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_result(result: CallToolResult) -> Self {
        Self {
            result: Some(result),
            scroll_offset: 0,
            error_display: None,
            active_tab: ResultTab::Structured,
            tool_name: None,
        }
    }

    pub fn with_error(message: String) -> Self {
        Self {
            result: None,
            scroll_offset: 0,
            error_display: Some(message),
            active_tab: ResultTab::Structured,
            tool_name: None,
        }
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    pub fn next_tab(&mut self) {
        self.active_tab = match self.active_tab {
            ResultTab::Structured => ResultTab::Raw,
            ResultTab::Raw => ResultTab::Structured,
        };
        self.scroll_offset = 0;
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the result view into the given area.
pub fn render_result_view(
    frame: &mut Frame,
    area: Rect,
    state: &ResultState,
    focused: bool,
) {
    let is_error = state
        .result
        .as_ref()
        .and_then(|r| r.is_error)
        .unwrap_or(false)
        || state.error_display.is_some();

    let border_color = if is_error {
        Color::Red
    } else if focused {
        Color::Cyan
    } else {
        Color::Green
    };

    // Tab bar + content split
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Tab bar
    let tab1_style = if state.active_tab == ResultTab::Structured {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let tab2_style = if state.active_tab == ResultTab::Raw {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_line = Line::from(vec![
        Span::styled(" [1] Structured ", tab1_style),
        Span::raw("  "),
        Span::styled(" [2] Raw ", tab2_style),
    ]);
    frame.render_widget(Paragraph::new(tab_line), chunks[0]);

    // Content
    let lines = match state.active_tab {
        ResultTab::Structured => build_structured_lines(state),
        ResultTab::Raw => build_raw_lines(state),
    };

    let tool_label = state.tool_name.as_deref().unwrap_or("Result");
    let block_title = if is_error {
        format!("{} (error)", tool_label)
    } else {
        tool_label.to_string()
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(block_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset, 0));

    frame.render_widget(paragraph, chunks[1]);
}

// ---------------------------------------------------------------------------
// Structured tab — friendly key-value rendering
// ---------------------------------------------------------------------------

fn build_structured_lines(state: &ResultState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(ref err) = state.error_display {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
        return lines;
    }

    let result = match &state.result {
        Some(r) => r,
        None => return lines,
    };

    // Try structured_content first, then fall back to content blocks
    if let Some(ref structured) = result.structured_content {
        render_value_friendly(structured, &mut lines, 0);
    } else {
        // Parse text content blocks as JSON for friendly rendering
        for block in &result.content {
            match block {
                ContentBlock::Text { text, .. } => {
                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(text) {
                        render_value_friendly(&json_val, &mut lines, 0);
                    } else {
                        for line in text.lines() {
                            lines.push(Line::from(line.to_string()));
                        }
                    }
                }
                _ => render_content_block(block, &mut lines, Style::default()),
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from("(empty result)"));
    }

    lines
}

/// Render a JSON value in a human-friendly format with indentation.
fn render_value_friendly(value: &serde_json::Value, lines: &mut Vec<Line<'static>>, depth: usize) {
    let indent = "  ".repeat(depth);

    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                match val {
                    serde_json::Value::Object(_) => {
                        lines.push(Line::from(vec![
                            Span::raw(indent.clone()),
                            Span::styled(
                                key.clone(),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        render_value_friendly(val, lines, depth + 1);
                    }
                    serde_json::Value::Array(arr) => {
                        lines.push(Line::from(vec![
                            Span::raw(indent.clone()),
                            Span::styled(
                                format!("{key}  "),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("({} items)", arr.len()),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));
                        for (i, item) in arr.iter().enumerate() {
                            // Separator between array items
                            if i > 0 {
                                lines.push(Line::from(Span::styled(
                                    format!("{}  ───", indent),
                                    Style::default().fg(Color::DarkGray),
                                )));
                            }
                            render_value_friendly(item, lines, depth + 1);
                        }
                    }
                    _ => {
                        lines.push(Line::from(vec![
                            Span::raw(indent.clone()),
                            Span::styled(
                                format!("{key}  "),
                                Style::default().fg(Color::DarkGray),
                            ),
                            format_scalar(val),
                        ]));
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    lines.push(Line::from(Span::styled(
                        format!("{}───", indent),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                render_value_friendly(item, lines, depth);
            }
        }
        _ => {
            lines.push(Line::from(vec![
                Span::raw(indent),
                format_scalar(value),
            ]));
        }
    }
}

/// Format a scalar JSON value with appropriate coloring.
fn format_scalar(value: &serde_json::Value) -> Span<'static> {
    match value {
        serde_json::Value::String(s) => Span::styled(s.clone(), Style::default().fg(Color::White)),
        serde_json::Value::Number(n) => {
            Span::styled(n.to_string(), Style::default().fg(Color::Yellow))
        }
        serde_json::Value::Bool(b) => Span::styled(
            b.to_string(),
            Style::default().fg(if *b { Color::Green } else { Color::Red }),
        ),
        serde_json::Value::Null => Span::styled("null", Style::default().fg(Color::DarkGray)),
        _ => Span::raw(value.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Raw tab — JSON pretty-print + content blocks
// ---------------------------------------------------------------------------

fn build_raw_lines(state: &ResultState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(ref err) = state.error_display {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
        return lines;
    }

    let result = match &state.result {
        Some(r) => r,
        None => return lines,
    };

    let is_error = result.is_error.unwrap_or(false);
    let text_style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };

    if let Some(ref structured) = result.structured_content {
        lines.push(Line::from(Span::styled(
            "── structured ──",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )));
        let pretty = serde_json::to_string_pretty(structured)
            .unwrap_or_else(|_| structured.to_string());
        for line in pretty.lines() {
            lines.push(Line::from(Span::styled(line.to_string(), text_style)));
        }
        lines.push(Line::from(""));
    }

    for (i, block) in result.content.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        render_content_block(block, &mut lines, text_style);
    }

    if lines.is_empty() {
        lines.push(Line::from("(empty result)"));
    }

    lines
}

/// Render a single `ContentBlock` into lines.
fn render_content_block(block: &ContentBlock, lines: &mut Vec<Line<'static>>, style: Style) {
    match block {
        ContentBlock::Text { text, .. } => {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(text) {
                let pretty = serde_json::to_string_pretty(&json_val)
                    .unwrap_or_else(|_| text.clone());
                for line in pretty.lines() {
                    lines.push(Line::from(Span::styled(line.to_string(), style)));
                }
            } else {
                for line in text.lines() {
                    lines.push(Line::from(Span::styled(line.to_string(), style)));
                }
            }
        }
        ContentBlock::Image {
            mime_type, data, ..
        } => {
            lines.push(Line::from(Span::styled(
                format!("[Image: {}, {} bytes]", mime_type, data.len()),
                Style::default().fg(Color::Blue),
            )));
        }
        ContentBlock::Audio {
            mime_type, data, ..
        } => {
            lines.push(Line::from(Span::styled(
                format!("[Audio: {}, {} bytes]", mime_type, data.len()),
                Style::default().fg(Color::Blue),
            )));
        }
        ContentBlock::ResourceLink { resource, .. } => {
            let name_part = if resource.name.is_empty() {
                String::new()
            } else {
                format!(" ({})", resource.name)
            };
            lines.push(Line::from(Span::styled(
                format!("-> {}{}", resource.uri, name_part),
                Style::default().fg(Color::Cyan),
            )));
        }
        ContentBlock::Resource { resource, .. } => match resource {
            ResourceContents::Text(text_res) => {
                lines.push(Line::from(Span::styled(
                    format!("[Resource: {}]", text_res.uri),
                    Style::default().fg(Color::Cyan),
                )));
                for line in text_res.text.lines() {
                    lines.push(Line::from(Span::styled(line.to_string(), style)));
                }
            }
            ResourceContents::Blob(blob_res) => {
                lines.push(Line::from(Span::styled(
                    format!("[Blob: {}, {} bytes]", blob_res.uri, blob_res.blob.len()),
                    Style::default().fg(Color::Blue),
                )));
            }
        },
        ContentBlock::ToolUse { id, name, .. } => {
            lines.push(Line::from(Span::styled(
                format!("[ToolUse: {} ({})]", name, id),
                Style::default().fg(Color::Yellow),
            )));
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => {
            let err_flag = if *is_error == Some(true) {
                " (error)"
            } else {
                ""
            };
            lines.push(Line::from(Span::styled(
                format!("[ToolResult: {}{}]", tool_use_id, err_flag),
                Style::default().fg(Color::Yellow),
            )));
            let inner_style = if *is_error == Some(true) {
                Style::default().fg(Color::Red)
            } else {
                style
            };
            for inner_block in content {
                render_content_block(inner_block, lines, inner_style);
            }
        }
    }
}
