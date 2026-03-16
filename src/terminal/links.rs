//! Link detection in terminal grid output.
//!
//! Detects URLs (via `linkify`) and file paths (via `regex`) in terminal rows.
//! Used for Cmd/Ctrl+hover highlighting and Cmd/Ctrl+click navigation.

use alacritty_terminal::term::Term;
use linkify::{LinkFinder, LinkKind};
use regex::Regex;
use std::sync::LazyLock;

use crate::terminal::state::EventProxy;

/// A detected link in the terminal grid.
#[derive(Debug, Clone)]
pub struct DetectedLink {
    /// Row in the visible grid (0-based from display top).
    pub row: usize,
    /// Start column (inclusive, 0-based).
    pub start_col: usize,
    /// End column (exclusive).
    pub end_col: usize,
    /// The resolved URL or file path string.
    pub url: String,
    /// Whether this is a file path (vs HTTP URL).
    pub is_path: bool,
}

/// Regex for detecting file paths in terminal output.
/// Matches absolute paths, home-relative paths, and dot-relative paths.
/// Uses a capturing group to handle word-boundary detection (lookbehind not supported).
static FILE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|\s)((?:/[\w.+\-/@]+)|(?:~/[\w.+\-/@]+)|(?:\./[\w.+\-/@]+)|(?:\.\./[\w.+\-/@]+))",
    )
    .expect("file path regex must compile")
});

/// Extract the text of a single visible row from the terminal grid.
fn row_text(term: &Term<EventProxy>, row: usize, cols: usize) -> String {
    let grid = term.grid();
    let line = alacritty_terminal::index::Line(row as i32);
    let mut text = String::with_capacity(cols);
    for col_idx in 0..cols {
        let col = alacritty_terminal::index::Column(col_idx);
        let c = grid[line][col].c;
        if c.is_control() || c == '\0' {
            text.push(' ');
        } else {
            text.push(c);
        }
    }
    text
}

/// Scan a single terminal row for URLs and file paths.
///
/// Returns all detected links ordered by column position.
/// URL links come first; file paths that overlap with URLs are excluded.
pub fn detect_links_on_row(term: &Term<EventProxy>, row: usize, cols: usize) -> Vec<DetectedLink> {
    let text = row_text(term, row, cols);
    let mut links: Vec<DetectedLink> = Vec::new();

    // 1. Detect URLs using linkify.
    let finder = LinkFinder::new();
    for link in finder.links(&text) {
        // Map byte offsets to column indices.
        // Since terminal cells are 1 char each, we need char-based offsets.
        let start_byte = link.start();
        let end_byte = link.end();
        let start_col = text[..start_byte].chars().count();
        let end_col = text[..end_byte].chars().count();

        if start_col < end_col {
            let url = match link.kind() {
                LinkKind::Url => link.as_str().to_string(),
                LinkKind::Email => format!("mailto:{}", link.as_str()),
                _ => link.as_str().to_string(),
            };
            links.push(DetectedLink {
                row,
                start_col,
                end_col,
                url,
                is_path: false,
            });
        }
    }

    // 2. Detect file paths using regex.
    for caps in FILE_PATH_RE.captures_iter(&text) {
        let path_match = caps.get(1).unwrap();
        let start_byte = path_match.start();
        let end_byte = path_match.end();
        let start_col = text[..start_byte].chars().count();
        let end_col = text[..end_byte].chars().count();

        // Skip file paths that overlap with already-detected URL links.
        let overlaps = links
            .iter()
            .any(|l| start_col < l.end_col && end_col > l.start_col);
        if overlaps {
            continue;
        }

        if start_col < end_col {
            links.push(DetectedLink {
                row,
                start_col,
                end_col,
                url: path_match.as_str().to_string(),
                is_path: true,
            });
        }
    }

    // Sort by column position.
    links.sort_by_key(|l| l.start_col);
    links
}

/// Find the link (if any) at a specific cell position.
pub fn link_at(
    term: &Term<EventProxy>,
    col: usize,
    row: usize,
    cols: usize,
) -> Option<DetectedLink> {
    let links = detect_links_on_row(term, row, cols);
    links
        .into_iter()
        .find(|l| col >= l.start_col && col < l.end_col)
}

/// Resolve a detected file path to an absolute path string.
/// Expands `~` to home directory, and resolves `./` and `../` relative to cwd.
pub fn resolve_path(path: &str) -> String {
    if path.starts_with('~')
        && let Some(home) = dirs::home_dir()
    {
        return home.join(&path[2..]).to_string_lossy().to_string();
    }
    // For ./ and ../ paths, resolve relative to current directory.
    if (path.starts_with("./") || path.starts_with("../"))
        && let Ok(cwd) = std::env::current_dir()
    {
        return cwd.join(path).to_string_lossy().to_string();
    }
    // Already absolute or unresolvable — return as-is.
    path.to_string()
}
