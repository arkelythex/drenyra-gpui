//! Operator settings and their on-disk file, GPUI-free.
//!
//! The file is a tiny `key = "value"` text file at
//! `$XDG_CONFIG_HOME/drenyra-gpui/settings.toml` (falling back to `~/.config/...`). Reading is
//! forgiving: a missing file, an unknown key or an unknown theme falls back to the defaults, so
//! a hand-edited or newer file never stops the window from opening.

use std::io;
use std::path::Path;

use super::theme::ThemeId;

/// Persisted operator preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub theme: ThemeId,
}

impl Settings {
    /// Parses the settings file text. Unknown keys and invalid values keep their defaults.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            if key.trim() == "theme" {
                if let Some(theme) = ThemeId::from_slug(value) {
                    settings.theme = theme;
                }
            }
        }
        settings
    }

    /// Serializes to the settings file text read back by [`Self::parse`].
    pub fn to_file_text(self) -> String {
        format!(
            "# Drenyra GPUI settings. Themes: dreamcoder-dark, dreamcoder-light.\ntheme = \"{}\"\n",
            self.theme.slug()
        )
    }

    /// Loads settings from `path`; a missing or unreadable file yields the defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Writes settings to `path`, creating its directory.
    pub fn save(self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_file_text())
    }
}

/// Saves `now` to `path` when it differs from `before`.
///
/// Returns `Ok(true)` when the file was written, `Ok(false)` when nothing changed or there is no
/// settings location, and `Err` with an operator-facing message when the write failed.
pub fn save_if_changed(
    before: Settings,
    now: Settings,
    path: Option<&Path>,
) -> Result<bool, String> {
    let Some(path) = path.filter(|_| now != before) else {
        return Ok(false);
    };
    now.save(path)
        .map(|()| true)
        .map_err(|err| format!("Settings not saved to {}: {err}", path.display()))
}

/// Default settings file location, or `None` when neither `XDG_CONFIG_HOME` nor `HOME` is set.
#[cfg(feature = "gpui")]
pub fn default_settings_path() -> Option<std::path::PathBuf> {
    settings_path_from(
        std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
        std::env::var_os("HOME").map(std::path::PathBuf::from),
    )
}

#[cfg(any(test, feature = "gpui"))]
fn settings_path_from(
    xdg_config_home: Option<std::path::PathBuf>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let base = xdg_config_home
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))?;
    Some(base.join("drenyra-gpui").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_reads_the_theme_and_ignores_comments_unknown_keys_and_bad_values() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(
            Settings::parse("# c\nfont = \"x\"\ntheme = \"dreamcoder-light\"\n").theme,
            ThemeId::DreamcoderLight
        );
        assert_eq!(
            Settings::parse("theme=dreamcoder-light").theme,
            ThemeId::DreamcoderLight
        );
        assert_eq!(
            Settings::parse("theme = \"solarized\"").theme,
            ThemeId::DreamcoderDark
        );
        assert_eq!(Settings::parse("not a pair").theme, ThemeId::DreamcoderDark);
    }

    #[test]
    fn file_text_round_trips_for_every_theme() {
        for theme in ThemeId::ALL {
            let settings = Settings { theme };
            assert_eq!(Settings::parse(&settings.to_file_text()), settings);
        }
    }

    #[test]
    fn save_creates_the_directory_and_load_reads_it_back() {
        let dir = std::env::temp_dir().join(format!("drenyra-settings-{}", std::process::id()));
        let path = dir.join("nested").join("settings.toml");
        assert_eq!(Settings::load(&path), Settings::default(), "missing file");

        let light = Settings {
            theme: ThemeId::DreamcoderLight,
        };
        light.save(&path).expect("save");
        assert_eq!(Settings::load(&path), light);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn save_if_changed_writes_only_real_changes_and_reports_failures() {
        let dir = std::env::temp_dir().join(format!("drenyra-save-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let dark = Settings::default();
        let light = Settings {
            theme: ThemeId::DreamcoderLight,
        };

        assert_eq!(save_if_changed(dark, dark, Some(&path)), Ok(false));
        assert!(!path.exists(), "unchanged settings must not touch the disk");
        assert_eq!(save_if_changed(dark, light, None), Ok(false));

        assert_eq!(save_if_changed(dark, light, Some(&path)), Ok(true));
        assert_eq!(Settings::load(&path), light);

        // A regular file where the parent directory should be makes the write fail.
        let blocked = path.join("settings.toml");
        let err = save_if_changed(light, dark, Some(&blocked)).expect_err("must fail");
        assert!(err.starts_with("Settings not saved to "), "{err}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn settings_path_prefers_absolute_xdg_config_home_then_home() {
        let file = |p: &str| Some(PathBuf::from(p).join("drenyra-gpui/settings.toml"));
        assert_eq!(
            settings_path_from(Some("/x/cfg".into()), Some("/home/u".into())),
            file("/x/cfg")
        );
        assert_eq!(
            settings_path_from(Some("relative".into()), Some("/home/u".into())),
            file("/home/u/.config")
        );
        assert_eq!(
            settings_path_from(None, Some("/home/u".into())),
            file("/home/u/.config")
        );
        assert_eq!(settings_path_from(None, None), None);
    }
}
