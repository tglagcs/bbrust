//! Persistent settings: chosen language and selected options.
//!
//! Stored as TOML at `%APPDATA%\bbrust\config.toml`. Custom (user-supplied)
//! cleaners live next to it in `%APPDATA%\bbrust\cleaners\`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

/// One remembered selection (`cleaner_id`, `option_id`).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Sel {
    pub cleaner: String,
    pub option: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    /// `"en"` or `"ru"`.
    #[serde(default)]
    pub language: String,
    /// Remembered checkbox selections.
    #[serde(default)]
    pub selected: Vec<Sel>,
}

/// `%APPDATA%\bbrust` (falls back to the current directory if APPDATA is unset).
pub fn config_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("bbrust")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Directory scanned for user-supplied CleanerML files (in addition to the
/// bundled `cleaners/`).
pub fn custom_cleaners_dir() -> PathBuf {
    config_dir().join("cleaners")
}

impl Config {
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        if std::fs::create_dir_all(config_dir()).is_err() {
            return;
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), text);
        }
    }
}

pub fn lang_to_str(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Ru => "ru",
    }
}

pub fn lang_from_str(s: &str) -> Lang {
    if s == "ru" {
        Lang::Ru
    } else {
        Lang::En
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = Config {
            language: "ru".into(),
            selected: vec![
                Sel {
                    cleaner: "firefox".into(),
                    option: "cache".into(),
                },
                Sel {
                    cleaner: "adobe_reader".into(),
                    option: "tmp".into(),
                },
            ],
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.language, "ru");
        assert_eq!(back.selected.len(), 2);
        assert_eq!(back.selected[0].cleaner, "firefox");
        assert_eq!(lang_from_str(&back.language), Lang::Ru);
    }
}
