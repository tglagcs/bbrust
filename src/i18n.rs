//! Minimal two-language string tables (English + Russian).
//!
//! Deliberately tiny — no gettext, no `.po` files. The fork ships exactly two
//! languages, switchable in the GUI.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }
}

/// Translation keys. Using an enum keeps lookups exhaustive and typo-proof.
#[derive(Clone, Copy)]
pub enum Key {
    Preview,
    Clean,
    Abort,
    SelectAll,
    SelectNone,
    Language,
    Ready,
    NothingSelected,
    ConfirmTitle,
    ConfirmBody,
    Yes,
    Cancel,
    DiskRecovered,
    DiskToRecover,
    FilesDeleted,
    FilesToDelete,
    SpecialOps,
    Errors,
    Aborted,
    CleanersLoaded,
    AddCustom,
    OpenFolder,
    CustomWarning,
    CustomAdded,
    CustomInvalid,
}

/// Lazily-built lookup for dynamic, cleaner-supplied strings.
fn ru_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| crate::translations_ru::RU_PAIRS.iter().copied().collect())
}

/// Translate a cleaner/option/warning string that came from the XML (not a fixed
/// UI string). Returns the Russian text when `lang` is Russian and a translation
/// exists; otherwise the original English `en`.
pub fn tr_cleaner(lang: Lang, en: &str) -> &str {
    if lang == Lang::Ru {
        if let Some(ru) = ru_map().get(en) {
            return ru;
        }
    }
    en
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaner_names_translate_to_russian() {
        // A common option label that exists in the generated table.
        assert_eq!(tr_cleaner(Lang::Ru, "Cache"), "Кэш");
        // English is returned unchanged.
        assert_eq!(tr_cleaner(Lang::En, "Cache"), "Cache");
        // Unknown strings fall back to the original.
        assert_eq!(tr_cleaner(Lang::Ru, "Nonexistent xyz"), "Nonexistent xyz");
    }
}

pub fn t(lang: Lang, key: Key) -> &'static str {
    use Key::*;
    use Lang::*;
    match (lang, key) {
        (En, Preview) => "Preview",
        (Ru, Preview) => "Просмотр",
        (En, Clean) => "Clean",
        (Ru, Clean) => "Очистить",
        (En, Abort) => "Abort",
        (Ru, Abort) => "Прервать",
        (En, SelectAll) => "Select all",
        (Ru, SelectAll) => "Выбрать все",
        (En, SelectNone) => "Select none",
        (Ru, SelectNone) => "Снять выбор",
        (En, Language) => "Language",
        (Ru, Language) => "Язык",
        (En, Ready) => "Ready.",
        (Ru, Ready) => "Готово.",
        (En, NothingSelected) => "Nothing selected.",
        (Ru, NothingSelected) => "Ничего не выбрано.",
        (En, ConfirmTitle) => "Confirm cleaning",
        (Ru, ConfirmTitle) => "Подтвердите очистку",
        (En, ConfirmBody) => {
            "Permanently delete the selected items? This cannot be undone."
        }
        (Ru, ConfirmBody) => "Безвозвратно удалить выбранное? Это действие необратимо.",
        (En, Yes) => "Delete",
        (Ru, Yes) => "Удалить",
        (En, Cancel) => "Cancel",
        (Ru, Cancel) => "Отмена",
        (En, DiskRecovered) => "Disk space recovered",
        (Ru, DiskRecovered) => "Освобождено места",
        (En, DiskToRecover) => "Disk space to be recovered",
        (Ru, DiskToRecover) => "Будет освобождено",
        (En, FilesDeleted) => "Files deleted",
        (Ru, FilesDeleted) => "Удалено файлов",
        (En, FilesToDelete) => "Files to be deleted",
        (Ru, FilesToDelete) => "Будет удалено файлов",
        (En, SpecialOps) => "Special operations",
        (Ru, SpecialOps) => "Особых операций",
        (En, Errors) => "Errors",
        (Ru, Errors) => "Ошибок",
        (En, Aborted) => "(aborted)",
        (Ru, Aborted) => "(прервано)",
        (En, CleanersLoaded) => "cleaners loaded",
        (Ru, CleanersLoaded) => "чистильщиков загружено",
        (En, AddCustom) => "➕ Custom cleaner…",
        (Ru, AddCustom) => "➕ Свой чистильщик…",
        (En, OpenFolder) => "📁 Folder",
        (Ru, OpenFolder) => "📁 Папка",
        (En, CustomWarning) => {
            "Custom cleaners can delete any files. Add only ones you trust."
        }
        (Ru, CustomWarning) => {
            "Свои чистильщики могут удалить любые файлы. Добавляйте только проверенные."
        }
        (En, CustomAdded) => "Custom cleaner added.",
        (Ru, CustomAdded) => "Свой чистильщик добавлен.",
        (En, CustomInvalid) => "Invalid CleanerML file.",
        (Ru, CustomInvalid) => "Неверный файл CleanerML.",
    }
}
