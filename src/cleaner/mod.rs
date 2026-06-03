//! The cleaner model and the in-memory registry of loaded cleaners.
//!
//! Ports `Cleaner.py` (the data model and `backends` registry) and drives the
//! CleanerML parser in [`cleanerml`].

pub mod cleanerml;

use std::path::Path;

use crate::actions::Action;

/// The default cleaner set, compiled into the binary by `build.rs`, so the .exe
/// runs without a `cleaners/` directory beside it. Each entry is `(filename, xml)`.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_cleaners.rs"));
}

/// One toggleable option within a cleaner (e.g. Firefox → "Cache").
#[derive(Debug, Clone)]
pub struct CleanerOption {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Shown before running when set (e.g. "you would lose saved sessions").
    pub warning: Option<String>,
    pub actions: Vec<Action>,
}

/// A cleaner loaded from one CleanerML file (e.g. Firefox, System).
#[derive(Debug, Clone)]
pub struct Cleaner {
    pub id: String,
    pub name: String,
    pub description: String,
    pub options: Vec<CleanerOption>,
}

impl Cleaner {
    /// A cleaner is usable on this OS when at least one option contributed an action.
    pub fn is_usable(&self) -> bool {
        self.options.iter().any(|o| !o.actions.is_empty())
    }

    pub fn option(&self, option_id: &str) -> Option<&CleanerOption> {
        self.options.iter().find(|o| o.id == option_id)
    }
}

/// Return whether the CleanerML `os=` attribute matches this (Windows) build.
///
/// Ports `General.os_match` for `win32` only: blank matches everything, otherwise
/// the value must be exactly `windows`.
pub fn os_match(os_str: &str) -> bool {
    os_str.is_empty() || os_str == "windows"
}

/// The set of cleaners available to preview and run (BleachBit's `backends`).
#[derive(Debug, Default)]
pub struct Backends {
    pub cleaners: Vec<Cleaner>,
}

impl Backends {
    /// The default registry: the cleaners embedded in the binary as the base,
    /// then any cleaners in `overlay_dirs` (e.g. the user's custom dir) override
    /// by id. This is what ships, so the .exe is fully self-contained.
    pub fn load_default(overlay_dirs: &[std::path::PathBuf]) -> Self {
        use std::collections::BTreeMap;
        let mut by_id: BTreeMap<String, Cleaner> = BTreeMap::new();
        for (name, xml) in embedded::EMBEDDED_CLEANERS {
            match cleanerml::parse_str(xml) {
                Ok(cleaner) if cleaner.is_usable() => {
                    by_id.insert(cleaner.id.clone(), cleaner);
                }
                Ok(_) => {}
                Err(e) => log::warn!("error in embedded cleaner {name}: {e}"),
            }
        }
        for dir in overlay_dirs {
            for cleaner in Self::load_from_dir(dir).cleaners {
                by_id.insert(cleaner.id.clone(), cleaner);
            }
        }
        Backends {
            cleaners: by_id.into_values().collect(),
        }
    }

    /// Load every `*.xml` in `dir`, keeping only those usable on Windows.
    /// Returns the loaded registry; unreadable files are logged and skipped.
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut cleaners = Vec::new();
        let mut not_usable = Vec::new();

        let mut files: Vec<_> = match std::fs::read_dir(dir) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|e| e.eq_ignore_ascii_case("xml"))
                        .unwrap_or(false)
                })
                .collect(),
            Err(e) => {
                log::error!("cannot read cleaners dir {}: {e}", dir.display());
                Vec::new()
            }
        };
        files.sort();

        for path in files {
            match cleanerml::parse_file(&path) {
                Ok(cleaner) => {
                    if cleaner.is_usable() {
                        cleaners.push(cleaner);
                    } else {
                        not_usable.push(cleaner.id);
                    }
                }
                Err(e) => log::warn!("error reading cleaner {}: {e}", path.display()),
            }
        }

        cleaners.sort_by(|a, b| a.id.cmp(&b.id));
        if !not_usable.is_empty() {
            log::debug!(
                "{} cleaners not usable on Windows: {}",
                not_usable.len(),
                not_usable.join(", ")
            );
        }
        Backends { cleaners }
    }

    pub fn get(&self, id: &str) -> Option<&Cleaner> {
        self.cleaners.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_cleaner(dir: &Path, id: &str, label: &str) {
        let xml = format!(
            r#"<cleaner id="{id}"><label>{label}</label><description>d</description>
            <option id="tmp"><label>Temp</label><description>d</description>
            <action command="delete" search="glob" path="~/x/*"/>
            </option></cleaner>"#
        );
        std::fs::write(dir.join(format!("{id}.xml")), xml).unwrap();
    }

    #[test]
    fn load_default_embeds_and_overlay_overrides() {
        // The embedded set ships real cleaners (e.g. "system" and "firefox").
        let embedded = Backends::load_default(&[]);
        assert!(embedded.get("system").is_some());
        let embedded_count = embedded.cleaners.len();
        assert!(embedded_count >= 20, "embedded set unexpectedly small");

        // A custom dir overlays: it overrides "system" by id and adds "gamma".
        let custom = std::env::temp_dir().join(format!("bbrust_overlay_{}", std::process::id()));
        std::fs::create_dir_all(&custom).unwrap();
        write_cleaner(&custom, "system", "System Custom");
        write_cleaner(&custom, "gamma", "Gamma");

        let backends = Backends::load_default(&[custom.clone()]);
        assert_eq!(backends.cleaners.len(), embedded_count + 1); // +gamma only
        assert!(backends.get("gamma").is_some());
        assert_eq!(backends.get("system").unwrap().name, "System Custom");

        let _ = std::fs::remove_dir_all(&custom);
    }
}
