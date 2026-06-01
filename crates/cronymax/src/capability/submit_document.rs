//! `submit_document` tool capability.
//!
//! Allows the LLM to produce a document (Markdown body with a declared
//! `doc_type`) during a flow run. The final document is written to
//! `<workspace>/.cronymax/specs/<run_id>/<doc_type>.md`
//! (workspace-visible, gitignore-friendly), while intermediate artefacts
//! go to the app-data cache dir:
//!
//!   `<cache_dir>/flows/<flow_id>/history/<doc_id>.<rev>.md`  — history snapshots
//!   `<cache_dir>/flows/<flow_id>/locks/<doc_id>.lock`         — POSIX flock sidecars
//!
//! * **POSIX flock locking** — exclusive lock on `.locks/<name>.lock`
//!   so concurrent Rust/C++ writers don't corrupt each other.
//! * **Atomic write** — write to `<path>.tmp` then `rename()`.
//! * **History snapshot** — every write is mirrored to
//!   `.history/<name>.<rev>.md` so the current file always has a
//!   companion immutable snapshot.
//! * **SHA-256 digest** — returned in the tool result and stored in
//!   `reviews.json` via the mpsc notification channel.
//!
//! On success, a [`DocumentSubmitted`] message is sent to the supervision
//! loop so `FlowRuntime` can update port state and schedule downstream agents.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

// ── Wire types ────────────────────────────────────────────────────────────────

/// Notification sent through the mpsc channel after a successful document write.
#[derive(Clone, Debug)]
pub struct DocumentSubmitted {
    pub run_id: String,
    pub flow_id: String,
    pub doc_type: String,
    pub document_id: String,
    /// Workspace-relative path written (relative to workspace root).
    pub relative_path: String,
    /// The agent that submitted this document.
    pub agent_id: String,
    /// Document body (needed for @mention routing in `FlowRuntime::on_document_submitted`).
    pub body: String,
    /// 1-based revision number from the history counter.
    pub revision: u32,
    /// SHA-256 hex digest of the full written content (front-matter + body).
    pub sha256: String,
}

// ── Tool argument / result types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SubmitDocumentArgs {
    /// Doc-type name (must match a registered type in the space's doc-type registry).
    pub doc_type: String,
    /// Short human-readable title for the document.
    pub title: String,
    /// Full Markdown body of the document.
    pub body: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
struct SubmitDocumentResult {
    document_id: String,
    path: String,
    revision: u32,
    sha256: String,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// SHA-256 hex digest of `data`.
fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

/// Count the number of existing history files for `doc_id` under `history_dir`
/// to determine the next revision number.
///
/// History files are named `<doc_id>.<rev>.md` (1-based integers).
/// Returns `0` if the directory is absent or empty.
fn count_history_revisions(history_dir: &std::path::Path, doc_id: &str) -> u32 {
    let prefix = format!("{doc_id}.");
    let Ok(rd) = std::fs::read_dir(history_dir) else {
        return 0;
    };
    let mut max = 0u32;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.starts_with(&prefix) || !s.ends_with(".md") {
            continue;
        }
        let middle = &s[prefix.len()..s.len() - 3]; // strip prefix and ".md"
        if let Ok(n) = middle.parse::<u32>() {
            max = max.max(n);
        }
    }
    max
}

/// Return the current UTC time as a compact `YYYYMMDD-HHMM` string, suitable
/// for use as a filename prefix. Computed from `SystemTime` without any
/// external date/time crate.
#[allow(dead_code)]
fn compact_utc_ts() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Decompose seconds-since-epoch into a human-readable UTC date/time.
    // Uses the proleptic Gregorian calendar algorithm (Julian Day Number).
    let days = (secs / 86400) as i64;
    let time_of_day = secs % 86400;
    let hour = time_of_day / 3600;
    let min = (time_of_day % 3600) / 60;

    // Julian Day Number for 1970-01-01 is 2440588.
    let jd = days + 2440588;
    let a = jd + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;

    format!("{year:04}{month:02}{day:02}-{hour:02}{min:02}")
}

/// Acquire an exclusive POSIX flock on `lock_path`.
/// Returns the file descriptor that holds the lock (keep alive while writing).
/// Uses blocking file I/O — call only from a `spawn_blocking` context.
#[allow(deprecated)]
fn acquire_flock(lock_path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt as _;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(lock_path)?;
    // Blocking exclusive lock.
    nix::fcntl::flock(
        std::os::unix::io::AsRawFd::as_raw_fd(&file),
        nix::fcntl::FlockArg::LockExclusive,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(file)
}

/// Atomic write: write to `<path>.tmp` then `rename()`.
fn atomic_write(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

// ── Persistence core ──────────────────────────────────────────────────────────

/// Persist a flow document to disk and build the [`DocumentSubmitted`] event.
///
/// This is the shared write path: it writes the workspace-visible spec file
/// and an immutable history snapshot under a POSIX flock, computes the
/// revision number and SHA-256 digest, and returns the event the supervision
/// loop expects. It does **not** send on any channel — the caller decides how
/// to deliver the returned event.
///
/// Two callers share this: the `submit_document` tool handler (the body comes
/// from an LLM tool call) and the extension-provider flow dispatcher (the body
/// is the extension agent's accumulated turn output). `title` goes into the
/// YAML front-matter — pass `doc_type` itself when no distinct title exists.
/// `agent_id` is stored verbatim (in flow context callers pass the node id,
/// matching `register_submit_document`).
#[allow(clippy::too_many_arguments)]
pub async fn persist_flow_document(
    workspace_root: PathBuf,
    flow_id: String,
    run_id: String,
    agent_id: String,
    doc_type: String,
    title: String,
    body: String,
    cache_dir: Option<PathBuf>,
) -> Result<DocumentSubmitted, String> {
    if doc_type.is_empty() {
        return Err("doc_type must not be empty".into());
    }
    if body.is_empty() {
        return Err("body must not be empty".into());
    }

    // doc_type doubles as the document id so the spec file path matches the
    // port name used by FlowRuntime lookups.
    let document_id = doc_type.clone();

    // Final produce:  <workspace>/.cronymax/specs/<run_id>/<doc_id>.md — each
    // run gets its own subdirectory so different runs never overwrite. History
    // and lock sidecars live in the app-data cache dir (or co-located when no
    // cache dir is configured).
    let specs_dir = workspace_root.join(".cronymax").join("specs").join(&run_id);
    let (history_dir, locks_dir) = if let Some(ref cd) = cache_dir {
        let base = cd.join("flows").join(&flow_id);
        (base.join("history"), base.join("locks"))
    } else {
        (specs_dir.join(".history"), specs_dir.join(".locks"))
    };

    let front_matter = format!("---\ntitle: {title}\ndoc_type: {doc_type}\n---\n\n");
    let content_bytes = format!("{front_matter}{body}").into_bytes();

    let doc_id_clone = document_id.clone();
    let specs_dir_clone = specs_dir.clone();

    // Write on a blocking thread (flock is a blocking syscall).
    let write_result: std::io::Result<(u32, String)> = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&specs_dir_clone)?;
        std::fs::create_dir_all(history_dir.as_path())?;
        std::fs::create_dir_all(locks_dir.as_path())?;

        let lock_path = locks_dir.join(format!("{doc_id_clone}.lock"));
        let _lock_guard = acquire_flock(&lock_path)?;

        let rev = count_history_revisions(&history_dir, &doc_id_clone) + 1;
        let digest = sha256_hex(&content_bytes);

        // Write history first (so the snapshot is never missing).
        let history_path = history_dir.join(format!("{doc_id_clone}.{rev}.md"));
        atomic_write(&history_path, &content_bytes)?;

        // Write the current revision to the workspace-visible specs dir.
        let doc_path = specs_dir_clone.join(format!("{doc_id_clone}.md"));
        atomic_write(&doc_path, &content_bytes)?;

        Ok((rev, digest))
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));

    let (revision, sha256) = write_result.map_err(|e| format!("write failed: {e}"))?;

    let relative_path = format!(".cronymax/specs/{run_id}/{document_id}.md");

    tracing::info!(
        %run_id,
        %flow_id,
        %document_id,
        %doc_type,
        revision,
        sha = %&sha256[..8],
        "persist_flow_document: wrote document"
    );

    Ok(DocumentSubmitted {
        run_id,
        flow_id,
        doc_type,
        document_id,
        relative_path,
        agent_id,
        body,
        revision,
        sha256,
    })
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Async handler invoked when the LLM calls `submit_document`.
///
/// Parses the tool arguments, delegates to [`persist_flow_document`] for the
/// write, then sends the resulting [`DocumentSubmitted`] event on `tx` so the
/// supervision loop can forward it to `FlowRuntime`.
pub async fn handle(
    args_json: String,
    workspace_root: PathBuf,
    flow_id: String,
    run_id: String,
    agent_id: String,
    tx: mpsc::Sender<DocumentSubmitted>,
    cache_dir: Option<PathBuf>,
) -> crate::agent_loop::tools::ToolOutcome {
    use crate::agent_loop::tools::ToolOutcome;

    let args: SubmitDocumentArgs = match serde_json::from_str(&args_json) {
        Ok(a) => a,
        Err(e) => return ToolOutcome::Error(format!("invalid submit_document args: {e}")),
    };

    let evt = match persist_flow_document(
        workspace_root,
        flow_id,
        run_id,
        agent_id,
        args.doc_type,
        args.title,
        args.body,
        cache_dir,
    )
    .await
    {
        Ok(evt) => evt,
        Err(e) => return ToolOutcome::Error(format!("submit_document: {e}")),
    };

    // Capture the result payload before `evt` is moved into the channel.
    let result = serde_json::json!({
        "document_id": evt.document_id,
        "path": evt.relative_path,
        "revision": evt.revision,
        "sha256": evt.sha256,
    });

    // Signal the supervision loop (bounded channel, capacity 64).
    if tx.try_send(evt).is_err() {
        tracing::warn!(
            "submit_document: notification channel full, document written but not signalled"
        );
        return ToolOutcome::Error(
            "submit_document: run supervision channel is full; please retry in a moment".into(),
        );
    }

    ToolOutcome::Output(result)
}
