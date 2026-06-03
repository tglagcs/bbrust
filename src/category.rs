//! Grouping of cleaners into user-facing categories.
//!
//! CleanerML has no notion of categories, so we map cleaner ids to a small fixed
//! set here. Anything unmapped falls into [`Category::Other`].

use crate::i18n::Lang;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Browsers,
    Communication,
    Office,
    Files,
    Media,
    Development,
    System,
    Other,
}

impl Category {
    /// Display order, top to bottom.
    pub const ALL: [Category; 8] = [
        Category::Browsers,
        Category::Communication,
        Category::Office,
        Category::Files,
        Category::Media,
        Category::Development,
        Category::System,
        Category::Other,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match (lang, self) {
            (Lang::En, Category::Browsers) => "Browsers",
            (Lang::Ru, Category::Browsers) => "Браузеры",
            (Lang::En, Category::Communication) => "Communication",
            (Lang::Ru, Category::Communication) => "Общение",
            (Lang::En, Category::Office) => "Office & Documents",
            (Lang::Ru, Category::Office) => "Офис и документы",
            (Lang::En, Category::Files) => "Files & Archives",
            (Lang::Ru, Category::Files) => "Файлы и архивы",
            (Lang::En, Category::Media) => "Media & Graphics",
            (Lang::Ru, Category::Media) => "Медиа и графика",
            (Lang::En, Category::Development) => "Development",
            (Lang::Ru, Category::Development) => "Разработка",
            (Lang::En, Category::System) => "Windows System",
            (Lang::Ru, Category::System) => "Система Windows",
            (Lang::En, Category::Other) => "Other",
            (Lang::Ru, Category::Other) => "Прочее",
        }
    }
}

/// Map a cleaner id to its category.
pub fn for_id(id: &str) -> Category {
    use Category::*;
    match id {
        "brave" | "chromium" | "firefox" | "google_chrome" | "microsoft_edge" | "opera"
        | "vivaldi" | "librewolf" | "zen" => Browsers,

        "discord" | "slack" | "zoom" | "teamviewer" | "thunderbird" | "telegram" => Communication,

        "microsoft_office" | "libreoffice" | "adobe_reader" | "onlyoffice" => Office,

        "filezilla" | "smartftp" | "winrar" | "winzip" => Files,

        "vlc" | "gimp" => Media,

        "java" | "vim" | "vscode" | "claude" => Development,

        "windows_explorer" | "windows_defender" | "windows_media_player" | "paint" | "system" => {
            System
        }

        _ => Other,
    }
}
