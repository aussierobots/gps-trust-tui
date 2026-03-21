use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::Value;
use turul_mcp_protocol::schema::JsonSchema;
use turul_mcp_protocol::tools::ToolSchema;

/// The type of a form field, derived from the JSON Schema property type.
#[derive(Debug, Clone)]
pub enum FieldType {
    /// Free-text string input.
    Text,
    /// Integer numeric input.
    Integer,
    /// Floating-point numeric input.
    Number,
    /// Boolean toggle (Space to flip).
    Boolean,
    /// String with a fixed set of allowed values (arrow keys to cycle).
    Enum(Vec<String>),
    /// Array of strings entered as comma-separated values.
    Array,
}

/// A single field in the tool parameter form.
#[derive(Debug, Clone)]
pub struct FormField {
    /// The JSON property name.
    pub name: String,
    /// The kind of input control to render.
    pub field_type: FieldType,
    /// The current text value being edited.
    pub value: String,
    /// Whether this field is required by the schema.
    pub required: bool,
    /// Optional description from the schema.
    pub description: Option<String>,
    /// If true, the field is auto-injected (e.g. account_id from session) and
    /// hidden from the interactive form.
    pub managed: bool,
}

/// Tracks form-level state for a single tool invocation.
#[derive(Debug, Clone)]
pub struct FormState {
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Ordered list of fields parsed from the tool schema.
    pub fields: Vec<FormField>,
    /// Index of the currently selected (highlighted) field among visible fields.
    pub selected_field: usize,
    /// Whether the user is actively editing the selected field's value.
    pub editing: bool,
}

impl FormState {
    /// Build a new form from a tool name + its input schema.
    ///
    /// `managed_fields` lists property names that should be auto-injected rather
    /// than shown to the user (e.g. `["account_id"]`).
    pub fn new(tool_name: &str, schema: &ToolSchema, managed_fields: &[String]) -> Self {
        let fields = build_form_fields(schema, managed_fields);
        Self {
            tool_name: tool_name.to_string(),
            fields,
            selected_field: 0,
            editing: false,
        }
    }

    /// The visible (non-managed) fields that the user interacts with.
    pub fn visible_fields(&self) -> Vec<(usize, &FormField)> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.managed)
            .collect()
    }

    /// Number of visible fields.
    pub fn visible_count(&self) -> usize {
        self.fields.iter().filter(|f| !f.managed).count()
    }

    /// Check if the tool has non-managed required fields (needs user input).
    pub fn has_required_input(&self) -> bool {
        self.fields.iter().any(|f| !f.managed && f.required)
    }

    /// Returns names of required visible fields that are still empty.
    pub fn missing_required(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|f| !f.managed && f.required && f.value.is_empty())
            .map(|f| f.name.as_str())
            .collect()
    }

    /// Whether this form can be submitted (all required visible fields filled).
    pub fn is_ready(&self) -> bool {
        self.missing_required().is_empty()
    }

    /// Move selection to the next visible field.
    pub fn select_next(&mut self) {
        let count = self.visible_count();
        if count > 0 {
            self.selected_field = (self.selected_field + 1) % count;
        }
    }

    /// Move selection to the previous visible field.
    pub fn select_prev(&mut self) {
        let count = self.visible_count();
        if count > 0 {
            self.selected_field = (self.selected_field + count - 1) % count;
        }
    }

    /// Returns a mutable reference to the currently selected visible field,
    /// or `None` if there are no visible fields.
    pub fn selected_field_mut(&mut self) -> Option<&mut FormField> {
        let visible_indices: Vec<usize> = self
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.managed)
            .map(|(i, _)| i)
            .collect();

        visible_indices
            .get(self.selected_field)
            .and_then(|&idx| self.fields.get_mut(idx))
    }

    /// Toggle a boolean field at the current selection.
    pub fn toggle_boolean(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.field_type, FieldType::Boolean) {
                field.value = if field.value == "true" {
                    "false".to_string()
                } else {
                    "true".to_string()
                };
            }
        }
    }

    /// Cycle an enum field forward.
    pub fn cycle_enum_forward(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if let FieldType::Enum(ref values) = field.field_type {
                if let Some(pos) = values.iter().position(|v| v == &field.value) {
                    field.value = values[(pos + 1) % values.len()].clone();
                } else if let Some(first) = values.first() {
                    field.value = first.clone();
                }
            }
        }
    }

    /// Cycle an enum field backward.
    pub fn cycle_enum_backward(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if let FieldType::Enum(ref values) = field.field_type {
                if let Some(pos) = values.iter().position(|v| v == &field.value) {
                    field.value = values[(pos + values.len() - 1) % values.len()].clone();
                } else if let Some(last) = values.last() {
                    field.value = last.clone();
                }
            }
        }
    }

    /// Append a character to the selected field's value (while editing).
    pub fn push_char(&mut self, ch: char) {
        if let Some(field) = self.selected_field_mut() {
            field.value.push(ch);
        }
    }

    /// Delete the last character from the selected field's value.
    pub fn pop_char(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.value.pop();
        }
    }
}

// ---------------------------------------------------------------------------
// Schema parsing
// ---------------------------------------------------------------------------

/// Build form fields from a `ToolSchema`, marking any field whose name appears
/// in `managed_fields` as managed (hidden from the UI, injected on submit).
pub fn build_form_fields(schema: &ToolSchema, managed_fields: &[String]) -> Vec<FormField> {
    let properties = match &schema.properties {
        Some(props) => props,
        None => return Vec::new(),
    };

    let required_set: Vec<&str> = schema
        .required
        .as_ref()
        .map(|r| r.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let mut fields: Vec<FormField> = properties
        .iter()
        .map(|(name, json_schema)| {
            let (field_type, description, default_value) = parse_json_schema(json_schema);
            let required = required_set.contains(&name.as_str());
            let managed = managed_fields.contains(name);

            FormField {
                name: name.clone(),
                field_type,
                value: default_value.unwrap_or_default(),
                required,
                description,
                managed,
            }
        })
        .collect();

    // Sort: required first, then alphabetical within each group.
    fields.sort_by(|a, b| {
        b.required
            .cmp(&a.required)
            .then_with(|| a.name.cmp(&b.name))
    });

    fields
}

/// Extract field type, description, and optional default from a `JsonSchema`.
fn parse_json_schema(schema: &JsonSchema) -> (FieldType, Option<String>, Option<String>) {
    match schema {
        JsonSchema::String {
            description,
            enum_values,
            ..
        } => {
            if let Some(values) = enum_values {
                let default = values.first().cloned();
                (
                    FieldType::Enum(values.clone()),
                    description.clone(),
                    default,
                )
            } else {
                (FieldType::Text, description.clone(), None)
            }
        }
        JsonSchema::Integer { description, .. } => {
            (FieldType::Integer, description.clone(), None)
        }
        JsonSchema::Number { description, .. } => {
            (FieldType::Number, description.clone(), None)
        }
        JsonSchema::Boolean { description } => (
            FieldType::Boolean,
            description.clone(),
            Some("false".to_string()),
        ),
        JsonSchema::Array { description, .. } => {
            (FieldType::Array, description.clone(), None)
        }
        JsonSchema::Object { description, .. } => {
            // Nested objects are edited as raw JSON text.
            (FieldType::Text, description.clone(), None)
        }
    }
}

// ---------------------------------------------------------------------------
// Value assembly
// ---------------------------------------------------------------------------

/// Assemble form field values into a JSON object suitable for `tools/call`.
///
/// - Converts types according to `FieldType`.
/// - Skips optional fields that have empty values.
/// - Includes managed fields (their values will have been injected externally).
pub fn assemble_args(fields: &[FormField]) -> Value {
    let mut map = serde_json::Map::new();

    for field in fields {
        // Skip optional fields with no value.
        if !field.required && field.value.is_empty() {
            continue;
        }

        let value = match &field.field_type {
            FieldType::Text | FieldType::Enum(_) => Value::String(field.value.clone()),
            FieldType::Integer => field
                .value
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::String(field.value.clone())),
            FieldType::Number => field
                .value
                .parse::<f64>()
                .map(|n| {
                    serde_json::Number::from_f64(n)
                        .map(Value::Number)
                        .unwrap_or(Value::String(field.value.clone()))
                })
                .unwrap_or_else(|_| Value::String(field.value.clone())),
            FieldType::Boolean => Value::Bool(field.value == "true"),
            FieldType::Array => {
                let items: Vec<Value> = field
                    .value
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                Value::Array(items)
            }
        };

        map.insert(field.name.clone(), value);
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the tool parameter form into the given area.
pub fn render_tool_form(frame: &mut Frame, area: Rect, form: &FormState) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Title
    lines.push(Line::from(vec![Span::styled(
        format!(" Tool: {} ", form.tool_name),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Show managed fields as read-only at the top.
    let managed: Vec<&FormField> = form.fields.iter().filter(|f| f.managed).collect();
    if !managed.is_empty() {
        for field in &managed {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}: ", field.name),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("(from session)", Style::default().fg(Color::DarkGray)),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Render visible fields.
    let visible = form.visible_fields();
    if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No additional parameters required.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }
    for (vis_idx, (_real_idx, field)) in visible.iter().enumerate() {
        let is_selected = vis_idx == form.selected_field;
        let highlight = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let marker = if field.required { "* " } else { "  " };

        match &field.field_type {
            FieldType::Boolean => {
                let toggle_text = if field.value == "true" {
                    "[true]"
                } else {
                    "[false]"
                };
                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{}: ", field.name), highlight),
                    Span::styled(
                        toggle_text.to_string(),
                        Style::default().fg(if field.value == "true" {
                            Color::Green
                        } else {
                            Color::Red
                        }),
                    ),
                ]));
            }
            FieldType::Enum(values) => {
                let display = if values.contains(&field.value) {
                    format!("< {} >", field.value)
                } else {
                    "< >".to_string()
                };
                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{}: ", field.name), highlight),
                    Span::styled(display, Style::default().fg(Color::Magenta)),
                ]));
            }
            _ => {
                let cursor = if is_selected && form.editing {
                    "_"
                } else {
                    ""
                };
                let display_value = if field.value.is_empty() && !form.editing {
                    "".to_string()
                } else {
                    format!("{}{}", field.value, cursor)
                };
                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{}: ", field.name), highlight),
                    Span::styled(
                        format!("[{}]", display_value),
                        if is_selected && form.editing {
                            Style::default().fg(Color::White)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                ]));
            }
        }

        // Description below the field.
        if let Some(ref desc) = field.description {
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", desc),
                Style::default().fg(Color::DarkGray),
            )]));
        }
    }

    // Footer
    lines.push(Line::from(""));
    let footer = if form.editing {
        "[Enter] Done  [Esc] Cancel edit"
    } else if visible.is_empty() {
        "[e] Execute  [Esc] Back"
    } else {
        "[e] Execute  [Enter] Edit field  [j/k] Navigate  [Space] Toggle  [Esc] Back"
    };
    lines.push(Line::from(vec![Span::styled(
        footer,
        Style::default().fg(Color::DarkGray),
    )]));

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Parameters "),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
