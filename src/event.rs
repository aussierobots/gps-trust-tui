use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::action::Action;

pub struct EventHandler {
    tx: mpsc::UnboundedSender<Action>,
}

impl EventHandler {
    pub fn new(tx: mpsc::UnboundedSender<Action>) -> Self {
        Self { tx }
    }

    /// Spawn the crossterm event reader task. Returns immediately.
    pub fn start(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let action = match evt {
                            CrosstermEvent::Key(key) => Self::map_key(key),
                            CrosstermEvent::Paste(text) => Some(Action::PasteText(text)),
                            _ => None,
                        };
                        if let Some(action) = action {
                            if tx.send(action).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    fn map_key(key: KeyEvent) -> Option<Action> {
        // Ctrl+C always quits regardless of mode.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Action::Quit);
        }

        match key.code {
            // Non-printable keys — always mapped, reducer interprets per-mode.
            KeyCode::Down => Some(Action::ScrollDown),
            KeyCode::Up => Some(Action::ScrollUp),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::Escape),
            KeyCode::Tab => Some(Action::FocusNext),
            KeyCode::BackTab => Some(Action::FocusPrev),
            KeyCode::Backspace => Some(Action::FilterBackspace),

            // ALL printable chars go through FilterChar.
            // The app reducer interprets them based on focus + input mode.
            KeyCode::Char(c) => Some(Action::FilterChar(c)),

            _ => None,
        }
    }
}
