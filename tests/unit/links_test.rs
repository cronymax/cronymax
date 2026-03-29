//! Unit tests for link detection in the terminal grid.

use cronymax::renderer::terminal::links::{
    DetectedLink, detect_links_on_row, link_at, resolve_path,
};
use cronymax::renderer::terminal::state::TermState;

/// Create a TermState with the given dimensions and write text into it.
fn make_term(cols: usize, rows: usize, text: &str) -> TermState {
    let mut state = TermState::new(cols, rows, 0);
    state.advance(text.as_bytes());
    state
}

// ── detect_links_on_row ──────────────────────────────────────────────────────

#[test]
fn detect_http_url() {
    let ts = make_term(80, 4, "Visit https://example.com for more info\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com");
    assert!(!links[0].is_path);
    assert_eq!(links[0].start_col, 6);
    assert_eq!(links[0].end_col, 25);
}

#[test]
fn detect_http_url_with_path() {
    let ts = make_term(80, 4, "See https://docs.rs/egui/latest/egui/ ok\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert!(links[0].url.starts_with("https://docs.rs"));
    assert!(!links[0].is_path);
}

#[test]
fn detect_absolute_path() {
    let ts = make_term(80, 4, "open /usr/local/bin/fish\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "/usr/local/bin/fish");
    assert!(links[0].is_path);
}

#[test]
fn detect_home_path() {
    let ts = make_term(80, 4, "edit ~/Documents/notes.md\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "~/Documents/notes.md");
    assert!(links[0].is_path);
}

#[test]
fn detect_relative_dot_path() {
    let ts = make_term(80, 4, "cat ./src/main.rs\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "./src/main.rs");
    assert!(links[0].is_path);
}

#[test]
fn detect_relative_dotdot_path() {
    let ts = make_term(80, 4, "look at ../README.md\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "../README.md");
    assert!(links[0].is_path);
}

#[test]
fn url_and_path_on_same_row() {
    let ts = make_term(120, 4, "https://example.com and /usr/bin/env\r\n");
    let links = detect_links_on_row(ts.term(), 0, 120);
    assert_eq!(links.len(), 2);
    // Links should be sorted by start_col.
    assert!(!links[0].is_path); // URL comes first at col 0
    assert!(links[1].is_path); // Path comes second
}

#[test]
fn overlapping_file_path_excluded() {
    // If a URL like https://example.com/path contains what looks like a path,
    // the regex match on /path should be excluded since it overlaps the URL.
    let ts = make_term(80, 4, "visit https://example.com/some/path here\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    // Should only have the URL, not a duplicate file path from the URL's path component.
    assert_eq!(links.len(), 1);
    assert!(!links[0].is_path);
}

#[test]
fn empty_row_returns_no_links() {
    let ts = make_term(80, 4, "");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert!(links.is_empty());
}

#[test]
fn row_with_no_links() {
    let ts = make_term(80, 4, "this is just plain text with no links\r\n");
    let links = detect_links_on_row(ts.term(), 0, 80);
    assert!(links.is_empty());
}

// ── link_at ──────────────────────────────────────────────────────────────────

#[test]
fn link_at_inside_url() {
    let ts = make_term(80, 4, "Go to https://example.com now\r\n");
    // "https://example.com" starts at col 6, ends at col 25.
    let result = link_at(ts.term(), 10, 0, 80);
    assert!(result.is_some());
    assert_eq!(result.unwrap().url, "https://example.com");
}

#[test]
fn link_at_outside_url() {
    let ts = make_term(80, 4, "Go to https://example.com now\r\n");
    // col 0 is "G" — not inside any link.
    let result = link_at(ts.term(), 0, 0, 80);
    assert!(result.is_none());
}

#[test]
fn link_at_boundary_start() {
    let ts = make_term(80, 4, "Go to https://example.com now\r\n");
    // Col 6 should be the first char of the URL.
    let result = link_at(ts.term(), 6, 0, 80);
    assert!(result.is_some());
}

#[test]
fn link_at_boundary_end() {
    let ts = make_term(80, 4, "Go to https://example.com now\r\n");
    // Col 25 should be just past the URL (exclusive end).
    let result = link_at(ts.term(), 25, 0, 80);
    assert!(result.is_none());
}

// ── resolve_path ─────────────────────────────────────────────────────────────

#[test]
fn resolve_absolute_path_unchanged() {
    assert_eq!(resolve_path("/usr/bin/env"), "/usr/bin/env");
}

#[test]
fn resolve_home_path_expands() {
    let resolved = resolve_path("~/test.txt");
    // Should start with the home directory, not "~".
    assert!(!resolved.starts_with('~'));
    assert!(resolved.ends_with("test.txt"));
}

#[test]
fn resolve_relative_dot_path() {
    let resolved = resolve_path("./src/main.rs");
    // Should be an absolute path containing src/main.rs.
    assert!(resolved.contains("src/main.rs"));
    assert!(!resolved.starts_with('.'));
}
