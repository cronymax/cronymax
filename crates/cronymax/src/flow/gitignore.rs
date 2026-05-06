//! `.gitignore` entry suggestions for the `.cronymax/` tree.
//!
//! The renderer surfaces these as an opt-in dialog ("Add these to your
//! .gitignore?"). This module MUST NEVER edit any user file — it is purely
//! advisory.
//!
//! Mirrors `app/flow/GitignoreHelper`.

use std::path::Path;

/// Advisory `.gitignore` suggestions for `.cronymax/` run artifacts.
pub struct GitignoreHelper;

impl GitignoreHelper {
    /// Returns the suggested `.gitignore` entries for the `.cronymax/` tree.
    ///
    /// Only run-time artifacts are suggested. Flow definitions, agent configs,
    /// doc-type schemas, and approved documents are **not** suggested for
    /// ignore — they are typically worth committing.
    pub fn suggested_entries() -> Vec<&'static str> {
        vec![
            // Run trace files can grow large (high-frequency events).
            ".cronymax/flows/*/runs/*/trace.jsonl",
            // Per-run mutable review state.
            ".cronymax/flows/*/runs/*/reviews.json",
            // Full runs directory (opt-in: more aggressive).
            // Commented out so the caller can decide.
            // ".cronymax/flows/*/runs/",
        ]
    }

    /// Returns entries from [`Self::suggested_entries()`] that are NOT already
    /// present in `<workspace_root>/.gitignore`.
    ///
    /// Reads only — never modifies any file.
    pub fn missing_entries(workspace_root: &Path) -> Vec<&'static str> {
        let gitignore = workspace_root.join(".gitignore");
        let existing_lines: std::collections::HashSet<String> =
            std::fs::read_to_string(&gitignore)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .map(|l| l.to_owned())
                .collect();

        Self::suggested_entries()
            .into_iter()
            .filter(|entry| !existing_lines.contains(*entry))
            .collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_entries_non_empty() {
        assert!(!GitignoreHelper::suggested_entries().is_empty());
    }

    #[test]
    fn all_missing_when_no_gitignore() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = GitignoreHelper::missing_entries(dir.path());
        assert_eq!(missing.len(), GitignoreHelper::suggested_entries().len());
    }

    #[test]
    fn already_present_entries_excluded() {
        let dir = tempfile::TempDir::new().unwrap();
        let entry = GitignoreHelper::suggested_entries()[0];
        std::fs::write(dir.path().join(".gitignore"), format!("{entry}\n")).unwrap();
        let missing = GitignoreHelper::missing_entries(dir.path());
        assert!(!missing.contains(&entry));
    }

    #[test]
    fn comments_in_gitignore_not_matched() {
        let dir = tempfile::TempDir::new().unwrap();
        let entry = GitignoreHelper::suggested_entries()[0];
        // Write the entry as a comment — it should NOT count as present.
        std::fs::write(
            dir.path().join(".gitignore"),
            format!("# {entry}\n"),
        )
        .unwrap();
        let missing = GitignoreHelper::missing_entries(dir.path());
        assert!(missing.contains(&entry));
    }
}
