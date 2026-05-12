//! Git tools: `git_status`, `git_diff`, `git_log`, `git_add`, `git_reset`,
//! `git_commit`, `git_push`.
//!
//! `git_commit` and `git_push` require approval before executing.

use std::path::{Path, PathBuf};

use git2::{DiffOptions, Repository, StatusOptions};

use crate::agent_loop::tools::ToolOutcome;
use crate::capability::dispatcher::DispatcherBuilder;
use crate::llm::ToolDef;

// ── Registration ─────────────────────────────────────────────────────────────

/// Register all git tools on `builder`.
pub fn register_git_tools(builder: &mut DispatcherBuilder, workspace_root: PathBuf) {
    let root = workspace_root;

    // git_status
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_status".into(),
                description: "List tracked files with changes: staged, unstaged, untracked.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            false,
            move |_args| {
                let r = r.clone();
                async move {
                    match git_status(&r) {
                        Ok(entries) => ToolOutcome::Output(serde_json::json!({ "entries": entries })),
                        Err(e) => ToolOutcome::Error(format!("git_status: {e}")),
                    }
                }
            },
        );
    }

    // git_diff
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_diff".into(),
                description: "Show unified diff of working tree vs a ref (default: HEAD). \
                              Pass `staged: true` to diff staged changes vs HEAD."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ref": {
                            "type": "string",
                            "description": "Git ref to diff against (default: HEAD)"
                        },
                        "staged": {
                            "type": "boolean",
                            "description": "If true, diff staged changes vs HEAD"
                        }
                    },
                    "required": []
                }),
            },
            false,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize, Default)]
                    struct Args {
                        #[serde(default)]
                        r#ref: Option<String>,
                        #[serde(default)]
                        staged: bool,
                    }
                    let a: Args = serde_json::from_str(&args).unwrap_or_default();
                    match git_diff(&r, a.r#ref.as_deref(), a.staged) {
                        Ok(diff) => ToolOutcome::Output(serde_json::json!({ "diff": diff })),
                        Err(e) => ToolOutcome::Error(format!("git_diff: {e}")),
                    }
                }
            },
        );
    }

    // git_log
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_log".into(),
                description: "Show recent commits. `n` controls how many to return (default 10)."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "n": {
                            "type": "integer",
                            "description": "Number of commits to return (default 10)",
                            "default": 10
                        }
                    },
                    "required": []
                }),
            },
            false,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize)]
                    struct Args {
                        #[serde(default = "default_n")]
                        n: usize,
                    }
                    fn default_n() -> usize { 10 }
                    let a: Args = serde_json::from_str(&args).unwrap_or(Args { n: 10 });
                    match git_log(&r, a.n) {
                        Ok(commits) => ToolOutcome::Output(serde_json::json!({ "commits": commits })),
                        Err(e) => ToolOutcome::Error(format!("git_log: {e}")),
                    }
                }
            },
        );
    }

    // git_add
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_add".into(),
                description: "Stage files for the next commit. Pass `paths` as a list.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paths to stage (relative to workspace root)"
                        }
                    },
                    "required": ["paths"]
                }),
            },
            false,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize)]
                    struct Args { paths: Vec<String> }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => return ToolOutcome::Error(format!("invalid git_add args: {e}")),
                    };
                    match git_add(&r, &a.paths) {
                        Ok(staged) => ToolOutcome::Output(serde_json::json!({ "staged": staged })),
                        Err(e) => ToolOutcome::Error(format!("git_add: {e}")),
                    }
                }
            },
        );
    }

    // git_reset
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_reset".into(),
                description: "Unstage files (non-destructive: resets index to HEAD for listed paths)."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paths to unstage"
                        }
                    },
                    "required": ["paths"]
                }),
            },
            false,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize)]
                    struct Args { paths: Vec<String> }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => return ToolOutcome::Error(format!("invalid git_reset args: {e}")),
                    };
                    match git_reset(&r, &a.paths) {
                        Ok(unstaged) => ToolOutcome::Output(serde_json::json!({ "unstaged": unstaged })),
                        Err(e) => ToolOutcome::Error(format!("git_reset: {e}")),
                    }
                }
            },
        );
    }

    // git_commit (needs_approval = true)
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_commit".into(),
                description: "Create a commit from staged changes. Requires approval. \
                              The reviewer may override the commit message via `notes`."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "Commit message"
                        }
                    },
                    "required": ["message"]
                }),
            },
            true,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize)]
                    struct Args {
                        message: String,
                        #[serde(default)]
                        notes: Option<String>,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => return ToolOutcome::Error(format!("invalid git_commit args: {e}")),
                    };
                    let effective_message = a.notes.as_deref().unwrap_or(&a.message);
                    match git_commit(&r, effective_message) {
                        Ok((hash, files_changed)) => ToolOutcome::Output(serde_json::json!({
                            "hash": hash,
                            "message": effective_message,
                            "files_changed": files_changed,
                        })),
                        Err(e) => ToolOutcome::Error(format!("git_commit: {e}")),
                    }
                }
            },
        );
    }

    // git_push (needs_approval = true)
    {
        let r = root.clone();
        builder.register(
            ToolDef {
                name: "git_push".into(),
                description: "Push commits to a remote. Always requires approval. \
                              Defaults to `origin` and current branch."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "remote": {
                            "type": "string",
                            "description": "Remote name (default: origin)"
                        },
                        "branch": {
                            "type": "string",
                            "description": "Branch name (default: current branch)"
                        }
                    },
                    "required": []
                }),
            },
            true,
            move |args| {
                let r = r.clone();
                async move {
                    #[derive(serde::Deserialize, Default)]
                    struct Args {
                        #[serde(default)]
                        remote: Option<String>,
                        #[serde(default)]
                        branch: Option<String>,
                    }
                    let a: Args = serde_json::from_str(&args).unwrap_or_default();
                    let remote = a.remote.as_deref().unwrap_or("origin");
                    let branch = match a.branch.as_deref() {
                        Some(b) => b.to_owned(),
                        None => match current_branch(&r) {
                            Ok(b) => b,
                            Err(e) => return ToolOutcome::Error(format!("git_push: cannot determine branch: {e}")),
                        },
                    };
                    match run_git_push(&r, remote, &branch).await {
                        Ok(commits_pushed) => ToolOutcome::Output(serde_json::json!({
                            "remote": remote,
                            "branch": branch,
                            "commits_pushed": commits_pushed,
                        })),
                        Err(e) => ToolOutcome::Error(format!("git_push: {e}")),
                    }
                }
            },
        );
    }
}

// ── Git operations ────────────────────────────────────────────────────────────

fn open_repo(root: &Path) -> anyhow::Result<Repository> {
    Repository::open(root).map_err(|e| anyhow::anyhow!("git open failed: {e}"))
}

fn git_status(root: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let repo = open_repo(root)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut entries = Vec::new();
    for entry in statuses.iter() {
        let status = entry.status();
        let flags: Vec<&str> = [
            (git2::Status::INDEX_NEW, "index_new"),
            (git2::Status::INDEX_MODIFIED, "index_modified"),
            (git2::Status::INDEX_DELETED, "index_deleted"),
            (git2::Status::INDEX_RENAMED, "index_renamed"),
            (git2::Status::WT_MODIFIED, "wt_modified"),
            (git2::Status::WT_NEW, "wt_new"),
            (git2::Status::WT_DELETED, "wt_deleted"),
            (git2::Status::CONFLICTED, "conflicted"),
        ]
        .iter()
        .filter_map(|(flag, name)| if status.contains(*flag) { Some(*name) } else { None })
        .collect();

        if !flags.is_empty() {
            entries.push(serde_json::json!({
                "path": entry.path().unwrap_or(""),
                "status": flags,
            }));
        }
    }
    Ok(entries)
}

fn git_diff(root: &Path, git_ref: Option<&str>, staged: bool) -> anyhow::Result<String> {
    let repo = open_repo(root)?;
    let mut diff_opts = DiffOptions::new();

    let diff = if staged {
        // Index vs HEAD
        let head_tree = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?
    } else {
        // Working tree vs a given ref (or HEAD)
        let tree = if let Some(r) = git_ref {
            let obj = repo.revparse_single(r)?;
            Some(obj.peel_to_tree()?)
        } else {
            repo.head().ok().and_then(|h| h.peel_to_tree().ok())
        };
        repo.diff_tree_to_workdir_with_index(tree.as_ref(), Some(&mut diff_opts))?
    };

    let mut buf = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        if let Ok(s) = std::str::from_utf8(line.content()) {
            buf.push_str(&prefix);
            buf.push_str(s);
        }
        true
    })?;
    Ok(buf)
}

fn git_log(root: &Path, n: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    let repo = open_repo(root)?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;

    let mut commits = Vec::new();
    for oid_result in revwalk.take(n) {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;
        let author = commit.author();
        commits.push(serde_json::json!({
            "hash": oid.to_string(),
            "short_hash": &oid.to_string()[..8],
            "author": author.name().unwrap_or(""),
            "email": author.email().unwrap_or(""),
            "date": commit.time().seconds(),
            "subject": commit.summary().unwrap_or(""),
        }));
    }
    Ok(commits)
}

fn git_add(root: &Path, paths: &[String]) -> anyhow::Result<Vec<String>> {
    let repo = open_repo(root)?;
    let mut index = repo.index()?;
    for path in paths {
        index.add_path(Path::new(path))
            .map_err(|e| anyhow::anyhow!("git add '{path}': {e}"))?;
    }
    index.write()?;
    Ok(paths.to_vec())
}

fn git_reset(root: &Path, paths: &[String]) -> anyhow::Result<Vec<String>> {
    let repo = open_repo(root)?;
    let head = repo.head()?.peel_to_commit()?;
    let head_tree = head.tree()?;

    let mut index = repo.index()?;
    for path in paths {
        let p = Path::new(path);
        // If the file exists in HEAD tree, reset index entry to tree entry.
        // If it doesn't exist in HEAD (newly staged file), remove from index.
        match head_tree.get_path(p) {
            Ok(entry) => {
                index.add(&git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: entry.filemode() as u32,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id: entry.id(),
                    flags: 0,
                    flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                })?;
            }
            Err(_) => {
                index.remove_path(p).ok();
            }
        }
    }
    index.write()?;
    Ok(paths.to_vec())
}

fn git_commit(root: &Path, message: &str) -> anyhow::Result<(String, Vec<String>)> {
    let repo = open_repo(root)?;
    let sig = repo.signature()?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit<'_>> = parent_commit.iter().collect();

    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;

    // Collect changed files from the diff
    let files_changed = if let Some(parent) = &parent_commit {
        let parent_tree = parent.tree()?;
        let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)?;
        let mut files = Vec::new();
        diff.foreach(
            &mut |delta, _progress| {
                if let Some(p) = delta.new_file().path() {
                    files.push(p.to_string_lossy().into_owned());
                }
                true
            },
            None,
            None,
            None,
        )?;
        files
    } else {
        // First commit — list all tree entries
        let mut files = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |_prefix, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                files.push(entry.name().unwrap_or("").to_owned());
            }
            git2::TreeWalkResult::Ok
        })?;
        files
    };

    Ok((oid.to_string(), files_changed))
}

fn current_branch(root: &Path) -> anyhow::Result<String> {
    let repo = open_repo(root)?;
    let head = repo.head()?;
    if head.is_branch() {
        Ok(head.shorthand().unwrap_or("HEAD").to_owned())
    } else {
        Ok("HEAD".to_owned())
    }
}

async fn run_git_push(root: &Path, remote: &str, branch: &str) -> anyhow::Result<usize> {
    // Count commits ahead of remote before pushing
    let ahead = commits_ahead(root, remote, branch).unwrap_or(0);

    let output = tokio::process::Command::new("git")
        .current_dir(root)
        .arg("push")
        .arg(remote)
        .arg(branch)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("git push spawn: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git push failed: {stderr}");
    }

    Ok(ahead)
}

fn commits_ahead(root: &Path, remote: &str, branch: &str) -> anyhow::Result<usize> {
    let repo = open_repo(root)?;
    let local_ref = format!("refs/heads/{branch}");
    let remote_ref = format!("refs/remotes/{remote}/{branch}");

    let local_oid = repo.refname_to_id(&local_ref)?;
    let remote_oid = repo.refname_to_id(&remote_ref)?;

    let (ahead, _behind) = repo.graph_ahead_behind(local_oid, remote_oid)?;
    Ok(ahead)
}

// ── Test helpers (pub so integration tests can call them directly) ────────────

/// Thin public wrapper for use by integration tests.
pub fn git_status_for_test(root: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    git_status(root)
}

/// Thin public wrapper for use by integration tests.
pub fn git_diff_for_test(root: &Path, git_ref: Option<&str>, staged: bool) -> anyhow::Result<String> {
    git_diff(root, git_ref, staged)
}

/// Thin public wrapper for use by integration tests.
pub fn git_log_for_test(root: &Path, n: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    git_log(root, n)
}

/// Thin public wrapper for use by integration tests.
pub fn git_add_for_test(root: &Path, paths: &[String]) -> anyhow::Result<Vec<String>> {
    git_add(root, paths)
}

/// Thin public wrapper for use by integration tests.
/// Returns `(commit_hash, files_changed_count)`.
pub fn git_commit_for_test(root: &Path, message: &str) -> anyhow::Result<(String, usize)> {
    git_commit(root, message).map(|(hash, files)| (hash, files.len()))
}
