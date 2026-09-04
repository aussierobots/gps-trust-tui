use std::collections::{HashMap, HashSet};

use ratatui::widgets::ListState;

use crate::action::Action;
use crate::auth::session::AuthSession;
use crate::mcp::id_cache::{AUTO_FETCH_CANDIDATES, IdCache};
use crate::mcp::types::{ActiveTask, IdFetchRequest, ServerCaps, ServerId, ServerRegistry, ToolEntry};
use crate::ui::result_view::ResultState;
use crate::ui::tool_form::FormState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
    /// No usable credential for this server (auth server declined the audience).
    Unauthorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
    FormEdit,
    #[allow(dead_code)]
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    ToolList,
    Detail,
    Form,
    Result,
}

pub struct App {
    pub should_quit: bool,
    pub input_mode: InputMode,
    pub auth_session: Option<AuthSession>,
    pub server_state: HashMap<ServerId, ConnectionState>,
    #[allow(dead_code)]
    pub server_caps: HashMap<ServerId, ServerCaps>,
    /// Ordered list of configured servers (drives status-bar rendering order).
    pub servers: Vec<ServerId>,

    // Tool browser state
    pub tools: Vec<ToolEntry>,
    pub tool_list_state: ListState,
    pub filter_text: String,
    pub filtered_indices: Vec<usize>,

    // Focus management
    pub focus: PanelFocus,

    // Form state (when editing tool parameters)
    pub form_state: Option<FormState>,

    // Result state
    pub result_state: Option<ResultState>,

    // Task state
    pub active_task: Option<ActiveTask>,

    /// `*_id` values seen in tool results, offered as cycle-through
    /// suggestions on matching form fields (see `cycle_suggestion`).
    pub id_cache: IdCache,
    /// Carry-forward field names an auto-fetch has already been queued for
    /// this session — prevents re-fetching every time an empty form reopens.
    /// Cleared per-field by an explicit refresh (`R`).
    id_fetch_attempted: HashSet<String>,
    /// Background id-cache fetches queued this tick, drained by main.rs.
    pub id_fetch_requests: Vec<IdFetchRequest>,

    // Execute signal — set by 'e' in form, consumed by main loop
    pub execute_requested: bool,
    // Reconnect signal — set by 'r', consumed by main loop
    pub reconnect_requested: bool,
    // Logout signal — set by 'L', consumed by main loop after quit
    pub logout_requested: bool,
}

impl App {
    pub fn new(registry: &ServerRegistry) -> Self {
        let servers: Vec<ServerId> = registry.iter().cloned().collect();
        let mut server_state = HashMap::new();
        for server in &servers {
            server_state.insert(server.clone(), ConnectionState::Disconnected);
        }

        Self {
            should_quit: false,
            input_mode: InputMode::Normal,
            auth_session: None,
            server_state,
            server_caps: HashMap::new(),
            servers,
            tools: Vec::new(),
            tool_list_state: ListState::default(),
            filter_text: String::new(),
            filtered_indices: Vec::new(),
            focus: PanelFocus::ToolList,
            form_state: None,
            result_state: None,
            active_task: None,
            id_cache: IdCache::new(),
            id_fetch_attempted: HashSet::new(),
            id_fetch_requests: Vec::new(),
            execute_requested: false,
            reconnect_requested: false,
            logout_requested: false,
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => {
                // Only from Ctrl+C, always quits
                self.should_quit = true;
            }

            // --- Auth lifecycle ---
            Action::AuthSuccess(session) => {
                self.auth_session = Some(session);
                self.input_mode = InputMode::Normal;
            }

            // --- MCP lifecycle ---
            Action::McpConnecting(server) => {
                self.server_state.insert(server, ConnectionState::Connecting);
            }
            Action::McpConnected(server) => {
                self.server_state.insert(server, ConnectionState::Connected);
            }
            Action::McpError(server, _msg) => {
                self.server_state.insert(server, ConnectionState::Error);
            }
            Action::McpUnauthorized(server) => {
                self.server_state
                    .insert(server, ConnectionState::Unauthorized);
            }
            Action::McpToolsRefreshed(_server) => {
                // Tools are set externally via set_tools(); this is a signal only.
            }

            // --- MCP progress and results ---
            Action::McpProgress {
                progress,
                total,
                message,
                ..
            } => {
                if let Some(ref mut task) = self.active_task {
                    task.progress = Some(progress);
                    task.total = total;
                    task.message = message;
                }
            }
            Action::McpToolResult(result) => {
                self.scan_result_for_id_suggestions(&result);
                // Preserve tool_name from the placeholder result state
                let tool_name = self
                    .result_state
                    .as_ref()
                    .and_then(|rs| rs.tool_name.clone());
                let mut rs = ResultState::with_result(*result);
                rs.tool_name = tool_name;
                self.result_state = Some(rs);
                self.active_task = None;
                self.focus = PanelFocus::Result;
            }
            Action::IdCacheFetched(result) => {
                self.scan_result_for_id_suggestions(&result);
                self.refill_empty_carry_forward_fields();
            }

            // --- Tool interaction ---
            Action::ToolCancel => {
                // Signal sent to the MCP dispatch layer to cancel the active task.
            }

            // --- Form interaction ---
            Action::FormFieldToggle => {
                if let Some(ref mut form) = self.form_state {
                    form.toggle_boolean();
                }
            }
            Action::FormInputChar(c) => {
                if let Some(ref mut form) = self.form_state {
                    form.push_char(c);
                }
            }
            Action::FormInputBackspace => {
                if let Some(ref mut form) = self.form_state {
                    form.pop_char();
                }
            }

            // --- Navigation ---
            Action::ScrollUp => {
                match self.focus {
                    PanelFocus::ToolList => self.select_prev(),
                    PanelFocus::Result => {
                        if let Some(ref mut rs) = self.result_state {
                            rs.scroll_up(1);
                        }
                    }
                    PanelFocus::Form => {
                        if let Some(ref mut form) = self.form_state {
                            form.select_prev();
                        }
                    }
                    _ => {}
                }
            }
            Action::ScrollDown => {
                match self.focus {
                    PanelFocus::ToolList => self.select_next(),
                    PanelFocus::Result => {
                        if let Some(ref mut rs) = self.result_state {
                            rs.scroll_down(1);
                        }
                    }
                    PanelFocus::Form => {
                        if let Some(ref mut form) = self.form_state {
                            form.select_next();
                        }
                    }
                    _ => {}
                }
            }
            Action::CyclePrev => self.cycle_suggestion(false),
            Action::CycleNext => self.cycle_suggestion(true),
            Action::FocusNext => {
                self.focus = match self.focus {
                    PanelFocus::ToolList => PanelFocus::Detail,
                    PanelFocus::Detail => {
                        if self.form_state.is_some() {
                            PanelFocus::Form
                        } else if self.result_state.is_some() {
                            PanelFocus::Result
                        } else {
                            PanelFocus::ToolList
                        }
                    }
                    PanelFocus::Form => {
                        if self.result_state.is_some() {
                            PanelFocus::Result
                        } else {
                            PanelFocus::ToolList
                        }
                    }
                    PanelFocus::Result => PanelFocus::ToolList,
                };
            }
            Action::FocusPrev => {
                self.focus = match self.focus {
                    PanelFocus::ToolList => {
                        if self.result_state.is_some() {
                            PanelFocus::Result
                        } else if self.form_state.is_some() {
                            PanelFocus::Form
                        } else {
                            PanelFocus::Detail
                        }
                    }
                    PanelFocus::Detail => PanelFocus::ToolList,
                    PanelFocus::Form => PanelFocus::Detail,
                    PanelFocus::Result => {
                        if self.form_state.is_some() {
                            PanelFocus::Form
                        } else {
                            PanelFocus::Detail
                        }
                    }
                };
            }
            Action::FilterStart => {
                self.input_mode = InputMode::Filter;
                self.focus = PanelFocus::ToolList;
            }
            // --- Central character routing ---
            // ALL printable chars arrive here. Route based on mode + focus.
            Action::FilterChar(c) => {
                self.handle_char(c);
            }
            Action::FilterBackspace => {
                match self.input_mode {
                    InputMode::Filter => {
                        self.filter_text.pop();
                        self.apply_filter();
                    }
                    InputMode::FormEdit => {
                        if let Some(ref mut form) = self.form_state {
                            form.pop_char();
                        }
                    }
                    _ => {}
                }
            }
            Action::PasteText(text) => {
                // Paste: insert all chars into the active text input
                for c in text.chars() {
                    match self.input_mode {
                        InputMode::Filter => {
                            self.filter_text.push(c);
                        }
                        InputMode::FormEdit => {
                            if let Some(ref mut form) = self.form_state {
                                form.push_char(c);
                            }
                        }
                        _ => {} // Ignore paste in Normal/Login mode
                    }
                }
                if self.input_mode == InputMode::Filter {
                    self.apply_filter();
                }
            }
            Action::Enter => {
                match self.input_mode {
                    InputMode::Filter => {
                        // Accept the current filter and return to Normal
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::FormEdit => {
                        // Stop editing the current field
                        if let Some(ref mut form) = self.form_state {
                            form.editing = false;
                        }
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::Normal => {
                        match self.focus {
                            PanelFocus::ToolList | PanelFocus::Detail => {
                                if self.selected_tool_needs_input() {
                                    self.open_form_for_selected();
                                } else {
                                    // No user input needed — execute immediately
                                    self.execute_requested = true;
                                }
                            }
                            PanelFocus::Form => {
                                // Start editing the selected field (only if there are visible fields)
                                if let Some(ref mut form) = self.form_state {
                                    if !form.editing && form.visible_count() > 0 {
                                        form.editing = true;
                                        self.input_mode = InputMode::FormEdit;
                                    }
                                }
                            }
                            PanelFocus::Result => {}
                        }
                    }
                    InputMode::Login => {}
                }
            }
            Action::Escape => {
                match self.input_mode {
                    InputMode::Filter => {
                        self.filter_text.clear();
                        self.apply_filter();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::FormEdit => {
                        if let Some(ref mut form) = self.form_state {
                            form.editing = false;
                        }
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::Normal => {
                        match self.focus {
                            PanelFocus::Result => {
                                self.result_state = None;
                                self.focus = if self.form_state.is_some() {
                                    PanelFocus::Form
                                } else {
                                    PanelFocus::Detail
                                };
                            }
                            PanelFocus::Form => {
                                self.form_state = None;
                                self.focus = PanelFocus::Detail;
                            }
                            PanelFocus::Detail => {
                                self.focus = PanelFocus::ToolList;
                            }
                            PanelFocus::ToolList => {}
                        }
                    }
                    InputMode::Login => {}
                }
            }

            Action::ResultNextTab => {
                if let Some(ref mut rs) = self.result_state {
                    rs.next_tab();
                }
            }

            Action::Reconnect => {
                // Signal handled by the MCP connection layer.
            }

            Action::ToolsLoaded(tools) => {
                self.set_tools(tools);
            }
        }
    }

    /// Route a printable character based on current mode and focus.
    fn handle_char(&mut self, c: char) {
        match self.input_mode {
            // --- Filter mode: all chars go to filter ---
            InputMode::Filter => {
                self.filter_text.push(c);
                self.apply_filter();
            }
            // --- Form edit mode: all chars go to active field ---
            InputMode::FormEdit => {
                if let Some(ref mut form) = self.form_state {
                    form.push_char(c);
                }
            }
            // --- Normal mode: interpret chars as commands based on focus ---
            InputMode::Normal => match c {
                'q' => self.should_quit = true,
                'j' => self.update(Action::ScrollDown),
                'k' => self.update(Action::ScrollUp),
                '/' => {
                    self.input_mode = InputMode::Filter;
                    self.focus = PanelFocus::ToolList;
                }
                ' ' => {
                    if self.focus == PanelFocus::Form {
                        if let Some(ref mut form) = self.form_state {
                            form.toggle_boolean();
                        }
                    }
                }
                '1' | '2' => {
                    if let Some(ref mut rs) = self.result_state {
                        rs.next_tab();
                    }
                }
                'e' => {
                    // Execute ONLY from Form pane
                    if self.focus == PanelFocus::Form {
                        // ToolExecute is intercepted by main.rs
                        // We just need to emit the signal — but we can't from here.
                        // Instead, set a flag that main.rs checks.
                        self.execute_requested = true;
                    }
                }
                'r' => {
                    self.reconnect_requested = true;
                }
                'R' => {
                    self.refresh_selected_field_suggestions();
                }
                'L' => {
                    self.logout_requested = true;
                    self.should_quit = true;
                }
                _ => {}
            },
            InputMode::Login => {}
        }
    }

    /// Feed a tool result's `*_id` fields into `id_cache` for later
    /// suggestion. Prefers `structured_content`; falls back to parsing
    /// text content blocks as JSON. Best-effort — a non-JSON/text result
    /// simply yields no suggestions.
    fn scan_result_for_id_suggestions(&mut self, result: &turul_mcp_client::CallToolResult) {
        if let Some(ref structured) = result.structured_content {
            self.id_cache.scan(structured);
            return;
        }
        for block in &result.content {
            if let turul_mcp_client::ContentBlock::Text { text, .. } = block
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
            {
                self.id_cache.scan(&value);
            }
        }
    }

    /// After an id-cache fetch lands, fill in any still-empty form field
    /// whose name now has a carry-forward default — the field the user is
    /// looking at may have been sitting empty waiting for exactly this.
    fn refill_empty_carry_forward_fields(&mut self) {
        let Some(form) = self.form_state.as_mut() else {
            return;
        };
        for field in form.fields.iter_mut() {
            if field.value.is_empty()
                && let Some(default) = self.id_cache.default_for(&field.name)
            {
                field.value = default.to_string();
            }
        }
    }

    /// Cycle the selected form field's value through its suggestions:
    /// schema enum values, or — for a plain text field — cached `*_id`
    /// values seen in prior tool results (see `id_cache`).
    fn cycle_suggestion(&mut self, forward: bool) {
        use crate::ui::tool_form::FieldType;

        if self.focus != PanelFocus::Form {
            return;
        }
        let field_info = self.form_state.as_mut().and_then(|form| {
            let field = form.selected_field_mut()?;
            Some((field.name.clone(), field.field_type.clone()))
        });
        let Some((name, field_type)) = field_info else {
            return;
        };

        let choices: Vec<String> = match &field_type {
            FieldType::Enum(values) => values.clone(),
            _ => self.id_cache.suggestions(&name).to_vec(),
        };
        if choices.is_empty() {
            return;
        }

        let Some(field) = self
            .form_state
            .as_mut()
            .and_then(|form| form.selected_field_mut())
        else {
            return;
        };
        let pos = choices.iter().position(|v| v == &field.value);
        let next = match (pos, forward) {
            (Some(p), true) => (p + 1) % choices.len(),
            (Some(p), false) => (p + choices.len() - 1) % choices.len(),
            (None, true) => 0,
            (None, false) => choices.len() - 1,
        };
        field.value = choices[next].clone();
    }

    /// Fields that are auto-injected from the session (hidden from the form).
    fn managed_fields(&self) -> Vec<String> {
        if self.auth_session.is_some() {
            vec!["account_id".to_string()]
        } else {
            Vec::new()
        }
    }

    /// Get the currently selected tool entry, if any.
    pub fn selected_tool(&self) -> Option<&ToolEntry> {
        let selected = self.tool_list_state.selected()?;
        let &idx = self.filtered_indices.get(selected)?;
        self.tools.get(idx)
    }

    /// Check if the selected tool needs user-editable parameters.
    /// Returns true if the tool has required fields that aren't managed.
    #[allow(dead_code)]
    pub fn selected_tool_needs_input(&self) -> bool {
        if let Some(entry) = self.selected_tool() {
            let managed = self.managed_fields();
            let form = FormState::new(&entry.tool.name, &entry.tool.input_schema, &managed, &self.id_cache);
            form.has_required_input()
        } else {
            false
        }
    }

    /// Open the form for the selected tool and return whether it was opened.
    pub fn open_form_for_selected(&mut self) -> bool {
        if let Some(entry) = self.selected_tool().cloned() {
            let managed = self.managed_fields();
            let form = FormState::new(&entry.tool.name, &entry.tool.input_schema, &managed, &self.id_cache);
            self.form_state = Some(form);
            self.focus = PanelFocus::Form;
            self.queue_id_fetches_for_open_form();
            true
        } else {
            false
        }
    }

    /// For every empty, non-managed carry-forward field on the open form with
    /// nothing cached yet, queue a background fetch from the first candidate
    /// discovery tool that's actually in this session's tool list (which
    /// candidate exists depends on the caller's entity type — see
    /// `AUTO_FETCH_CANDIDATES`). At most once per field name per session
    /// unless explicitly refreshed (`R`) — see `id_fetch_attempted`.
    fn queue_id_fetches_for_open_form(&mut self) {
        let Some(form) = self.form_state.as_ref() else {
            return;
        };
        let field_names: Vec<String> = form
            .visible_fields()
            .into_iter()
            .filter(|(_, f)| f.value.is_empty())
            .map(|(_, f)| f.name.clone())
            .collect();

        for name in field_names {
            self.queue_id_fetch(&name, false);
        }
    }

    /// Queue a background fetch to populate `field_name`'s cache entry, if
    /// it has an [`AUTO_FETCH_CANDIDATES`] entry and (unless `force`) hasn't
    /// already been attempted this session and isn't already cached.
    fn queue_id_fetch(&mut self, field_name: &str, force: bool) {
        if !force {
            if self.id_cache.default_for(field_name).is_some() {
                return;
            }
            if self.id_fetch_attempted.contains(field_name) {
                return;
            }
        }
        let Some((_, candidates)) = AUTO_FETCH_CANDIDATES.iter().find(|(name, _)| *name == field_name)
        else {
            return;
        };
        let Some(entry) = candidates
            .iter()
            .find_map(|tool_name| self.tools.iter().find(|e| e.tool.name == *tool_name))
        else {
            return;
        };

        self.id_fetch_attempted.insert(field_name.to_string());
        self.id_fetch_requests.push(IdFetchRequest {
            server: entry.server.clone(),
            tool_name: entry.tool.name.clone(),
            field_name: field_name.to_string(),
        });
    }

    /// Force-refresh the currently selected form field's suggestions (`R`),
    /// bypassing the once-per-session auto-fetch gate — e.g. after adding a
    /// new device, so a stale cache doesn't hide it.
    fn refresh_selected_field_suggestions(&mut self) {
        if self.focus != PanelFocus::Form {
            return;
        }
        let Some(name) = self
            .form_state
            .as_ref()
            .and_then(|form| form.visible_fields().get(form.selected_field).map(|(_, f)| f.name.clone()))
        else {
            return;
        };
        self.queue_id_fetch(&name, true);
    }

    /// Replace the tool list and rebuild the filter index.
    pub fn set_tools(&mut self, tools: Vec<ToolEntry>) {
        self.tools = tools;
        self.apply_filter();
    }

    /// Rebuild `filtered_indices` from the current `filter_text`.
    pub fn apply_filter(&mut self) {
        let query = self.filter_text.to_lowercase();
        self.filtered_indices = self
            .tools
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.tool.name.to_lowercase().contains(&query)
                    || entry.display_name().to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection
        if self.filtered_indices.is_empty() {
            self.tool_list_state.select(None);
        } else {
            match self.tool_list_state.selected() {
                Some(sel) if sel >= self.filtered_indices.len() => {
                    self.tool_list_state
                        .select(Some(self.filtered_indices.len() - 1));
                }
                None if !self.filtered_indices.is_empty() => {
                    self.tool_list_state.select(Some(0));
                }
                _ => {}
            }
        }
    }

    /// Number of tools currently visible (filtered or all).
    pub fn visible_tool_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Clear form and result when changing tool selection.
    fn clear_right_panel(&mut self) {
        self.form_state = None;
        self.result_state = None;
        self.active_task = None;
    }

    /// Move selection down in the filtered tool list.
    fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let prev_selected = self.tool_list_state.selected();
        let next = match prev_selected {
            Some(i) => {
                if i + 1 < self.filtered_indices.len() {
                    i + 1
                } else {
                    0
                }
            }
            None => 0,
        };
        self.tool_list_state.select(Some(next));
        if prev_selected != Some(next) {
            self.clear_right_panel();
        }
    }

    /// Move selection up in the filtered tool list.
    fn select_prev(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let prev_selected = self.tool_list_state.selected();
        let prev = match prev_selected {
            Some(0) => self.filtered_indices.len() - 1,
            Some(i) => i - 1,
            None => 0,
        };
        self.tool_list_state.select(Some(prev));
        if prev_selected != Some(prev) {
            self.clear_right_panel();
        }
    }

    pub fn identity_display(&self) -> String {
        self.auth_session
            .as_ref()
            .map(|s| s.display_name.clone())
            .unwrap_or_else(|| "(not authenticated)".to_string())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tool_form::{FieldType, FormField};

    fn app_with_form(fields: Vec<FormField>) -> App {
        let registry = ServerRegistry::from_specs("https://u/mcp", "https://a/mcp", &[]).unwrap();
        let mut app = App::new(&registry);
        app.focus = PanelFocus::Form;
        app.form_state = Some(FormState {
            tool_name: "test_tool".to_string(),
            fields,
            selected_field: 0,
            editing: false,
        });
        app
    }

    fn field(name: &str, field_type: FieldType, value: &str) -> FormField {
        FormField {
            name: name.to_string(),
            field_type,
            value: value.to_string(),
            required: false,
            description: None,
            managed: false,
        }
    }

    fn tool_entry(server_key: &str, tool_name: &str) -> ToolEntry {
        ToolEntry {
            server: ServerId::from_key_url(server_key, "https://example/mcp"),
            tool: turul_mcp_client::Tool::new(
                tool_name,
                turul_mcp_client::tools::ToolSchema::object(),
            ),
        }
    }

    #[test]
    fn cycle_suggestion_advances_through_enum_values() {
        let mut app = app_with_form(vec![field(
            "status",
            FieldType::Enum(vec!["a".into(), "b".into(), "c".into()]),
            "a",
        )]);
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "b");
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "c");
        app.cycle_suggestion(true); // wraps
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "a");
        app.cycle_suggestion(false); // wraps the other way
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "c");
    }

    #[test]
    fn cycle_suggestion_uses_id_cache_for_text_fields() {
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        app.id_cache.scan(&serde_json::json!({
            "devices": [{ "device_id": "D#1" }, { "device_id": "D#2" }]
        }));
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "D#2");
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "D#1");
    }

    #[test]
    fn cycle_suggestion_is_noop_with_no_suggestions() {
        let mut app = app_with_form(vec![field("notes", FieldType::Text, "hello")]);
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "hello");
    }

    #[test]
    fn cycle_suggestion_ignored_outside_form_focus() {
        let mut app = app_with_form(vec![field(
            "status",
            FieldType::Enum(vec!["a".into(), "b".into()]),
            "a",
        )]);
        app.focus = PanelFocus::ToolList;
        app.cycle_suggestion(true);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "a");
    }

    #[test]
    fn mcp_tool_result_action_populates_cache_and_cycle_next_action_applies_it() {
        // End-to-end through the same Action::update path main.rs drives:
        // a tool result carrying a device_id should make it selectable via
        // the Right-arrow (CycleNext) action on a device_id form field.
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);

        let result = turul_mcp_client::CallToolResult {
            content: vec![],
            is_error: None,
            structured_content: Some(serde_json::json!({
                "devices": [{ "device_id": "D#42" }]
            })),
            meta: None,
        };
        app.update(Action::McpToolResult(Box::new(result)));
        // McpToolResult moves focus to the Result panel; simulate the user
        // tabbing back to the still-open form, as they would in the real TUI.
        app.focus = PanelFocus::Form;

        app.update(Action::CycleNext);
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "D#42");
    }

    #[test]
    fn scan_result_for_id_suggestions_reads_structured_content() {
        let mut app = App::new(
            &ServerRegistry::from_specs("https://u/mcp", "https://a/mcp", &[]).unwrap(),
        );
        let result = turul_mcp_client::CallToolResult {
            content: vec![],
            is_error: None,
            structured_content: Some(serde_json::json!({ "agent_id": "AG#1" })),
            meta: None,
        };
        app.scan_result_for_id_suggestions(&result);
        assert_eq!(app.id_cache.suggestions("agent_id"), &["AG#1"]);
    }

    #[test]
    fn open_form_queues_auto_fetch_for_empty_carry_forward_field_with_known_tool() {
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        app.tools = vec![tool_entry("user", "station_devices")];
        app.queue_id_fetches_for_open_form();

        assert_eq!(app.id_fetch_requests.len(), 1);
        assert_eq!(app.id_fetch_requests[0].field_name, "device_id");
        assert_eq!(app.id_fetch_requests[0].tool_name, "station_devices");
    }

    #[test]
    fn auto_fetch_skips_field_with_no_matching_tool_in_session() {
        // None of AUTO_FETCH_CANDIDATES' device_id tools are in `tools`.
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        app.tools = vec![tool_entry("user", "device_location")];
        app.queue_id_fetches_for_open_form();
        assert!(app.id_fetch_requests.is_empty());
    }

    #[test]
    fn auto_fetch_only_queues_once_per_session_unless_refreshed() {
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        app.tools = vec![tool_entry("user", "station_devices")];

        app.queue_id_fetches_for_open_form();
        assert_eq!(app.id_fetch_requests.len(), 1);
        app.id_fetch_requests.clear();

        // Reopening the same empty field doesn't re-queue — already attempted.
        app.queue_id_fetches_for_open_form();
        assert!(app.id_fetch_requests.is_empty());

        // Explicit refresh (R) bypasses the once-per-session gate.
        app.refresh_selected_field_suggestions();
        assert_eq!(app.id_fetch_requests.len(), 1);
    }

    #[test]
    fn auto_fetch_skipped_when_already_cached() {
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        app.tools = vec![tool_entry("user", "station_devices")];
        app.id_cache.scan(&serde_json::json!({ "device_id": "D#1" }));

        app.queue_id_fetches_for_open_form();
        assert!(app.id_fetch_requests.is_empty());
    }

    #[test]
    fn id_cache_fetched_action_fills_waiting_empty_field() {
        let mut app = app_with_form(vec![field("device_id", FieldType::Text, "")]);
        let result = turul_mcp_client::CallToolResult {
            content: vec![],
            is_error: None,
            structured_content: Some(serde_json::json!({
                "devices": [{ "deviceId": "D#99" }]
            })),
            meta: None,
        };
        app.update(Action::IdCacheFetched(Box::new(result)));
        assert_eq!(app.form_state.as_ref().unwrap().fields[0].value, "D#99");
    }
}
