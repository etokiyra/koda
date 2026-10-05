//! Global user settings.
//!
//! A tiny JSON store under Koda's state directory. Koda is fully usable with no
//! settings file: a missing file loads the defaults, and a malformed file is
//! quarantined (renamed aside) rather than silently overwritten, so the defaults
//! load and the broken file is preserved for recovery. Writes are atomic, so an
//! interrupted write cannot leave a half-written file.
//!
//! The format is parsed without derive macros, matching the session and recent
//! stores, so no new dependency is needed. Unknown fields are ignored, which
//! keeps a settings file written by a newer Koda loadable.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The bundled themes. Mellow is Koda's default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    Mellow,
    Midnight,
    Daylight,
}

impl ThemeId {
    /// Every bundled theme, in the order they are offered.
    pub const ALL: [ThemeId; 3] = [ThemeId::Mellow, ThemeId::Midnight, ThemeId::Daylight];

    /// The display name shown in Settings.
    pub fn name(self) -> &'static str {
        match self {
            ThemeId::Mellow => "Mellow",
            ThemeId::Midnight => "Midnight",
            ThemeId::Daylight => "Daylight",
        }
    }

    /// The stable identifier stored in the settings file.
    pub fn slug(self) -> &'static str {
        match self {
            ThemeId::Mellow => "mellow",
            ThemeId::Midnight => "midnight",
            ThemeId::Daylight => "daylight",
        }
    }

    /// Resolve a stored slug; an unknown slug falls back to `None`.
    pub fn from_slug(slug: &str) -> Option<ThemeId> {
        ThemeId::ALL
            .iter()
            .copied()
            .find(|theme| theme.slug() == slug)
    }
}

/// The indent widths Settings offers; `None` means "infer from the file".
pub const INDENT_CHOICES: [Option<u8>; 4] = [None, Some(2), Some(4), Some(8)];

/// Koda's global preferences, with their zero-config defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: ThemeId,
    pub line_numbers: bool,
    pub soft_wrap: bool,
    /// `None` infers the indent width from each file (the zero-config default).
    pub indent_width: Option<u8>,
    pub use_spaces: bool,
    pub auto_completion: bool,
    /// Welcome and busy animations.
    pub motion: bool,
    /// Diagnostic messages shown at the end of their line.
    pub inline_diagnostics: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeId::Mellow,
            line_numbers: true,
            soft_wrap: false,
            indent_width: None,
            use_spaces: true,
            auto_completion: true,
            motion: true,
            inline_diagnostics: true,
        }
    }
}

impl Settings {
    /// Load the user's settings, or the defaults.
    pub fn load() -> Settings {
        settings_path()
            .map(|path| Settings::load_or_default_from(&path))
            .unwrap_or_default()
    }

    /// Persist the settings. Errors are non-fatal.
    pub fn save(&self) {
        if let Some(path) = settings_path() {
            let _ = self.save_to(&path);
        }
    }

    /// Load from `path`, quarantining a malformed file and returning the
    /// defaults instead of failing.
    pub fn load_or_default_from(path: &Path) -> Settings {
        match Settings::load_from(path) {
            Some(settings) => settings,
            None => {
                if path.is_file() {
                    // Preserve the broken file rather than overwriting it on the
                    // next save.
                    let _ = quarantine(path);
                }
                Settings::default()
            }
        }
    }

    /// Read settings from `path`, returning `None` when it is missing or
    /// malformed.
    pub fn load_from(path: &Path) -> Option<Settings> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(Settings::from_value(&value))
    }

    /// Write the settings to `path`, atomically.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&self.to_value()).map_err(std::io::Error::other)?;
        crate::filesystem::write_atomic(path, &text)
    }

    /// Parse settings from a JSON value, tolerating absent, unknown and invalid
    /// fields by keeping the defaults for them.
    pub fn from_value(value: &Value) -> Settings {
        let mut settings = Settings::default();
        if let Some(slug) = value.get("theme").and_then(Value::as_str)
            && let Some(theme) = ThemeId::from_slug(slug)
        {
            settings.theme = theme;
        }
        if let Some(enabled) = value.get("line_numbers").and_then(Value::as_bool) {
            settings.line_numbers = enabled;
        }
        if let Some(enabled) = value.get("soft_wrap").and_then(Value::as_bool) {
            settings.soft_wrap = enabled;
        }
        if let Some(width) = value.get("indent_width").and_then(Value::as_u64) {
            // Clamp rather than reject: an out-of-range value cannot be
            // persisted, and a sane value is better than none.
            settings.indent_width = Some(width.clamp(1, 16) as u8);
        }
        if let Some(enabled) = value.get("use_spaces").and_then(Value::as_bool) {
            settings.use_spaces = enabled;
        }
        if let Some(enabled) = value.get("auto_completion").and_then(Value::as_bool) {
            settings.auto_completion = enabled;
        }
        if let Some(enabled) = value.get("motion").and_then(Value::as_bool) {
            settings.motion = enabled;
        }
        if let Some(enabled) = value.get("inline_diagnostics").and_then(Value::as_bool) {
            settings.inline_diagnostics = enabled;
        }
        settings
    }

    /// Serialize the settings as a JSON value.
    pub fn to_value(&self) -> Value {
        json!({
            "theme": self.theme.slug(),
            "line_numbers": self.line_numbers,
            "soft_wrap": self.soft_wrap,
            "indent_width": self.indent_width,
            "use_spaces": self.use_spaces,
            "auto_completion": self.auto_completion,
            "motion": self.motion,
            "inline_diagnostics": self.inline_diagnostics,
        })
    }
}

/// Move a malformed settings file aside so it is not silently lost.
fn quarantine(path: &Path) -> std::io::Result<()> {
    let backup = path.with_extension("json.corrupt");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(path, &backup)
}

/// The settings file under Koda's state directory, if a home directory exists.
pub fn settings_path() -> Option<PathBuf> {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(state).join("koda/settings.json"));
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state/koda/settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_zero_config() {
        let settings = Settings::default();
        assert_eq!(settings.theme, ThemeId::Mellow);
        assert!(settings.line_numbers);
        assert!(!settings.soft_wrap);
        assert_eq!(settings.indent_width, None, "width is inferred by default");
        assert!(settings.use_spaces);
        assert!(settings.auto_completion);
        assert!(settings.motion);
        assert!(settings.inline_diagnostics);
    }

    #[test]
    fn every_theme_has_a_unique_slug_and_round_trips() {
        let mut slugs = std::collections::HashSet::new();
        for theme in ThemeId::ALL {
            assert!(slugs.insert(theme.slug()));
            assert_eq!(ThemeId::from_slug(theme.slug()), Some(theme));
        }
        assert!(slugs.contains("mellow"));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = scratch("roundtrip");
        let path = dir.join("settings.json");
        let settings = Settings {
            theme: ThemeId::Daylight,
            line_numbers: false,
            soft_wrap: true,
            indent_width: Some(2),
            use_spaces: false,
            auto_completion: false,
            motion: false,
            inline_diagnostics: false,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), Some(settings));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_loads_defaults() {
        let dir = scratch("missing");
        let path = dir.join("settings.json");
        assert_eq!(Settings::load_or_default_from(&path), Settings::default());
        assert_eq!(Settings::load_from(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_file_is_quarantined_and_defaults_load() {
        let dir = scratch("malformed");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        assert_eq!(Settings::load_or_default_from(&path), Settings::default());
        assert!(
            !path.exists(),
            "the broken file must not be overwritten in place"
        );
        let backup = path.with_extension("json.corrupt");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{ this is not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_values_are_tolerated() {
        let value = serde_json::json!({
            "theme": "not-a-theme",
            "indent_width": 999,
            "line_numbers": "yes",
            "future_field": { "unknown": true }
        });
        let settings = Settings::from_value(&value);
        assert_eq!(
            settings.theme,
            ThemeId::Mellow,
            "unknown theme keeps default"
        );
        assert_eq!(settings.indent_width, Some(16), "width is clamped");
        assert!(settings.line_numbers, "wrong type keeps default");
    }

    #[test]
    fn out_of_range_widths_are_clamped() {
        assert_eq!(
            Settings::from_value(&serde_json::json!({ "indent_width": 0 })).indent_width,
            Some(1)
        );
        assert_eq!(
            Settings::from_value(&serde_json::json!({ "indent_width": 4 })).indent_width,
            Some(4)
        );
    }

    #[test]
    fn saving_twice_is_clean() {
        let dir = scratch("resave");
        let path = dir.join("settings.json");
        let mut settings = Settings::default();
        settings.save_to(&path).unwrap();
        settings.theme = ThemeId::Midnight;
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path).unwrap().theme, ThemeId::Midnight);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
