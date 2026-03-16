//! Path auto-completion — filesystem-aware Tab completion for the prompt editor.
//!
//! When the user presses Tab in the prompt editor (and the suggestion panel is
//! not showing), the last whitespace-delimited token is treated as a partial
//! path.  Completions are fetched from the filesystem and either:
//!
//! - Immediately completed if there's exactly one match.
//! - Completed to the longest common prefix if multiple matches exist.

use std::path::PathBuf;

/// Result of a path-completion attempt.
#[derive(Debug)]
#[allow(unused)]
pub enum CompletionResult {
    /// No completions found — do nothing.
    None,
    /// Exactly one match — replace the token entirely.
    Single(String),
    /// Multiple matches — replace with the longest common prefix.
    /// Also stores the list of candidates for potential display.
    Multiple {
        #[allow(dead_code)]
        common_prefix: String,
        candidates: Vec<String>,
    },
}

/// Attempt to complete the last whitespace-delimited token in `text` as a path.
///
/// Returns the new full text (with the completion applied) and the completion
/// result type.
pub fn complete_path(text: &str) -> (String, CompletionResult) {
    // Find the last whitespace-delimited token.
    let (prefix, token) = match text.rfind(|c: char| c.is_whitespace()) {
        Some(pos) => (&text[..=pos], &text[pos + 1..]),
        None => ("", text),
    };

    if token.is_empty() {
        return (text.to_string(), CompletionResult::None);
    }

    // Expand `~` to the home directory.
    let expanded = if let Some(after_tilde) = token.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            let rest = after_tilde.strip_prefix('/').unwrap_or(after_tilde);
            home.join(rest)
        } else {
            PathBuf::from(token)
        }
    } else {
        PathBuf::from(token)
    };

    // Determine the directory to list and the partial filename to match against.
    let (dir, partial_name) = if expanded.is_dir() && token.ends_with('/') {
        // User typed a complete directory with trailing `/` — list its contents.
        (expanded.clone(), String::new())
    } else if let Some(parent) = expanded.parent() {
        // Partial — match against files in the parent directory.
        let name = expanded
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        (parent.to_path_buf(), name)
    } else {
        return (text.to_string(), CompletionResult::None);
    };

    // Read directory entries that start with the partial name.
    let entries = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return (text.to_string(), CompletionResult::None),
    };

    let partial_lower = partial_name.to_lowercase();
    let mut candidates: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden files unless the user explicitly typed a dot.
        if name.starts_with('.') && !partial_name.starts_with('.') {
            continue;
        }
        if name.to_lowercase().starts_with(&partial_lower) {
            let mut full = dir.join(&name).to_string_lossy().to_string();
            // Append `/` for directories.
            if entry.path().is_dir() && !full.ends_with('/') {
                full.push('/');
            }
            candidates.push(full);
        }
    }

    if candidates.is_empty() {
        return (text.to_string(), CompletionResult::None);
    }

    candidates.sort();

    // Re-collapse home directory into `~` if the original token started with `~`.
    let collapse_home = token.starts_with('~');
    let candidates: Vec<String> = if collapse_home {
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_string_lossy();
            candidates
                .into_iter()
                .map(|c| {
                    if c.starts_with(home_str.as_ref()) {
                        format!("~{}", &c[home_str.len()..])
                    } else {
                        c
                    }
                })
                .collect()
        } else {
            candidates
        }
    } else {
        candidates
    };

    if candidates.len() == 1 {
        let completed = format!("{}{}", prefix, candidates[0]);
        (completed, CompletionResult::Single(candidates[0].clone()))
    } else {
        // Find longest common prefix.
        let lcp = longest_common_prefix(&candidates);
        let completed = format!("{}{}", prefix, lcp);
        (
            completed,
            CompletionResult::Multiple {
                common_prefix: lcp,
                candidates,
            },
        )
    }
}

fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = &strings[0];
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        for (i, (a, b)) in first.chars().zip(s.chars()).enumerate() {
            if a != b {
                len = len.min(i);
                break;
            }
        }
    }
    first[..len].to_string()
}
