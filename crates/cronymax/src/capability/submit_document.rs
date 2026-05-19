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

// ── Handler ───────────────────────────────────────────────────────────────────

/// Async handler invoked when the LLM calls `submit_document`.
///
/// Writes the document file (with locking and history) and sends a
/// [`DocumentSubmitted`] event on `tx` so the supervision loop can forward
/// it to `FlowRuntime`.
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

    // 1. Parse arguments.
    let args: SubmitDocumentArgs = match serde_json::from_str(&args_json) {
        Ok(a) => a,
        Err(e) => return ToolOutcome::Error(format!("invalid submit_document args: {e}")),
    };

    if args.doc_type.is_empty() {
        return ToolOutcome::Error("submit_document: doc_type must not be empty".into());
    }
    if args.body.is_empty() {
        return ToolOutcome::Error("submit_document: body must not be empty".into());
    }

    // 2. Use doc_type as the document id so the spec file path matches the
    //    port name used by FlowRuntime lookups.
    //    Format: `<doc_type>` (e.g. `prototype`).
    //    Revision history is preserved separately in the cache dir.
    let document_id = args.doc_type.clone();

    // 3. Determine the output directories.
    //    Final produce:  <workspace>/.cronymax/specs/<run_id>/<run_id>/<doc_id>.md
    //    Each flow run gets its own subdirectory so different chat sessions
    //    (= different run_ids) never overwrite each other's documents.
    //    History (internal): <cache_dir>/flows/<flow_id>/history/
    //    Locks   (internal): <cache_dir>/flows/<flow_id>/locks/
    let specs_dir = workspace_root.join(".cronymax").join("specs").join(&run_id);

    let (history_dir, locks_dir) = if let Some(ref cd) = cache_dir {
        let base = cd.join("flows").join(&flow_id);
        (base.join("history"), base.join("locks"))
    } else {
        // Fallback: co-locate with the specs dir (no cache_dir configured).
        (specs_dir.join(".history"), specs_dir.join(".locks"))
    };

    // 4. Build the YAML front-matter + body content.
    let front_matter = format!(
        "---\ntitle: {}\ndoc_type: {}\n---\n\n",
        args.title, args.doc_type,
    );
    let content = format!("{front_matter}{}", args.body);
    let content_bytes = content.as_bytes().to_vec();

    let doc_id_clone = document_id.clone();
    let specs_dir_clone = specs_dir.clone();

    // 5. Write on a blocking thread (flock is a blocking syscall).
    let write_result: std::io::Result<(u32, String, PathBuf)> =
        tokio::task::spawn_blocking(move || {
            // Ensure directories exist.
            std::fs::create_dir_all(&specs_dir_clone)?;
            std::fs::create_dir_all(history_dir.as_path())?;
            std::fs::create_dir_all(locks_dir.as_path())?;

            let lock_path = locks_dir.join(format!("{doc_id_clone}.lock"));
            let _lock_guard = acquire_flock(&lock_path)?;

            // Determine revision.
            let rev = count_history_revisions(&history_dir, &doc_id_clone) + 1;

            // SHA-256 over the full content.
            let digest = sha256_hex(&content_bytes);

            // Write history first (so the snapshot is never missing).
            let history_path = history_dir.join(format!("{doc_id_clone}.{rev}.md"));
            atomic_write(&history_path, &content_bytes)?;

            // Write the current revision to the workspace-visible specs dir.
            let doc_path = specs_dir_clone.join(format!("{doc_id_clone}.md"));
            atomic_write(&doc_path, &content_bytes)?;

            Ok((rev, digest, doc_path))
        })
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));

    let (revision, sha256, _doc_path) = match write_result {
        Ok(r) => r,
        Err(e) => return ToolOutcome::Error(format!("submit_document: write failed: {e}")),
    };

    // 6. Build the workspace-relative path for the result payload.
    let relative_path = format!(".cronymax/specs/{run_id}/{document_id}.md");

    tracing::info!(
        %run_id,
        %flow_id,
        %document_id,
        doc_type = %args.doc_type,
        revision,
        sha = %&sha256[..8],
        "submit_document: wrote document"
    );

    // 7. Signal the supervision loop (bounded channel, capacity 64).
    let evt = DocumentSubmitted {
        run_id: run_id.clone(),
        flow_id,
        doc_type: args.doc_type.clone(),
        document_id: document_id.clone(),
        relative_path: relative_path.clone(),
        agent_id,
        body: args.body.clone(),
        revision,
        sha256: sha256.clone(),
    };
    if tx.try_send(evt).is_err() {
        tracing::warn!(
            %run_id,
            "submit_document: notification channel full, document written but not signalled"
        );
        return ToolOutcome::Error(
            "submit_document: run supervision channel is full; please retry in a moment".into(),
        );
    }

    // 8. Return success.
    ToolOutcome::Output(serde_json::json!({
        "document_id": document_id,
        "path": relative_path,
        "revision": revision,
        "sha256": sha256,
    }))
}
