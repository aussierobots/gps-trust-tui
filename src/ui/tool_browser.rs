use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, InputMode, PanelFocus};
use turul_mcp_protocol::JsonSchema;

/// Render the tool list in the left panel.
pub fn render_tool_list(frame: &mut Frame, area: Rect, app: &mut App) {
    // Split area: main list + optional filter bar at bottom
    let show_filter = app.input_mode == InputMode::Filter || !app.filter_text.is_empty();
    let chunks = if show_filter {
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area)
    } else {
        Layout::vertical([Constraint::Min(0)]).split(area)
    };

    let filtered_count = app.filtered_indices.len();
    let total_count = app.tools.len();

    let title = if app.filter_text.is_empty() {
        format!("Tools ({})", total_count)
    } else {
        format!("Tools ({}/{})", filtered_count, total_count)
    };

    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let entry = &app.tools[idx];
            let prefix = format!("[{}]", entry.server.prefix());
            let badge = entry.task_badge();

            let mut spans = vec![
                Span::styled(prefix, Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::raw(entry.display_name()),
            ];
            if !badge.is_empty() {
                let badge_color = if badge.contains('!') {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
                spans.push(Span::raw(" "));
                spans.push(Span::styled(badge, Style::default().fg(badge_color)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let focused = app.focus == PanelFocus::ToolList;
    let border_color = if focused { Color::Cyan } else { Color::DarkGray };

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[0], &mut app.tool_list_state);

    // Render filter bar
    if show_filter {
        let filter_style = if app.input_mode == InputMode::Filter {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let filter_text = format!(" /{}_ ", app.filter_text);
        let filter_bar = Paragraph::new(filter_text).style(filter_style);
        frame.render_widget(filter_bar, chunks[1]);
    }
}

/// Render the detail pane for the currently selected tool.
pub fn render_tool_detail(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == PanelFocus::Detail;
    let border_color = if focused { Color::Cyan } else { Color::DarkGray };

    let Some(entry) = app.selected_tool() else {
        let empty = Paragraph::new("No tool selected")
            .block(
                Block::default()
                    .title("Detail")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, area);
        return;
    };

    let tool = &entry.tool;
    let mut lines: Vec<Line> = Vec::new();

    // Title — annotation title if present, else tool name
    let display_name = entry.display_name();
    lines.push(Line::from(vec![
        Span::styled(
            display_name,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{}]", entry.server.prefix()),
            Style::default().fg(Color::Cyan),
        ),
    ]));
    // Show the snake_case tool name when it differs from the display name
    if display_name != tool.name {
        lines.push(Line::from(Span::styled(
            &tool.name,
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Task badge
    let badge = entry.task_badge();
    if !badge.is_empty() {
        let label = match badge {
            "[T!]" => "Task: required",
            "[T?]" => "Task: optional",
            _ => badge,
        };
        lines.push(Line::from(Span::styled(
            label,
            Style::default().fg(Color::Yellow),
        )));
    }

    // Annotations hints
    if let Some(ann) = &tool.annotations {
        let mut hints = Vec::new();
        if ann.read_only_hint == Some(true) {
            hints.push("read-only");
        }
        if ann.destructive_hint == Some(true) {
            hints.push("destructive");
        }
        if ann.idempotent_hint == Some(true) {
            hints.push("idempotent");
        }
        if ann.open_world_hint == Some(true) {
            hints.push("open-world");
        }
        if !hints.is_empty() {
            lines.push(Line::from(Span::styled(
                hints.join(" | "),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));

    // Description
    if let Some(desc) = &tool.description {
        for paragraph in desc.split('\n') {
            lines.push(Line::from(Span::raw(paragraph)));
        }
        lines.push(Line::from(""));
    }

    // Parameters
    let required_set: Vec<&str> = tool
        .input_schema
        .required
        .as_ref()
        .map(|r| r.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if let Some(props) = &tool.input_schema.properties {
        lines.push(Line::from(Span::styled(
            "Parameters",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        lines.push(Line::from(""));

        // Sort property names for stable display
        let mut prop_names: Vec<&String> = props.keys().collect();
        prop_names.sort();

        for name in prop_names {
            let schema = &props[name];
            let is_required = required_set.contains(&name.as_str());
            let type_name = schema_type_name(schema);

            let is_managed = name == "account_id";

            let mut spans = vec![
                Span::styled(
                    format!("  {name}"),
                    Style::default().fg(Color::Green),
                ),
            ];
            if is_required {
                spans.push(Span::styled("*", Style::default().fg(Color::Red)));
            }
            spans.push(Span::styled(
                format!(" : {type_name}"),
                Style::default().fg(Color::DarkGray),
            ));
            if is_managed {
                spans.push(Span::styled(
                    "  (from session)",
                    Style::default().fg(Color::Yellow),
                ));
            }
            lines.push(Line::from(spans));

            // Property description
            if let Some(desc) = schema_description(schema) {
                lines.push(Line::from(Span::styled(
                    format!("    {desc}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .title(entry.display_name())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(detail, area);
}

/// Extract a display type name from a JsonSchema variant.
fn schema_type_name(schema: &JsonSchema) -> &'static str {
    match schema {
        JsonSchema::String { .. } => "string",
        JsonSchema::Number { .. } => "number",
        JsonSchema::Integer { .. } => "integer",
        JsonSchema::Boolean { .. } => "boolean",
        JsonSchema::Array { .. } => "array",
        JsonSchema::Object { .. } => "object",
    }
}

/// Extract description from a JsonSchema variant.
fn schema_description(schema: &JsonSchema) -> Option<&str> {
    match schema {
        JsonSchema::String { description, .. }
        | JsonSchema::Number { description, .. }
        | JsonSchema::Integer { description, .. }
        | JsonSchema::Boolean { description, .. }
        | JsonSchema::Array { description, .. }
        | JsonSchema::Object { description, .. } => description.as_deref(),
    }
}
