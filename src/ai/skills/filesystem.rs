//! Filesystem skills — read, write, patch, and list files with path security.
//!
//! All paths are resolved against CWD and canonicalized to prevent directory
//! traversal attacks. Category: `"filesystem"` for per-profile filtering.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};

/// Register all filesystem skills into the registry.
pub fn register_filesystem_skills(registry: &mut SkillRegistry) {
    register_fs_read_file(registry);
    register_fs_write_file(registry);
    register_fs_patch_file(registry);
    register_fs_list_dir(registry);
}

/// Resolve and validate a path relative to CWD.
///
/// Returns the canonicalized absolute path if it falls under CWD or is an
/// absolute path that exists. Rejects paths that escape CWD via `..` traversal.
fn resolve_path(raw: &str) -> anyhow::Result<std::path::PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let candidate = if std::path::Path::new(raw).is_absolute() {
        std::path::PathBuf::from(raw)
    } else {
        cwd.join(raw)
    };

    // For existing paths, canonicalize to resolve symlinks and `..`.
    // For new files (write/patch), canonicalize the parent.
    let resolved = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Cannot resolve path '{}': {}", raw, e))?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid path: no parent for '{}'", raw))?;
        if parent.exists() {
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| anyhow::anyhow!("Cannot resolve parent of '{}': {}", raw, e))?;
            canon_parent.join(candidate.file_name().unwrap_or_default())
        } else {
            candidate
        }
    };

    Ok(resolved)
}

// ─── cronymax.fs.read_file ──────────────────────────────────────────────────

fn register_fs_read_file(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.fs.read_file".into(),
        description: "Read the contents of a file, optionally a specific line range. \
            Returns the file content with line numbers. Use this to inspect source code, \
            configuration files, or any text file."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (absolute or relative to CWD)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Start line number (1-based, optional)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "End line number (1-based, inclusive, optional)"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum number of lines to return (default: 200)"
                }
            },
            "required": ["path"]
        }),
        category: "filesystem".into(),
    };

    let handler: SkillHandler = Arc::new(|args: Value| {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
            let resolved = resolve_path(raw_path)?;
            let max_lines = args["max_lines"].as_u64().unwrap_or(200) as usize;

            let content = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
                anyhow::anyhow!("Cannot read '{}': {}", resolved.display(), e)
            })?;

            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len();

            let start = args["start_line"]
                .as_u64()
                .map(|n| (n as usize).saturating_sub(1))
                .unwrap_or(0);
            let end = args["end_line"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(total_lines);

            let selected: Vec<String> = lines
                .iter()
                .enumerate()
                .skip(start)
                .take((end - start).min(max_lines))
                .map(|(i, line)| format!("{:>4} | {}", i + 1, line))
                .collect();

            let truncated = (end - start) > max_lines;

            Ok(json!({
                "path": resolved.display().to_string(),
                "content": selected.join("\n"),
                "total_lines": total_lines,
                "lines_shown": selected.len(),
                "truncated": truncated,
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── cronymax.fs.write_file ─────────────────────────────────────────────────

fn register_fs_write_file(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.fs.write_file".into(),
        description: "Write content to a file. Creates the file if it doesn't exist. \
            Can optionally create parent directories. Use this to create new files \
            or overwrite existing ones."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (absolute or relative to CWD)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "create_dirs": {
                    "type": "boolean",
                    "description": "Create parent directories if they don't exist (default: false)"
                }
            },
            "required": ["path", "content"]
        }),
        category: "filesystem".into(),
    };

    let handler: SkillHandler = Arc::new(|args: Value| {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;
            let create_dirs = args["create_dirs"].as_bool().unwrap_or(false);

            let resolved = resolve_path(raw_path)?;

            if create_dirs
                && let Some(parent) = resolved.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        anyhow::anyhow!("Cannot create directories for '{}': {}", resolved.display(), e)
                    })?;
                }

            let bytes_written = content.len();
            tokio::fs::write(&resolved, content).await.map_err(|e| {
                anyhow::anyhow!("Cannot write '{}': {}", resolved.display(), e)
            })?;

            Ok(json!({
                "path": resolved.display().to_string(),
                "bytes_written": bytes_written,
                "created": true,
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── cronymax.fs.patch_file ─────────────────────────────────────────────────

fn register_fs_patch_file(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.fs.patch_file".into(),
        description: "Find and replace text in a file. Replaces the first occurrence of \
            'search' with 'replace'. Safer than rewriting entire files. \
            Returns the number of replacements made."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (absolute or relative to CWD)"
                },
                "search": {
                    "type": "string",
                    "description": "Exact text to find in the file"
                },
                "replace": {
                    "type": "string",
                    "description": "Text to replace the found occurrence with"
                }
            },
            "required": ["path", "search", "replace"]
        }),
        category: "filesystem".into(),
    };

    let handler: SkillHandler = Arc::new(|args: Value| {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
            let search = args["search"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'search' argument"))?;
            let replace = args["replace"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'replace' argument"))?;

            let resolved = resolve_path(raw_path)?;
            let content = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
                anyhow::anyhow!("Cannot read '{}': {}", resolved.display(), e)
            })?;

            if !content.contains(search) {
                return Ok(json!({
                    "path": resolved.display().to_string(),
                    "replacements": 0,
                    "error": "Search text not found in file",
                }));
            }

            let new_content = content.replacen(search, replace, 1);
            tokio::fs::write(&resolved, &new_content).await.map_err(|e| {
                anyhow::anyhow!("Cannot write '{}': {}", resolved.display(), e)
            })?;

            Ok(json!({
                "path": resolved.display().to_string(),
                "replacements": 1,
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── cronymax.fs.list_dir ───────────────────────────────────────────────────

fn register_fs_list_dir(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.fs.list_dir".into(),
        description: "List the contents of a directory. Can be recursive up to a \
            specified depth. Returns file names, types, and sizes. Use this to \
            explore project structure."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory (absolute or relative to CWD)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Whether to list recursively (default: false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum recursion depth (default: 3)"
                },
                "max_entries": {
                    "type": "integer",
                    "description": "Maximum number of entries to return (default: 200)"
                }
            },
            "required": ["path"]
        }),
        category: "filesystem".into(),
    };

    let handler: SkillHandler = Arc::new(|args: Value| {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
            let recursive = args["recursive"].as_bool().unwrap_or(false);
            let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;
            let max_entries = args["max_entries"].as_u64().unwrap_or(200) as usize;

            let resolved = resolve_path(raw_path)?;

            if !resolved.is_dir() {
                return Err(anyhow::anyhow!("'{}' is not a directory", resolved.display()));
            }

            let mut entries = Vec::new();
            list_dir_recursive(&resolved, &resolved, recursive, max_depth, 0, max_entries, &mut entries).await?;

            Ok(json!({
                "path": resolved.display().to_string(),
                "entries": entries,
                "count": entries.len(),
                "truncated": entries.len() >= max_entries,
            }))
        })
    });

    registry.register(skill, handler);
}

/// Recursively list directory entries, respecting depth and count limits.
async fn list_dir_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    recursive: bool,
    max_depth: usize,
    current_depth: usize,
    max_entries: usize,
    entries: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let mut read_dir = tokio::fs::read_dir(dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        if entries.len() >= max_entries {
            break;
        }

        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/directories.
        if file_name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata().await?;
        let is_dir = metadata.is_dir();
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .display()
            .to_string();

        entries.push(json!({
            "name": relative,
            "type": if is_dir { "directory" } else { "file" },
            "size": if is_dir { 0 } else { metadata.len() },
        }));

        if recursive && is_dir && current_depth < max_depth {
            Box::pin(list_dir_recursive(
                base,
                &path,
                recursive,
                max_depth,
                current_depth + 1,
                max_entries,
                entries,
            ))
            .await?;
        }
    }

    Ok(())
}
