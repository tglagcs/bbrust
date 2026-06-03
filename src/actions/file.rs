//! File-based actions: `delete`, `shred`, `truncate`.
//!
//! Ports `FileActionProvider` and its `Delete`/`Shred`/`Truncate` subclasses from
//! BleachBit's `Action.py`, including the searchers
//! (`file`, `glob`, `walk.all`, `walk.files`, `walk.top`) and the
//! `regex` / `nregex` / `wholeregex` / `nwholeregex` / `type` filters.

use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

use super::Command;
use crate::util::{expand_path, has_glob};

/// Which file command an action emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Delete,
    Shred,
    Truncate,
}

impl FileKind {
    pub fn from_command(command: &str) -> Option<Self> {
        match command {
            "delete" => Some(FileKind::Delete),
            "shred" => Some(FileKind::Shred),
            "truncate" => Some(FileKind::Truncate),
            _ => None,
        }
    }

    pub fn command_key(self) -> &'static str {
        match self {
            FileKind::Delete => "delete",
            FileKind::Shred => "shred",
            FileKind::Truncate => "truncate",
        }
    }
}

/// The path-finding half of a file action: a searcher plus optional filters.
///
/// Shared by [`FileAction`] and the special file actions (sqlite vacuum, json,
/// ini, office), which all locate files the same way and then do different work.
#[derive(Debug, Clone)]
pub struct Searcher {
    /// Searcher: `file`, `glob`, `walk.all`, `walk.files`, `walk.top`.
    pub search: String,
    /// Expanded raw paths (may still contain glob wildcards).
    pub paths: Vec<PathBuf>,
    /// Match against the basename; keep only matches.
    pub regex: Option<Regex>,
    /// Match against the basename; drop matches.
    pub nregex: Option<Regex>,
    /// Match against the whole path; keep only matches.
    pub wholeregex: Option<Regex>,
    /// Match against the whole path; drop matches.
    pub nwholeregex: Option<Regex>,
    /// `f` to keep only files, `d` to keep only directories.
    pub object_type: Option<char>,
}

impl Searcher {
    /// Build a [`Searcher`] from already-extracted attribute strings.
    /// Empty strings mean "absent". Invalid regexes are reported and skipped.
    pub fn new(
        search: &str,
        raw_path: &str,
        regex: &str,
        nregex: &str,
        wholeregex: &str,
        nwholeregex: &str,
        object_type: &str,
    ) -> Self {
        Searcher {
            search: search.to_string(),
            paths: expand_path(raw_path).into_iter().collect(),
            regex: compile(regex),
            nregex: compile(nregex),
            wholeregex: compile(wholeregex),
            nwholeregex: compile(nwholeregex),
            object_type: object_type.chars().next(),
        }
    }

    fn use_fast_path(&self) -> bool {
        self.object_type.is_none()
            && self.regex.is_none()
            && self.nregex.is_none()
            && self.wholeregex.is_none()
            && self.nwholeregex.is_none()
    }

    /// Resolve and filter the paths this searcher matches.
    pub fn find(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for input in &self.paths {
            self.search_one(input, &mut found);
        }
        if self.use_fast_path() {
            return found;
        }
        found.retain(|p| self.passes_filters(p));
        found
    }

    fn search_one(&self, input: &Path, out: &mut Vec<PathBuf>) {
        match self.search.as_str() {
            "file" => {
                if symlink_exists(input) {
                    out.push(input.to_path_buf());
                }
            }
            "glob" => {
                if !has_glob(&input.to_string_lossy()) {
                    log::debug!("path is not a glob pattern: {}", input.display());
                }
                glob_into(input, out);
            }
            "walk.all" => {
                for top in glob_expand(input) {
                    children_in_directory(&top, true, out);
                }
            }
            "walk.files" => {
                for top in glob_expand(input) {
                    children_in_directory(&top, false, out);
                }
            }
            "walk.top" => {
                for top in glob_expand(input) {
                    children_in_directory(&top, true, out);
                    if top.exists() {
                        out.push(top);
                    }
                }
            }
            other => {
                log::warn!("invalid search='{other}'");
            }
        }
    }

    fn passes_filters(&self, path: &Path) -> bool {
        let basename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let whole = path.to_string_lossy();

        if let Some(re) = &self.regex {
            if !re.is_match(&basename) {
                return false;
            }
        }
        if let Some(re) = &self.nregex {
            if re.is_match(&basename) {
                return false;
            }
        }
        if let Some(re) = &self.wholeregex {
            if !re.is_match(&whole) {
                return false;
            }
        }
        if let Some(re) = &self.nwholeregex {
            if re.is_match(&whole) {
                return false;
            }
        }
        match self.object_type {
            Some('f') if !path.is_file() => return false,
            Some('d') if !path.is_dir() => return false,
            _ => {}
        }
        true
    }
}

/// A `delete`/`shred`/`truncate` action: a [`Searcher`] plus which command to emit.
#[derive(Debug, Clone)]
pub struct FileAction {
    pub kind: FileKind,
    pub searcher: Searcher,
}

impl FileAction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: FileKind,
        search: &str,
        raw_path: &str,
        regex: &str,
        nregex: &str,
        wholeregex: &str,
        nwholeregex: &str,
        object_type: &str,
    ) -> Self {
        FileAction {
            kind,
            searcher: Searcher::new(
                search,
                raw_path,
                regex,
                nregex,
                wholeregex,
                nwholeregex,
                object_type,
            ),
        }
    }

    /// Expand into concrete commands.
    pub fn commands(&self) -> Vec<Command> {
        self.searcher
            .find()
            .into_iter()
            .map(|path| match self.kind {
                FileKind::Delete => Command::Delete { path, shred: false },
                FileKind::Shred => Command::Delete { path, shred: true },
                FileKind::Truncate => Command::Truncate { path },
            })
            .collect()
    }
}

/// CleanerML scans paths case-insensitively; BleachBit uses Python's `re.IGNORECASE`.
fn compile(pattern: &str) -> Option<Regex> {
    if pattern.is_empty() {
        return None;
    }
    match Regex::new(&format!("(?i){pattern}")) {
        Ok(re) => Some(re),
        Err(e) => {
            log::warn!("invalid regex '{pattern}': {e}");
            None
        }
    }
}

/// Like `os.path.lexists`: true if the path exists, even as a broken symlink.
fn symlink_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Expand a possibly-glob path into matching paths. A non-glob path is returned
/// as-is when it exists.
fn glob_expand(input: &Path) -> Vec<PathBuf> {
    let s = input.to_string_lossy();
    if has_glob(&s) {
        let mut out = Vec::new();
        glob_into(input, &mut out);
        out
    } else if input.exists() {
        vec![input.to_path_buf()]
    } else {
        Vec::new()
    }
}

fn glob_into(input: &Path, out: &mut Vec<PathBuf>) {
    // The `glob` crate treats `\` as an escape character, not a path separator,
    // so on Windows it never matches backslash patterns. Normalize to `/`, which
    // it matches correctly (and std::fs accepts on Windows).
    let pattern = input.to_string_lossy().replace('\\', "/");
    match glob::glob(&pattern) {
        Ok(paths) => {
            for entry in paths.flatten() {
                out.push(entry);
            }
        }
        Err(e) => log::warn!("bad glob '{pattern}': {e}"),
    }
}

/// Iterate files and, optionally, subdirectories inside `top` (not `top` itself).
///
/// Directories are emitted after their children so a later delete pass removes
/// them only once empty. Symlinked directories are not traversed (matching the
/// Windows junction/symlink guard in BleachBit's `children_in_directory`).
fn children_in_directory(top: &Path, list_directories: bool, out: &mut Vec<PathBuf>) {
    if !top.is_dir() {
        return;
    }
    let walker = WalkDir::new(top)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
        .filter_entry(|e| {
            // Skip the symlinked directory's contents, but the entry itself is
            // still yielded by WalkDir so it can be deleted as a link.
            !(e.file_type().is_symlink() && e.path() != top)
        });

    for entry in walker.flatten() {
        let path = entry.path();
        if path == top {
            // Never emit the top directory here; walk.top adds it separately.
            continue;
        }
        let is_dir = entry.file_type().is_dir();
        if is_dir && !list_directories {
            continue;
        }
        out.push(path.to_path_buf());
    }
}
