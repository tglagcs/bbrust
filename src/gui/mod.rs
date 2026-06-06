//! The egui/eframe graphical interface.
//!
//! The whole point of the fork: cleaning runs on the [`crate::worker`] thread and
//! streams events over a channel, which this UI drains once per frame. The UI
//! thread therefore never blocks — no more "виснет".

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::category::{self, Category};
use crate::cleaner::Backends;
use crate::i18n::{t, tr_cleaner, Key, Lang};
use crate::util::bytes_to_human;
use crate::worker::{self, Selection, WorkerEvent};

/// Identifier of one selectable option: `(cleaner_id, option_id)`.
type OptKey = (String, String);

/// A category and the cleaners under it (display-ready, built once at startup).
struct CategoryView {
    category: Category,
    cleaners: Vec<CleanerView>,
}

/// A flattened, display-ready view of a cleaner, built once at startup so the
/// tree can be drawn without borrowing `backends` while mutating selection state.
struct CleanerView {
    id: String,
    name: String,
    options: Vec<OptView>,
}

struct OptView {
    id: String,
    name: String,
    warning: Option<String>,
}

/// One line of the action log, tagged so errors can be coloured and filtered.
struct LogLine {
    text: String,
    error: bool,
}

/// Flatten loaded cleaners into category-grouped views for rendering. Options
/// with no runnable actions and empty categories are dropped.
fn build_tree(backends: &Backends) -> Vec<CategoryView> {
    let cleaners: Vec<CleanerView> = backends
        .cleaners
        .iter()
        .filter_map(|c| {
            let options: Vec<OptView> = c
                .options
                .iter()
                .filter(|o| !o.actions.is_empty())
                .map(|o| OptView {
                    id: o.id.clone(),
                    name: o.name.clone(),
                    warning: o.warning.clone(),
                })
                .collect();
            if options.is_empty() {
                None
            } else {
                Some(CleanerView {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    options,
                })
            }
        })
        .collect();

    Category::ALL
        .into_iter()
        .filter_map(|cat| {
            let group: Vec<CleanerView> = cleaners
                .iter()
                .filter(|c| category::for_id(&c.id) == cat)
                .map(|c| CleanerView {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    options: c
                        .options
                        .iter()
                        .map(|o| OptView {
                            id: o.id.clone(),
                            name: o.name.clone(),
                            warning: o.warning.clone(),
                        })
                        .collect(),
                })
                .collect();
            if group.is_empty() {
                None
            } else {
                Some(CategoryView {
                    category: cat,
                    cleaners: group,
                })
            }
        })
        .collect()
}

pub fn run(explicit_dir: Option<PathBuf>) -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([900.0, 640.0])
        .with_min_inner_size([640.0, 420.0]);
    // Window/taskbar icon for the running app (separate from the .exe icon).
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "bbrust",
        options,
        Box::new(move |_cc| Ok(Box::new(BbApp::new(explicit_dir)))),
    )
}

struct BbApp {
    backends: Backends,
    /// Explicit `--cleaners-dir` override (rare); `None` uses the embedded set.
    explicit_dir: Option<PathBuf>,
    /// The user's custom cleaners directory (overlays the embedded set by id).
    custom_dir: PathBuf,
    /// Pre-flattened, category-grouped tree for rendering.
    tree: Vec<CategoryView>,
    lang: Lang,
    /// Set when settings change so they get saved at the end of the frame.
    dirty: bool,
    selected: BTreeSet<OptKey>,
    /// Per-option recovered size from the last preview.
    sizes: HashMap<OptKey, u64>,
    log: Vec<LogLine>,
    /// Substring filter for the log (case-insensitive); empty shows everything.
    log_filter: String,
    /// When set, only error lines are shown.
    errors_only: bool,
    status: String,
    progress: f32,
    really_delete: bool,
    confirm_clean: bool,

    // Live worker handles (None when idle).
    events: Option<Receiver<WorkerEvent>>,
    abort: Option<Arc<AtomicBool>>,
    handle: Option<JoinHandle<()>>,
}

impl BbApp {
    /// Build the registry: embedded default set as the base, plus the custom dir
    /// (and an explicit `--cleaners-dir` override if one was given).
    fn load(explicit_dir: &Option<PathBuf>, custom_dir: &Path) -> Backends {
        let mut overlay: Vec<PathBuf> = Vec::new();
        if let Some(dir) = explicit_dir {
            overlay.push(dir.clone());
        }
        overlay.push(custom_dir.to_path_buf());
        Backends::load_default(&overlay)
    }

    fn new(explicit_dir: Option<PathBuf>) -> Self {
        let custom_dir = crate::config::custom_cleaners_dir();
        let cfg = crate::config::Config::load();
        let lang = crate::config::lang_from_str(&cfg.language);

        let backends = Self::load(&explicit_dir, &custom_dir);
        let count = backends.cleaners.len();
        let tree = build_tree(&backends);

        // Restore remembered selections, keeping only ones that still exist.
        let mut selected = BTreeSet::new();
        for sel in &cfg.selected {
            if backends
                .get(&sel.cleaner)
                .and_then(|c| c.option(&sel.option))
                .is_some()
            {
                selected.insert((sel.cleaner.clone(), sel.option.clone()));
            }
        }

        BbApp {
            backends,
            explicit_dir,
            custom_dir,
            tree,
            lang,
            dirty: false,
            selected,
            sizes: HashMap::new(),
            log: Vec::new(),
            log_filter: String::new(),
            errors_only: false,
            status: format!("{count} {}", t(lang, Key::CleanersLoaded)),
            progress: 0.0,
            really_delete: false,
            confirm_clean: false,
            events: None,
            abort: None,
            handle: None,
        }
    }

    /// Reload cleaners from disk (bundled + custom) and rebuild the tree,
    /// preserving the current selection.
    fn reload(&mut self) {
        self.backends = Self::load(&self.explicit_dir, &self.custom_dir);
        self.tree = build_tree(&self.backends);
        self.selected.retain(|(c, o)| {
            self.backends
                .get(c)
                .and_then(|cl| cl.option(o))
                .is_some()
        });
    }

    /// Save language + selections to the config file.
    fn save_config(&self) {
        let cfg = crate::config::Config {
            language: crate::config::lang_to_str(self.lang).to_string(),
            selected: self
                .selected
                .iter()
                .map(|(c, o)| crate::config::Sel {
                    cleaner: c.clone(),
                    option: o.clone(),
                })
                .collect(),
        };
        cfg.save();
    }

    /// Let the user pick a CleanerML `.xml`, validate it, copy it into the custom
    /// cleaners directory, and reload. Custom cleaners can delete arbitrary files,
    /// hence the warning shown next to the button.
    fn add_custom_cleaner(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CleanerML", &["xml"])
            .pick_file()
        else {
            return;
        };

        match crate::cleaner::cleanerml::parse_file(&path) {
            Ok(cleaner) if cleaner.is_usable() => {
                if std::fs::create_dir_all(&self.custom_dir).is_err() {
                    self.status = self.tr(Key::CustomInvalid).to_string();
                    return;
                }
                let filename = path
                    .file_name()
                    .map(std::ffi::OsString::from)
                    .unwrap_or_else(|| std::ffi::OsString::from("custom.xml"));
                let dest = self.custom_dir.join(filename);
                if std::fs::copy(&path, &dest).is_ok() {
                    self.reload();
                    self.status = self.tr(Key::CustomAdded).to_string();
                } else {
                    self.status = self.tr(Key::CustomInvalid).to_string();
                }
            }
            _ => self.status = self.tr(Key::CustomInvalid).to_string(),
        }
    }

    /// Open the custom cleaners folder in Explorer.
    fn open_custom_folder(&self) {
        let _ = std::fs::create_dir_all(&self.custom_dir);
        let _ = std::process::Command::new("explorer")
            .arg(&self.custom_dir)
            .spawn();
    }

    fn running(&self) -> bool {
        self.events.is_some()
    }

    fn tr(&self, key: Key) -> &'static str {
        t(self.lang, key)
    }

    /// Begin a preview or clean of the current selection.
    fn start_run(&mut self, really_delete: bool) {
        let selections: Vec<Selection> = self
            .selected
            .iter()
            .map(|(c, o)| Selection {
                cleaner: c.clone(),
                option: o.clone(),
            })
            .collect();

        let ops = worker::collect(&self.backends, &selections);
        if ops.is_empty() {
            self.status = self.tr(Key::NothingSelected).to_string();
            return;
        }

        self.really_delete = really_delete;
        self.log.clear();
        self.progress = 0.0;
        if really_delete {
            self.sizes.clear();
        }

        let running = worker::spawn(ops, really_delete);
        self.events = Some(running.events);
        self.abort = Some(running.abort);
        self.handle = Some(running.handle);
    }

    fn request_abort(&self) {
        if let Some(flag) = &self.abort {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Drain all pending worker events; return whether the run finished.
    fn pump_events(&mut self) -> bool {
        let mut batch = Vec::new();
        let mut finished = false;
        if let Some(rx) = &self.events {
            loop {
                match rx.try_recv() {
                    Ok(ev) => batch.push(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }
        for ev in batch {
            if self.handle_event(ev) {
                finished = true;
            }
        }
        finished
    }

    /// Apply one event to the UI state. Returns true on the terminal `Done` event.
    fn handle_event(&mut self, ev: WorkerEvent) -> bool {
        match ev {
            WorkerEvent::Progress(p) => self.progress = p,
            WorkerEvent::Status(s) => self.status = s,
            WorkerEvent::Line(line) => {
                // The worker emits failures as "Error: ...". Tag them so the UI
                // can colour them red and the "errors only" filter can find them.
                let error = line.starts_with("Error");
                self.log.push(LogLine { text: line, error });
            }
            WorkerEvent::ItemSize {
                cleaner,
                option,
                size,
            } => {
                self.sizes.insert((cleaner, option), size);
            }
            WorkerEvent::Done {
                total_bytes,
                files,
                special,
                errors,
                aborted,
            } => {
                self.append_summary(total_bytes, files, special, errors, aborted);
                self.progress = 1.0;
                self.status = self.tr(Key::Ready).to_string();
                return true;
            }
        }
        false
    }

    fn append_summary(&mut self, bytes: u64, files: u64, special: u64, errors: u64, aborted: bool) {
        let l = self.lang;
        let disk_key = if self.really_delete {
            Key::DiskRecovered
        } else {
            Key::DiskToRecover
        };
        let files_key = if self.really_delete {
            Key::FilesDeleted
        } else {
            Key::FilesToDelete
        };
        let mut push = |text: String, error: bool| self.log.push(LogLine { text, error });
        push(String::new(), false);
        push(format!("{}: {}", t(l, disk_key), bytes_to_human(bytes as i64)), false);
        push(format!("{}: {files}", t(l, files_key)), false);
        if special > 0 {
            push(format!("{}: {special}", t(l, Key::SpecialOps)), false);
        }
        if errors > 0 {
            // Mark the summary error count red too, so it stands out and shows
            // under the "errors only" filter.
            push(format!("{}: {errors}", t(l, Key::Errors)), true);
        }
        if aborted {
            push(t(l, Key::Aborted).to_string(), false);
        }
    }

    fn finish(&mut self) {
        self.events = None;
        self.abort = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    /// Toggle every option on or off.
    fn select_all(&mut self, on: bool) {
        self.dirty = true;
        self.selected.clear();
        if on {
            for cat in &self.tree {
                for cleaner in &cat.cleaners {
                    for opt in &cleaner.options {
                        self.selected.insert((cleaner.id.clone(), opt.id.clone()));
                    }
                }
            }
        }
    }
}

impl eframe::App for BbApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        use eframe::egui;

        if self.running() {
            if self.pump_events() {
                self.finish();
            }
            // Keep the frame loop alive so we keep draining the channel.
            ctx.request_repaint();
        }

        let busy = self.running();

        // Top toolbar.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button(format!("🔍 {}", self.tr(Key::Preview))).clicked() {
                        self.start_run(false);
                    }
                    if ui.button(format!("🗑 {}", self.tr(Key::Clean))).clicked() {
                        self.confirm_clean = true;
                    }
                });
                if ui
                    .add_enabled(busy, egui::Button::new(format!("✖ {}", self.tr(Key::Abort))))
                    .clicked()
                {
                    self.request_abort();
                }

                ui.separator();
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button(self.tr(Key::SelectAll)).clicked() {
                        self.select_all(true);
                    }
                    if ui.button(self.tr(Key::SelectNone)).clicked() {
                        self.select_all(false);
                    }
                    ui.separator();
                    if ui
                        .button(self.tr(Key::AddCustom))
                        .on_hover_text(self.tr(Key::CustomWarning))
                        .clicked()
                    {
                        self.add_custom_cleaner();
                    }
                    if ui.button(self.tr(Key::OpenFolder)).clicked() {
                        self.open_custom_folder();
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut lang = self.lang;
                    egui::ComboBox::from_id_source("lang")
                        .selected_text(lang.name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut lang, Lang::En, Lang::En.name());
                            ui.selectable_value(&mut lang, Lang::Ru, Lang::Ru.name());
                        });
                    if lang != self.lang {
                        self.lang = lang;
                        self.dirty = true;
                    }
                    ui.label(format!("{}:", self.tr(Key::Language)));
                });
            });
            // Mini warning that custom cleaners are powerful.
            ui.horizontal(|ui| {
                ui.weak(format!("⚠ {}", self.tr(Key::CustomWarning)));
            });
            ui.add_space(4.0);
        });

        // Bottom: status + progress.
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.add(egui::ProgressBar::new(self.progress).show_percentage());
            ui.label(&self.status);
            ui.add_space(2.0);
        });

        // Left: cleaner tree with checkboxes.
        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| self.draw_tree(ui));
                });
            });

        // Center: the action log, with a filter bar above it.
        egui::CentralPanel::default().show(ctx, |ui| {
            let filter_label = self.tr(Key::Filter);
            let errors_only_label = self.tr(Key::ErrorsOnly);
            ui.horizontal(|ui| {
                ui.label(format!("{filter_label}:"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.log_filter)
                        .desired_width(220.0)
                        .hint_text(filter_label),
                );
                if ui.button("✖").clicked() {
                    self.log_filter.clear();
                }
                ui.checkbox(&mut self.errors_only, errors_only_label);
            });
            ui.separator();
            self.draw_log(ui);
        });

        // Confirmation modal for real deletion.
        if self.confirm_clean {
            self.draw_confirm(ctx);
        }

        // Persist settings if anything changed this frame.
        if self.dirty {
            self.save_config();
            self.dirty = false;
        }
    }
}

impl BbApp {
    fn draw_tree(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        // Split the borrow so the rendering closures can mutate `selected` without
        // conflicting with the immutable iteration over `tree`.
        let BbApp {
            tree,
            selected,
            sizes,
            lang,
            dirty,
            ..
        } = self;
        let lang = *lang;

        for cat in tree.iter() {
            egui::CollapsingHeader::new(
                egui::RichText::new(cat.category.label(lang)).strong(),
            )
            .id_source(("cat", cat.category as u8))
            .default_open(true)
            .show(ui, |ui| {
                for cleaner in &cat.cleaners {
                    // Custom header: a checkbox that toggles every option of this
                    // cleaner, plus the collapse arrow. Indeterminate when only
                    // some options are selected.
                    let header_id = ui.make_persistent_id(("cleaner", &cleaner.id));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        header_id,
                        false,
                    )
                    .show_header(ui, |ui| {
                        let total = cleaner.options.len();
                        let sel_count = cleaner
                            .options
                            .iter()
                            .filter(|o| {
                                selected.contains(&(cleaner.id.clone(), o.id.clone()))
                            })
                            .count();
                        let mut all = total > 0 && sel_count == total;
                        let partial = sel_count > 0 && sel_count < total;
                        let cb = egui::Checkbox::new(&mut all, tr_cleaner(lang, &cleaner.name))
                            .indeterminate(partial);
                        if ui.add(cb).changed() {
                            for o in &cleaner.options {
                                let key = (cleaner.id.clone(), o.id.clone());
                                if all {
                                    selected.insert(key);
                                } else {
                                    selected.remove(&key);
                                }
                            }
                            *dirty = true;
                        }
                    })
                    .body(|ui| {
                        for opt in &cleaner.options {
                            let key = (cleaner.id.clone(), opt.id.clone());
                            let mut on = selected.contains(&key);
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut on, tr_cleaner(lang, &opt.name)).changed() {
                                    if on {
                                        selected.insert(key.clone());
                                    } else {
                                        selected.remove(&key);
                                    }
                                    *dirty = true;
                                }
                                if let Some(size) = sizes.get(&key) {
                                    if *size > 0 {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.weak(bytes_to_human(*size as i64));
                                            },
                                        );
                                    }
                                }
                            });
                            if let Some(w) = &opt.warning {
                                ui.indent(&opt.id, |ui| {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(200, 120, 0),
                                        tr_cleaner(lang, w),
                                    );
                                });
                            }
                        }
                    });
                }
            });
        }
    }

    /// Render the action log: red error lines, filtered by substring and/or the
    /// "errors only" toggle. Rows are virtualised so a huge clean (thousands of
    /// deleted files) stays smooth.
    fn draw_log(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;

        let needle = self.log_filter.to_lowercase();
        let visible: Vec<&LogLine> = self
            .log
            .iter()
            .filter(|l| !self.errors_only || l.error)
            .filter(|l| needle.is_empty() || l.text.to_lowercase().contains(&needle))
            .collect();

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let error_color = egui::Color32::from_rgb(220, 80, 80);
        // Only stick to the bottom when viewing the full, unfiltered log — while
        // filtering the user is inspecting, so don't yank the scroll position.
        let stick = needle.is_empty() && !self.errors_only;

        // `both()` adds a horizontal scrollbar for long paths; rows stay
        // vertically virtualised via show_rows.
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .stick_to_bottom(stick)
            .show_rows(ui, row_height, visible.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for line in &visible[range] {
                    let mut text = egui::RichText::new(&line.text).monospace();
                    if line.error {
                        text = text.color(error_color);
                    }
                    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
                }
            });
    }

    fn draw_confirm(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;
        egui::Window::new(self.tr(Key::ConfirmTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(self.tr(Key::ConfirmBody));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(self.tr(Key::Cancel)).clicked() {
                        self.confirm_clean = false;
                    }
                    let del = egui::Button::new(self.tr(Key::Yes))
                        .fill(egui::Color32::from_rgb(150, 40, 40));
                    if ui.add(del).clicked() {
                        self.confirm_clean = false;
                        self.start_run(true);
                    }
                });
            });
    }
}
