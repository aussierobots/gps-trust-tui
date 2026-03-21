use std::collections::HashMap;

use ratatui::widgets::ListState;

use crate::action::Action;
use crate::auth::session::AuthSession;
use crate::mcp::types::{ActiveTask, ServerCaps, ServerIdentity, ToolEntry};
use crate::ui::result_view::ResultState;
use crate::ui::tool_form::FormState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
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
    pub server_state: HashMap<ServerIdentity, ConnectionState>,
    #[allow(dead_code)]
    pub server_caps: HashMap<ServerIdentity, ServerCaps>,

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

    // Execute signal — set by 'e' in form, consumed by main loop
    pub execute_requested: bool,
    // Reconnect signal — set by 'r', consumed by main loop
    pub reconnect_requested: bool,
    // Logout signal — set by 'L', consumed by main loop after quit
    pub logout_requested: bool,
}

impl App {
    pub fn new() -> Self {
        let mut server_state = HashMap::new();
        server_state.insert(ServerIdentity::User, ConnectionState::Disconnected);
        server_state.insert(ServerIdentity::Agent, ConnectionState::Disconnected);

        Self {
            should_quit: false,
            input_mode: InputMode::Normal,
            auth_session: None,
            server_state,
            server_caps: HashMap::new(),
            tools: Vec::new(),
            tool_list_state: ListState::default(),
            filter_text: String::new(),
            filtered_indices: Vec::new(),
            focus: PanelFocus::ToolList,
            form_state: None,
            result_state: None,
            active_task: None,
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
            Action::McpDisconnected(server) => {
                self.server_state
                    .insert(server, ConnectionState::Disconnected);
            }
            Action::McpError(server, _msg) => {
                self.server_state.insert(server, ConnectionState::Error);
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
                'L' => {
                    self.logout_requested = true;
                    self.should_quit = true;
                }
                _ => {}
            },
            InputMode::Login => {}
        }
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
            let form = FormState::new(&entry.tool.name, &entry.tool.input_schema, &managed);
            form.has_required_input()
        } else {
            false
        }
    }

    /// Open the form for the selected tool and return whether it was opened.
    pub fn open_form_for_selected(&mut self) -> bool {
        if let Some(entry) = self.selected_tool().cloned() {
            let managed = self.managed_fields();
            let form = FormState::new(&entry.tool.name, &entry.tool.input_schema, &managed);
            self.form_state = Some(form);
            self.focus = PanelFocus::Form;
            true
        } else {
            false
        }
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
