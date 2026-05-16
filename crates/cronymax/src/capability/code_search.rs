//! Workspace code search capability: `search_workspace`, `grep_workspace`,
//! and `glob_files` tools.
//!
//! ## `search_workspace`
//!
//! Full-text search over workspace files using ripgrep (`rg`). Falls back
//! gracefully when `rg` is not in PATH with a clear error message.
//!
//! ## `grep_workspace`
//!
//! Pattern search via ripgrep with optional context lines. Identical
//! subprocess strategy to `search_workspace` but exposes `context_lines`
//! and filters results differently.
//!
//! ## `glob_files`
//!
//! File path enumeration using `globset` + `walkdir`, respecting
//! `.gitignore` via the `ignore` crate. Results are capped at 200;
//! `truncated: true` is set when the limit is exceeded.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use crate::agent_loop::tools::ToolOutcome;
use crate::capability::dispatcher::DispatcherBuilder;
use crate::llm::ToolDef;

// ── Tool: search_workspace ────────────────────────────────────────────────────

/// Register `search_workspace` on `builder`. Uses `rg` as the search engine.
pub fn register_search_workspace(builder: &mut DispatcherBuilder, workspace_root: PathBuf) {
    let root = workspace_root.clone();
    let def = ToolDef {
        name: "search_workspace".into(),
        description: "Full-text search across all workspace files using ripgrep. \
             Returns up to 20 matches with surrounding context. \
             Use `path_glob` to restrict to a sub-tree (e.g. 'src/**/*.rs')."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (regex)" },
                "path_glob": {
                    "type": "string",
                    "description": "Optional glob pattern to limit search scope",
                    "default": ""
                }
            },
            "required": ["query"]
        }),
    };
    builder.register(def, false, move |args| {
        let root = root.clone();
        async move {
            #[derive(serde::Deserialize)]
            struct Args {
                query: String,
                #[serde(default)]
                path_glob: String,
            }
            let a: Args = match serde_json::from_str(&args) {
                Ok(r) => r,
                Err(e) => return ToolOutcome::Error(format!("invalid search_workspace args: {e}")),
            };
            match run_rg_search(&root, &a.query, &a.path_glob, 3, 20).await {
                Ok(results) => ToolOutcome::Output(serde_json::json!({ "matches": results })),
                Err(e) => ToolOutcome::Error(format!("search_workspace failed: {e}")),
            }
        }
    });
}

// ── Tool: grep_workspace ──────────────────────────────────────────────────────

/// Register `grep_workspace` on `builder`.
pub fn register_grep_workspace(builder: &mut DispatcherBuilder, workspace_root: PathBuf) {
    let root = workspace_root.clone();
    let def = ToolDef {
        name: "grep_workspace".into(),
        description: "Search workspace files with a regex pattern via ripgrep. \
             Returns up to 50 matches. Set `context_lines` for surrounding context."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern" },
                "path_glob": {
                    "type": "string",
                    "description": "Optional glob to restrict to a subtree",
                    "default": ""
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lines of context before/after each match (default 0)",
                    "default": 0
                }
            },
            "required": ["pattern"]
        }),
    };
    builder.register(def, false, move |args| {
        let root = root.clone();
        async move {
            #[derive(serde::Deserialize)]
            struct Args {
                pattern: String,
                #[serde(default)]
                path_glob: String,
                #[serde(default)]
                context_lines: usize,
            }
            let a: Args = match serde_json::from_str(&args) {
                Ok(r) => r,
                Err(e) => return ToolOutcome::Error(format!("invalid grep_workspace args: {e}")),
            };
            match run_rg_search(&root, &a.pattern, &a.path_glob, a.context_lines, 50).await {
                Ok(results) => ToolOutcome::Output(serde_json::json!({ "matches": results })),
                Err(e) => ToolOutcome::Error(format!("grep_workspace failed: {e}")),
            }
        }
    });
}

// ── Tool: glob_files ──────────────────────────────────────────────────────────

/// Register `glob_files` on `builder`.
pub fn register_glob_files(builder: &mut DispatcherBuilder, workspace_root: PathBuf) {
    let root = workspace_root.clone();
    let def = ToolDef {
        name: "glob_files".into(),
        description: "List workspace files matching a glob pattern. \
             Respects .gitignore. Returns up to 200 paths; \
             `truncated: true` when limit exceeded."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern relative to workspace root (e.g. 'src/**/*.rs')"
                }
            },
            "required": ["pattern"]
        }),
    };
    builder.register(def, false, move |args| {
        let root = root.clone();
        async move {
            #[derive(serde::Deserialize)]
            struct Args {
                pattern: String,
            }
            let a: Args = match serde_json::from_str(&args) {
                Ok(r) => r,
                Err(e) => return ToolOutcome::Error(format!("invalid glob_files args: {e}")),
            };
            match run_glob(&root, &a.pattern, 200).await {
                Ok((files, truncated)) => ToolOutcome::Output(serde_json::json!({
                    "files": files,
                    "truncated": truncated,
                })),
                Err(e) => ToolOutcome::Error(format!("glob_files failed: {e}")),
            }
        }
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Invoke `rg --json` and parse results into a JSON array of match objects.
pub async fn run_rg_search(
    root: &Path,
    pattern: &str,
    path_glob: &str,
    context_lines: usize,
    max_results: usize,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut cmd = Command::new("rg");
    cmd.current_dir(root)
        .arg("--json")
        .arg("--max-count")
        .arg("1") // 1 match per line (to cap volume)
        .arg("--context")
        .arg(context_lines.to_string());

    if !path_glob.is_empty() {
        cmd.arg("--glob").arg(path_glob);
    }

    cmd.arg(pattern);

    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("rg timed out after 30 s"))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("ripgrep (`rg`) is not installed or not in PATH")
            } else {
                anyhow::anyhow!("rg spawn error: {e}")
            }
        })?;

    // rg exits with code 1 when there are no matches; that's fine.
    // Code 2 indicates an error.
    if output.status.code() == Some(2) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("rg error: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        if results.len() >= max_results {
            break;
        }
        let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(line) else {
            continue;
        };
        // rg --json emits "match", "context", "begin", "end", "summary" objects.
        // We only surface "match" and "context" lines.
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind == "match" || kind == "context" {
            if let Some(data) = v.get("data") {
                results.push(serde_json::json!({
                    "kind": kind,
                    "path": data.get("path").and_then(|p| p.get("text")).and_then(|t| t.as_str()).unwrap_or(""),
                    "line_number": data.get("line_number"),
                    "text": data.get("lines").and_then(|l| l.get("text")).and_then(|t| t.as_str()).unwrap_or("").trim_end_matches('\n'),
                }));
            }
        }
    }

    Ok(results)
}

/// Walk the workspace using the `ignore` crate and return matching paths.
pub async fn run_glob(
    root: &Path,
    pattern: &str,
    limit: usize,
) -> anyhow::Result<(Vec<String>, bool)> {
    let root = root.to_path_buf();
    let pattern = pattern.to_owned();

    tokio::task::spawn_blocking(move || {
        let glob = globset::GlobBuilder::new(&pattern)
            .literal_separator(false)
            .build()
            .map_err(|e| anyhow::anyhow!("invalid glob pattern '{pattern}': {e}"))?;
        let matcher = globset::GlobSet::builder()
            .add(glob)
            .build()
            .map_err(|e| anyhow::anyhow!("glob build failed: {e}"))?;

        let walker = ignore::WalkBuilder::new(&root)
            .hidden(false)
            .git_ignore(true)
            .build();

        let mut files: Vec<String> = Vec::new();
        let mut truncated = false;

        for entry in walker.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let rel = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                if matcher.is_match(&rel) {
                    if files.len() >= limit {
                        truncated = true;
                        break;
                    }
                    files.push(rel);
                }
            }
        }

        files.sort();
        Ok::<_, anyhow::Error>((files, truncated))
    })
    .await
    .map_err(|e| anyhow::anyhow!("glob_files task panicked: {e}"))
    .and_then(|r| r)
}
