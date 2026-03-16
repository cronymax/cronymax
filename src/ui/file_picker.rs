//! `#`-triggered inline file picker with fuzzy matching (nucleo).
//!
//! When the user types `#` in the prompt editor, a popup appears above the
//! cursor showing files in the current working directory. Characters typed
//! after `#` filter entries via fuzzy matching. Selecting an entry inserts
//! the relative path into the prompt text.
//!
//! Uses the high-level `Nucleo` async matcher (same engine as Helix editor):
//! - File collection runs on a background thread via `Injector`
//! - Matching runs in parallel on nucleo's internal thread pool
//! - UI thread only calls `tick()` + reads the snapshot — never blocks

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo, Utf32String};

// ─── Public State ────────────────────────────────────────────────────────────

/// Per-session file picker state.
pub struct FilePickerState {
    /// Whether the popup is currently visible.
    pub active: bool,
    /// The fuzzy query (characters after `#`).
    pub query: String,
    /// Currently selected index in the match list.
    pub selected: usize,
    /// Path selected by a click in the popup (consumed by caller).
    pub picked_path: Option<String>,
    /// Cached match results (path + score).
    matches: Vec<FileMatch>,
    /// The async nucleo matcher — owns the background thread pool.
    nucleo: Option<Nucleo<String>>,
    /// The CWD that was used to populate the file list.
    populated_cwd: Option<PathBuf>,
    /// Previous query text — used for `append` optimisation in `reparse`.
    prev_query: String,
}

#[derive(Debug, Clone)]
pub struct FileMatch {
    /// Display path (relative to CWD).
    pub path: String,
}

impl Default for FilePickerState {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for FilePickerState {
    fn clone(&self) -> Self {
        // Nucleo is not Clone — a cloned state starts without a matcher.
        // The matcher will be re-created on the next `activate()`.
        Self {
            active: self.active,
            query: self.query.clone(),
            selected: self.selected,
            picked_path: None,
            matches: self.matches.clone(),
            nucleo: None,
            populated_cwd: self.populated_cwd.clone(),
            prev_query: self.prev_query.clone(),
        }
    }
}

impl std::fmt::Debug for FilePickerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePickerState")
            .field("active", &self.active)
            .field("query", &self.query)
            .field("selected", &self.selected)
            .field("matches_count", &self.matches.len())
            .finish()
    }
}

impl FilePickerState {
    /// Maximum visible rows in the file picker popup.
    pub const MAX_VISIBLE_ROWS: usize = 12;

    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            selected: 0,
            picked_path: None,
            matches: Vec::new(),
            nucleo: None,
            populated_cwd: None,
            prev_query: String::new(),
        }
    }

    /// Activate the file picker, populating the file list if needed.
    /// File collection runs on a background thread — the UI never blocks.
    pub fn activate(&mut self, cwd: &Path) {
        self.active = true;
        self.query.clear();
        self.prev_query.clear();
        self.selected = 0;

        // Re-populate if CWD changed or first time.
        if self.populated_cwd.as_deref() != Some(cwd) {
            self.populated_cwd = Some(cwd.to_path_buf());

            // Create the high-level Nucleo matcher (1 column for path).
            let config = Config::DEFAULT.match_paths();
            let nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);

            // Grab an injector and spawn file walking on a background thread.
            let injector = nucleo.injector();
            let root = cwd.to_path_buf();
            std::thread::Builder::new()
                .name("file-picker-walk".into())
                .spawn(move || {
                    collect_files_async(&root, &injector);
                })
                .ok();

            self.nucleo = Some(nucleo);
        } else if let Some(nucleo) = &mut self.nucleo {
            // Same CWD — clear the pattern to show all files.
            nucleo
                .pattern
                .reparse(0, "", CaseMatching::Smart, Normalization::Smart, false);
        }
    }

    /// Deactivate and reset.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.query.clear();
        self.selected = 0;
        self.picked_path = None;
        self.matches.clear();
    }

    /// Update the fuzzy query and refresh matches.
    pub fn set_query(&mut self, query: &str) {
        let append = query.starts_with(&self.prev_query) && !self.prev_query.is_empty();
        self.query = query.to_string();
        self.selected = 0;

        if let Some(nucleo) = &mut self.nucleo {
            nucleo
                .pattern
                .reparse(0, query, CaseMatching::Smart, Normalization::Smart, append);
        }
        self.refresh_matches();
        self.prev_query = query.to_string();
    }

    /// Get the current matches.
    pub fn current_matches(&self) -> &[FileMatch] {
        &self.matches
    }

    /// Number of current matches.
    pub fn matches_count(&self) -> usize {
        self.matches.len()
    }

    /// Navigate selection up.
    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
        }
    }

    /// Navigate selection down.
    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }

    /// Get the currently selected path, if any.
    pub fn selected_path(&self) -> Option<&str> {
        self.matches.get(self.selected).map(|m| m.path.as_str())
    }

    fn refresh_matches(&mut self) {
        let Some(nucleo) = &mut self.nucleo else {
            return;
        };

        // Let nucleo process in the background; wait up to 10ms.
        nucleo.tick(10);

        let snap = nucleo.snapshot();
        let count = snap.matched_item_count().min(20);

        // Snapshot already returns items sorted by score (best first).
        self.matches = snap
            .matched_items(0..count)
            .map(|item| FileMatch {
                path: item.data.clone(),
            })
            .collect();

        if self.selected >= self.matches.len() {
            self.selected = 0;
        }
    }
}

/// Walk the directory tree from `root`, respecting `.gitignore`, and inject
/// paths into the nucleo matcher. Runs on a dedicated background thread so
/// the UI never blocks on I/O.
fn collect_files_async(root: &Path, injector: &nucleo::Injector<String>) {
    use ignore::WalkBuilder;

    let max_files = 50_000u32;

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(16))
        .threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)))
        .build_parallel();

    let root = root.to_path_buf();
    let injector = injector.clone();
    let counter = std::sync::atomic::AtomicU32::new(0);

    walker.run(|| {
        let root = root.clone();
        let injector = injector.clone();
        let counter = &counter;
        Box::new(move |entry| {
            if counter.load(std::sync::atomic::Ordering::Relaxed) >= max_files {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return ignore::WalkState::Continue;
            };
            // Skip directories.
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return ignore::WalkState::Continue;
            }
            if let Ok(rel) = entry.path().strip_prefix(&root)
                && let Some(s) = rel.to_str()
                && !s.is_empty()
            {
                let path = s.to_string();
                injector.push(path, |data, cols| {
                    cols[0] = Utf32String::from(data.as_str());
                });
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            ignore::WalkState::Continue
        })
    });
}
