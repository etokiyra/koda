//! The Settings screen and the live application of global preferences.

use super::overlay::{SettingsRow, SettingsState};
use super::*;
use crate::settings::{INDENT_CHOICES, Settings, ThemeId};

impl App {
    /// Apply the current preferences to the live theme and editor state.
    ///
    /// Called at startup and after every change, so a preference takes effect
    /// immediately rather than on the next launch.
    pub(super) fn apply_settings(&mut self) {
        crate::ui::theme::set_theme(self.settings.theme);
        self.show_line_numbers = self.settings.line_numbers;
        self.wrap = self.settings.soft_wrap;
        self.motion = self.settings.motion;
        self.inline_diagnostics = self.settings.inline_diagnostics;
        let width = self.settings.indent_width.map(usize::from);
        self.editor
            .set_indent_preference(width, self.settings.use_spaces);
    }

    /// Open the Settings screen.
    pub(super) fn open_settings(&mut self) {
        self.overlay = Overlay::Settings(SettingsState::default());
    }

    /// Write the preferences if the user changed any this session.
    ///
    /// Settings are saved on quit (like the session and recent stores), so Koda
    /// never creates a settings file for a user who changed nothing.
    pub(super) fn persist_settings(&self) {
        if self.settings_dirty {
            self.settings.save();
        }
    }

    /// Handle a key while the Settings screen is open.
    pub(super) fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.overlay = Overlay::None;
                return;
            }
            KeyCode::Up | KeyCode::BackTab => {
                if let Overlay::Settings(state) = &mut self.overlay {
                    state.move_up();
                }
                return;
            }
            KeyCode::Down | KeyCode::Tab => {
                if let Overlay::Settings(state) = &mut self.overlay {
                    state.move_down();
                }
                return;
            }
            _ => {}
        }
        let row = match &self.overlay {
            Overlay::Settings(state) => state.selected_row(),
            _ => return,
        };
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right => self.change_setting(row, 1),
            KeyCode::Left => self.change_setting(row, -1),
            _ => {}
        }
    }

    /// Change the value on `row` by `delta`, apply it, and persist it.
    fn change_setting(&mut self, row: SettingsRow, delta: i32) {
        match row {
            SettingsRow::Header(_) => {}
            SettingsRow::Theme => {
                self.settings.theme = cycle(&ThemeId::ALL, self.settings.theme, delta);
            }
            SettingsRow::LineNumbers => self.settings.line_numbers = !self.settings.line_numbers,
            SettingsRow::Animations => self.settings.motion = !self.settings.motion,
            SettingsRow::SoftWrap => self.settings.soft_wrap = !self.settings.soft_wrap,
            SettingsRow::IndentWidth => {
                self.settings.indent_width =
                    cycle(&INDENT_CHOICES, self.settings.indent_width, delta);
            }
            SettingsRow::UseSpaces => self.settings.use_spaces = !self.settings.use_spaces,
            SettingsRow::InlineDiagnostics => {
                self.settings.inline_diagnostics = !self.settings.inline_diagnostics
            }
            SettingsRow::AutoCompletion => {
                self.settings.auto_completion = !self.settings.auto_completion
            }
            SettingsRow::Reset => {
                self.settings = Settings::default();
                self.set_status("Settings reset to defaults");
            }
            SettingsRow::Back => {
                self.overlay = Overlay::None;
                return;
            }
        }
        self.apply_settings();
        self.settings_dirty = true;
    }
}

/// Pick the neighbour of `current` within `choices`, wrapping around.
fn cycle<T: Copy + PartialEq>(choices: &[T], current: T, delta: i32) -> T {
    if choices.is_empty() {
        return current;
    }
    let index = choices.iter().position(|c| *c == current).unwrap_or(0) as i32;
    let len = choices.len() as i32;
    choices[(index + delta).rem_euclid(len) as usize]
}
