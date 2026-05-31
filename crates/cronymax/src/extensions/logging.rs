//! Extension log management. See `docs/extensions/extension-logs.md`.
//!
//! Four log sinks per extension, each rotated on size:
//!
//! * [`LogKind::ExtensionStderr`] — Node host stderr (`host.log`)
//! * [`LogKind::ExtensionStdout`] — Node host stdout (`output.log`)
//! * [`LogKind::ExtensionChannel`] — `createOutputChannel(name).appendLine(...)`
//!   via RPC (`channels/<name>.log`)
//!
//! Plus two session-wide sinks:
//!
//! * [`LogKind::Platform`] — cronymax core (`platform.log`)
//! * [`LogKind::ExtensionHost`] — host manager lifecycle events (spawn /
//!   exit / crash) across all extensions (`extension-host.log`)
//!
//! v1 alpha dropped the security-audit framing. Crash / lifecycle telemetry
//! still lands in `extension-host.log` and `host.log` (Node stderr) — those
//! are operational, not security-audit. No per-extension `audit.log` is
//! produced, and the SDK does not surface a wrap-fs-calls audit hook.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::error::{ExtensionError, ExtensionResult};

/// Defaults match spec §3 of `extension-logs.md`.
#[derive(Clone, Debug)]
pub struct LogConfig {
    /// One log file rolls over when it reaches this many bytes.
    pub max_file_size: u64,
    /// Number of historical `.log.N` files kept per sink (default 3).
    pub max_history: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            max_file_size: 10 * 1024 * 1024,
            max_history: 3,
        }
    }
}

/// Per-session log manager. Hands out cached `Arc<LogWriter>` for any sink,
/// so callers don't accidentally open the same file twice.
#[derive(Debug)]
pub struct LogManager {
    root: PathBuf,
    session_id: String,
    config: LogConfig,
    writers: Mutex<HashMap<PathBuf, Arc<LogWriter>>>,
}

/// One of the well-known log sinks. `&str` ids keep the lifetime story
/// simple — callers always have a borrow handy at the call site.
#[derive(Debug, Clone, Copy)]
pub enum LogKind<'a> {
    /// `~/.cronymax/logs/<session>/platform.log`
    Platform,
    /// `~/.cronymax/logs/<session>/extension-host.log`
    ExtensionHost,
    /// `~/.cronymax/logs/<session>/extensions/<id>/host.log` — Node stderr
    ExtensionStderr(&'a str),
    /// `~/.cronymax/logs/<session>/extensions/<id>/output.log` — Node stdout
    ExtensionStdout(&'a str),
    /// `~/.cronymax/logs/<session>/extensions/<id>/channels/<channel>.log`
    ExtensionChannel { ext_id: &'a str, channel: &'a str },
}

/// One selectable log channel for the settings "Logs" tab dropdown.
/// `stdout` / `stderr` are the always-present `console.*` fallbacks; the rest
/// are `createOutputChannel` channels discovered on disk.
#[derive(Debug, Clone, Serialize)]
pub struct LogChannelInfo {
    /// Stable id passed back to [`LogManager::read_channel`] (`stdout`,
    /// `stderr`, or a channel file stem).
    pub id: String,
    /// Human label for the dropdown.
    pub label: String,
    /// `"stdout" | "stderr" | "channel"`.
    pub kind: String,
}

/// One rendered log line. `t` / `level` are populated only for structured
/// (NDJSON channel) logs; raw stdout/stderr lines carry just `text`.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    pub text: String,
}

/// Result of reading one channel: the (tail-bounded, optionally time-filtered)
/// entries, whether the source is structured NDJSON, and whether older lines
/// were dropped by the `limit`.
#[derive(Debug, Clone, Serialize)]
pub struct LogReadResult {
    pub entries: Vec<LogEntry>,
    pub structured: bool,
    pub truncated: bool,
}

/// Hard cap on lines returned by a single [`LogManager::read_channel`] so a
/// runaway log can't blow the control-channel payload. The UI tails; M1 adds
/// paging if needed.
const READ_LINE_CAP: usize = 5000;

impl LogManager {
    /// Open (or fail loudly) a new session under `root` (e.g.
    /// `~/.cronymax/logs/`). Generates a fresh `session_id` and creates the
    /// session directory.
    pub fn new_session(root: impl Into<PathBuf>) -> ExtensionResult<Self> {
        Self::with_config(root, LogConfig::default())
    }

    pub fn with_config(root: impl Into<PathBuf>, config: LogConfig) -> ExtensionResult<Self> {
        let mgr = Self {
            root: root.into(),
            session_id: generate_session_id(),
            config,
            writers: Mutex::new(HashMap::new()),
        };
        fs::create_dir_all(mgr.session_dir())?;
        Ok(mgr)
    }

    /// Adopt an existing `session_id` instead of generating one. Used by
    /// tests and by tools that need to attach to a known session dir.
    pub fn attach(
        root: impl Into<PathBuf>,
        session_id: impl Into<String>,
    ) -> ExtensionResult<Self> {
        let mgr = Self {
            root: root.into(),
            session_id: session_id.into(),
            config: LogConfig::default(),
            writers: Mutex::new(HashMap::new()),
        };
        fs::create_dir_all(mgr.session_dir())?;
        Ok(mgr)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn session_dir(&self) -> PathBuf {
        self.root.join(&self.session_id)
    }

    pub fn ext_dir(&self, ext_id: &str) -> PathBuf {
        self.session_dir().join("extensions").join(ext_id)
    }

    /// Return (or open + cache) the writer for the given sink. Repeated
    /// calls with the same `LogKind` return the same `Arc<LogWriter>`, so
    /// concurrent writes serialise on the writer's internal mutex.
    pub fn writer(&self, kind: LogKind<'_>) -> ExtensionResult<Arc<LogWriter>> {
        let path = self.resolve_path(kind);
        let mut writers = self
            .writers
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("log writers mutex poisoned".into()))?;
        if let Some(w) = writers.get(&path) {
            return Ok(w.clone());
        }
        let w = Arc::new(LogWriter::open(path.clone(), self.config.clone())?);
        writers.insert(path, w.clone());
        Ok(w)
    }

    fn resolve_path(&self, kind: LogKind<'_>) -> PathBuf {
        let session = self.session_dir();
        match kind {
            LogKind::Platform => session.join("platform.log"),
            LogKind::ExtensionHost => session.join("extension-host.log"),
            LogKind::ExtensionStderr(id) => self.ext_dir(id).join("host.log"),
            LogKind::ExtensionStdout(id) => self.ext_dir(id).join("output.log"),
            LogKind::ExtensionChannel { ext_id, channel } => self
                .ext_dir(ext_id)
                .join("channels")
                .join(format!("{}.log", sanitize_channel_name(channel))),
        }
    }

    /// On-disk path for a channel id as exposed to the UI (`stdout` / `stderr`
    /// map to the console fallbacks; any other id is a `createOutputChannel`
    /// file). The id is sanitised so a malicious value can't escape the
    /// channels dir.
    fn channel_file(&self, ext_id: &str, channel_id: &str) -> PathBuf {
        match channel_id {
            "stdout" => self.ext_dir(ext_id).join("output.log"),
            "stderr" => self.ext_dir(ext_id).join("host.log"),
            other => self
                .ext_dir(ext_id)
                .join("channels")
                .join(format!("{}.log", sanitize_channel_name(other))),
        }
    }

    /// Enumerate the log channels available for `ext_id`: the always-present
    /// `stdout` / `stderr` console fallbacks, then each `createOutputChannel`
    /// file found under `channels/` (live `.log` only; rotated `.log.N` are
    /// folded into their base). Sorted by label for a stable dropdown.
    pub fn channels(&self, ext_id: &str) -> Vec<LogChannelInfo> {
        let mut out = vec![
            LogChannelInfo {
                id: "stdout".into(),
                label: "stdout (console.log)".into(),
                kind: "stdout".into(),
            },
            LogChannelInfo {
                id: "stderr".into(),
                label: "stderr (console.error)".into(),
                kind: "stderr".into(),
            },
        ];
        let dir = self.ext_dir(ext_id).join("channels");
        if let Ok(entries) = fs::read_dir(&dir) {
            let mut channels: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    // Live channel files only: `<id>.log`, not rotated `<id>.log.1`.
                    name.strip_suffix(".log").map(|s| s.to_string())
                })
                .collect();
            channels.sort();
            channels.dedup();
            for id in channels {
                out.push(LogChannelInfo {
                    label: id.clone(),
                    id,
                    kind: "channel".into(),
                });
            }
        }
        out
    }

    /// Read one channel's live log file, tail-bounded to [`READ_LINE_CAP`] (or
    /// a smaller `limit`). `stdout`/`stderr` are returned as raw text lines;
    /// any other channel is parsed as NDJSON (`{t,level,msg}`) and, when
    /// `since_ms` is set, filtered to records at or after that wall-clock ms.
    /// A missing file yields an empty result (the channel just hasn't written
    /// yet), never an error.
    pub fn read_channel(
        &self,
        ext_id: &str,
        channel_id: &str,
        since_ms: Option<u64>,
        limit: Option<usize>,
    ) -> LogReadResult {
        let structured = !matches!(channel_id, "stdout" | "stderr");
        let path = self.channel_file(ext_id, channel_id);
        let cap = limit.unwrap_or(READ_LINE_CAP).min(READ_LINE_CAP);

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            // Not-yet-created file (or unreadable) → empty, not an error.
            Err(_) => {
                return LogReadResult {
                    entries: Vec::new(),
                    structured,
                    truncated: false,
                }
            }
        };

        let mut entries: Vec<LogEntry> = Vec::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            if structured {
                // Parse the NDJSON record; tolerate a malformed line by
                // surfacing it raw rather than dropping it.
                match serde_json::from_str::<serde_json::Value>(line) {
                    Ok(v) => {
                        let t = v.get("t").and_then(|x| x.as_u64());
                        if let (Some(since), Some(ts)) = (since_ms, t) {
                            if ts < since {
                                continue;
                            }
                        }
                        let level = v
                            .get("level")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        let text = v
                            .get("msg")
                            .and_then(|x| x.as_str())
                            .unwrap_or(line)
                            .to_string();
                        entries.push(LogEntry { t, level, text });
                    }
                    Err(_) => entries.push(LogEntry {
                        t: None,
                        level: None,
                        text: line.to_string(),
                    }),
                }
            } else {
                entries.push(LogEntry {
                    t: None,
                    level: None,
                    text: line.to_string(),
                });
            }
        }

        // Tail: keep the last `cap` entries.
        let truncated = entries.len() > cap;
        if truncated {
            entries.drain(0..entries.len() - cap);
        }
        LogReadResult {
            entries,
            structured,
            truncated,
        }
    }

    /// Truncate a channel's live log file (`OutputChannel.clear()` / the tab's
    /// "clear" button). No-op if the file doesn't exist.
    pub fn clear_channel(&self, ext_id: &str, channel_id: &str) -> ExtensionResult<()> {
        let path = self.channel_file(ext_id, channel_id);
        if path.exists() {
            OpenOptions::new().write(true).truncate(true).open(&path)?;
        }
        Ok(())
    }
}

/// Size-rotated, append-only log file. Cheap to share via `Arc`.
#[derive(Debug)]
pub struct LogWriter {
    path: PathBuf,
    file: Mutex<File>,
    config: LogConfig,
}

impl LogWriter {
    fn open(path: PathBuf, config: LogConfig) -> ExtensionResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
            config,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append `data` verbatim. Calls `rotate` afterwards iff the file
    /// crossed the size threshold.
    pub fn write_bytes(&self, data: &[u8]) -> ExtensionResult<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("log writer mutex poisoned".into()))?;
        file.write_all(data)?;
        let len = file.metadata()?.len();
        if len >= self.config.max_file_size {
            self.rotate_locked(&mut file)?;
        }
        Ok(())
    }

    /// Append `line`, terminating with `\n` if it doesn't already end with
    /// one.
    pub fn write_line(&self, line: &str) -> ExtensionResult<()> {
        if line.ends_with('\n') {
            self.write_bytes(line.as_bytes())
        } else {
            // Allocate once instead of two write_all calls so the rotation
            // check happens at a known size.
            let mut buf = String::with_capacity(line.len() + 1);
            buf.push_str(line);
            buf.push('\n');
            self.write_bytes(buf.as_bytes())
        }
    }

    /// Truncate the file to zero length. Backs `OutputChannel.clear()`.
    /// The handle stays in append mode, so the next write lands at offset 0.
    pub fn truncate(&self) -> ExtensionResult<()> {
        let file = self
            .file
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("log writer mutex poisoned".into()))?;
        file.set_len(0)?;
        Ok(())
    }

    fn rotate_locked(&self, file: &mut std::sync::MutexGuard<'_, File>) -> ExtensionResult<()> {
        // Drop the oldest history if it exists.
        let oldest = rotated_path(&self.path, self.config.max_history);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        // Shift .i → .(i+1) for i from max_history-1 down to 1.
        for i in (1..self.config.max_history).rev() {
            let from = rotated_path(&self.path, i);
            let to = rotated_path(&self.path, i + 1);
            if from.exists() {
                fs::rename(&from, &to)?;
            }
        }
        // Move the live file → .1. On Unix the existing fd in `file` stays
        // valid after rename — appended bytes would land in `.log.1`, but
        // we're about to swap it out anyway.
        if self.path.exists() {
            fs::rename(&self.path, rotated_path(&self.path, 1))?;
        }
        // Re-open the live file path and swap in.
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        **file = new_file;
        Ok(())
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn generate_session_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos_suffix = (now.subsec_nanos() ^ std::process::id()) & 0xffff;
    format!("session-{}-{:04x}", now.as_secs(), nanos_suffix)
}

/// Build `<base>.<n>` without going through `Path::with_extension` (which
/// would replace the existing `.log` instead of appending).
fn rotated_path(base: &Path, n: u32) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// Make a channel name safe for use as a filename. `"Coco / ACP"` →
/// `"coco-acp"`. Non-alphanumeric becomes `-`; consecutive dashes collapse.
fn sanitize_channel_name(name: &str) -> String {
    let lowered: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let parts: Vec<&str> = lowered.split('-').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        "channel".to_string()
    } else {
        parts.join("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn read_to_string(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    #[test]
    fn new_session_creates_session_dir() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::new_session(root.path()).unwrap();
        assert!(mgr.session_dir().is_dir());
        assert!(mgr.session_id().starts_with("session-"));
    }

    #[test]
    fn channels_lists_stdout_stderr_fallbacks_plus_discovered_files() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        // No channels written yet → still surfaces the two console fallbacks.
        let base = mgr.channels("ext.a");
        assert_eq!(
            base.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["stdout", "stderr"]
        );

        // Writing a channel makes it appear (sorted after the fallbacks).
        mgr.writer(LogKind::ExtensionChannel {
            ext_id: "ext.a",
            channel: "Coco / ACP",
        })
        .unwrap()
        .write_line(r#"{"t":1,"level":"info","msg":"hi"}"#)
        .unwrap();
        let ids: Vec<String> = mgr.channels("ext.a").into_iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["stdout", "stderr", "coco-acp"]);
        assert_eq!(
            mgr.channels("ext.a")
                .iter()
                .find(|c| c.id == "coco-acp")
                .unwrap()
                .kind,
            "channel"
        );
    }

    #[test]
    fn read_channel_parses_ndjson_and_filters_by_since_ms() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        let w = mgr
            .writer(LogKind::ExtensionChannel {
                ext_id: "ext.a",
                channel: "trace",
            })
            .unwrap();
        w.write_line(r#"{"t":100,"level":"info","msg":"first"}"#)
            .unwrap();
        w.write_line(r#"{"t":200,"level":"error","msg":"second"}"#)
            .unwrap();

        let all = mgr.read_channel("ext.a", "trace", None, None);
        assert!(all.structured);
        assert_eq!(all.entries.len(), 2);
        assert_eq!(all.entries[1].level.as_deref(), Some("error"));
        assert_eq!(all.entries[1].text, "second");

        // since_ms drops the earlier record.
        let recent = mgr.read_channel("ext.a", "trace", Some(150), None);
        assert_eq!(recent.entries.len(), 1);
        assert_eq!(recent.entries[0].text, "second");
    }

    #[test]
    fn read_channel_returns_raw_lines_for_stdout_and_empty_for_missing() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        // Missing file → empty, not an error.
        assert!(mgr
            .read_channel("ext.a", "stdout", None, None)
            .entries
            .is_empty());

        mgr.writer(LogKind::ExtensionStdout("ext.a"))
            .unwrap()
            .write_bytes(b"plain line one\nplain line two\n")
            .unwrap();
        let r = mgr.read_channel("ext.a", "stdout", None, None);
        assert!(!r.structured);
        assert_eq!(
            r.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["plain line one", "plain line two"]
        );
        // since_ms is ignored for raw streams (no timestamps).
        assert_eq!(
            mgr.read_channel("ext.a", "stdout", Some(999), None)
                .entries
                .len(),
            2
        );
    }

    #[test]
    fn clear_channel_truncates_the_file() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        let w = mgr
            .writer(LogKind::ExtensionChannel {
                ext_id: "ext.a",
                channel: "c",
            })
            .unwrap();
        w.write_line(r#"{"t":1,"msg":"x"}"#).unwrap();
        assert_eq!(mgr.read_channel("ext.a", "c", None, None).entries.len(), 1);
        mgr.clear_channel("ext.a", "c").unwrap();
        assert!(mgr
            .read_channel("ext.a", "c", None, None)
            .entries
            .is_empty());
    }

    #[test]
    fn writer_routes_each_kind_to_correct_path() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();

        for (kind, suffix) in [
            (LogKind::Platform, "S/platform.log"),
            (LogKind::ExtensionHost, "S/extension-host.log"),
            (
                LogKind::ExtensionStderr("alice.x"),
                "S/extensions/alice.x/host.log",
            ),
            (
                LogKind::ExtensionStdout("alice.x"),
                "S/extensions/alice.x/output.log",
            ),
        ] {
            let w = mgr.writer(kind).unwrap();
            assert_eq!(w.path(), root.path().join(suffix));
        }

        let ch = mgr
            .writer(LogKind::ExtensionChannel {
                ext_id: "alice.x",
                channel: "Coco / ACP",
            })
            .unwrap();
        assert_eq!(
            ch.path(),
            root.path()
                .join("S/extensions/alice.x/channels/coco-acp.log"),
        );
    }

    #[test]
    fn writer_caches_same_arc_for_same_kind() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        let a = mgr.writer(LogKind::ExtensionStderr("alice.x")).unwrap();
        let b = mgr.writer(LogKind::ExtensionStderr("alice.x")).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same kind must return cached Arc");
    }

    #[test]
    fn write_line_terminates_with_newline() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        let w = mgr.writer(LogKind::Platform).unwrap();
        w.write_line("hello").unwrap();
        w.write_line("world\n").unwrap();
        let body = read_to_string(w.path());
        assert_eq!(body, "hello\nworld\n");
    }

    #[test]
    fn write_bytes_does_not_alter_payload() {
        let root = TempDir::new().unwrap();
        let mgr = LogManager::attach(root.path(), "S").unwrap();
        let w = mgr.writer(LogKind::ExtensionStdout("alice.x")).unwrap();
        w.write_bytes(b"raw stdout chunk no newline").unwrap();
        let body = fs::read(w.path()).unwrap();
        assert_eq!(body, b"raw stdout chunk no newline");
    }

    #[test]
    fn rotation_shifts_history_on_size_threshold() {
        let root = TempDir::new().unwrap();
        let cfg = LogConfig {
            max_file_size: 16,
            max_history: 3,
        };
        let mgr = LogManager::with_config(root.path(), cfg).unwrap();
        let w = mgr.writer(LogKind::Platform).unwrap();

        // Write enough to rotate three times. Each line is 20 bytes (incl.
        // newline) > 16 byte threshold, so each write triggers a rotation.
        w.write_line("aaaaaaaaaaaaaaaaaaa").unwrap(); // rotates to .1
        w.write_line("bbbbbbbbbbbbbbbbbbb").unwrap(); // rotates: prev to .1, .1 → .2
        w.write_line("ccccccccccccccccccc").unwrap(); // rotates again

        let base = w.path();
        // After 3 rotations and a fresh live file (now empty until next
        // write), we should have base + .1 + .2 + .3 — but the live file
        // got truncated each rotation so the base file is empty at this
        // point. Each successive rotation pushes prior content down.
        assert!(base.exists(), "live base file must exist");
        assert!(rotated_path(base, 1).exists(), ".1 must exist");
        assert!(rotated_path(base, 2).exists(), ".2 must exist");
        assert!(rotated_path(base, 3).exists(), ".3 must exist");
    }

    #[test]
    fn rotation_drops_oldest_beyond_max_history() {
        let root = TempDir::new().unwrap();
        let cfg = LogConfig {
            max_file_size: 8,
            max_history: 2,
        };
        let mgr = LogManager::with_config(root.path(), cfg).unwrap();
        let w = mgr.writer(LogKind::Platform).unwrap();
        let base = w.path();

        // Drive 4 rotations. max_history=2 → we should still only see .1
        // and .2 at the end.
        for line in ["111111111", "222222222", "333333333", "444444444"] {
            w.write_line(line).unwrap();
        }
        assert!(rotated_path(base, 1).exists());
        assert!(rotated_path(base, 2).exists());
        assert!(
            !rotated_path(base, 3).exists(),
            ".3 must have been pruned (max_history=2)",
        );
    }

    #[test]
    fn sanitize_channel_name_cases() {
        assert_eq!(sanitize_channel_name("Coco"), "coco");
        assert_eq!(sanitize_channel_name("Coco / ACP"), "coco-acp");
        assert_eq!(sanitize_channel_name("CocoRequests"), "cocorequests");
        assert_eq!(sanitize_channel_name("foo_bar-baz"), "foo_bar-baz");
        assert_eq!(sanitize_channel_name("hello!!@#world"), "hello-world");
        assert_eq!(sanitize_channel_name(""), "channel");
        assert_eq!(sanitize_channel_name("!!!"), "channel");
        assert_eq!(sanitize_channel_name("中文"), "channel");
    }

    #[test]
    fn rotated_path_appends_suffix_after_log_extension() {
        // Sanity for the helper: must produce `host.log.1`, not `host.1`.
        let p = Path::new("/tmp/foo/host.log");
        assert_eq!(rotated_path(p, 1), Path::new("/tmp/foo/host.log.1"));
        assert_eq!(rotated_path(p, 42), Path::new("/tmp/foo/host.log.42"));
    }

    #[test]
    fn concurrent_writes_serialise() {
        use std::thread;
        let root = TempDir::new().unwrap();
        let mgr = Arc::new(LogManager::attach(root.path(), "S").unwrap());
        let mut handles = Vec::new();
        for tid in 0..8 {
            let mgr = mgr.clone();
            handles.push(thread::spawn(move || {
                let w = mgr.writer(LogKind::Platform).unwrap();
                for i in 0..100 {
                    w.write_line(&format!("t{tid:02}-{i:03}")).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let body = read_to_string(&root.path().join("S/platform.log"));
        let line_count = body.lines().count();
        assert_eq!(
            line_count,
            8 * 100,
            "every concurrent write_line should produce exactly one well-formed line",
        );
        // No torn writes: each line should match the pattern.
        for line in body.lines() {
            assert!(
                line.starts_with('t') && line.contains('-'),
                "torn line: {line:?}",
            );
        }
    }
}
