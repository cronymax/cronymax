//! Document store for flow runs.
//!
//! Provides POSIX-flock-safe, atomic, history-snapshotting document I/O
//! under `<workspace>/.cronymax/flows/<flow>/docs/`.
//!
//! Layout under `<flow_dir>/`:
//!   docs/<name>.md                    — current revision
//!   docs/.history/<name>.<rev>.md     — immutable snapshots
//!   docs/.locks/<name>.lock           — POSIX flock sidecar

use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};

static NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]*$").unwrap());

pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && NAME_RE.is_match(name)
}

pub fn sha256_hex(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

pub struct WriteResult {
    pub revision: u32,
    pub sha256_hex: String,
    pub doc_path: PathBuf,
    pub history_path: PathBuf,
}

pub struct DocInfo {
    pub name: String,
    pub latest_revision: u32,
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------------
// DocumentStore
// ---------------------------------------------------------------------------

pub struct DocumentStore {
    flow_dir: PathBuf,
}

impl DocumentStore {
    pub fn new(flow_dir: PathBuf) -> Self {
        Self { flow_dir }
    }

    fn docs_dir(&self) -> PathBuf {
        self.flow_dir.join("docs")
    }
    fn history_dir(&self) -> PathBuf {
        self.flow_dir.join("docs").join(".history")
    }
    fn locks_dir(&self) -> PathBuf {
        self.flow_dir.join("docs").join(".locks")
    }
    fn doc_path(&self, name: &str) -> PathBuf {
        self.docs_dir().join(format!("{}.md", name))
    }
    fn history_path(&self, name: &str, rev: u32) -> PathBuf {
        self.history_dir().join(format!("{}.{}.md", name, rev))
    }

    pub fn latest_revision(&self, name: &str) -> u32 {
        let history = self.history_dir();
        let prefix = format!("{}.", name);
        let mut latest = 0u32;
        let entries = match std::fs::read_dir(&history) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let s = fname.to_string_lossy();
            if !s.starts_with(&prefix) {
                continue;
            }
            if !s.ends_with(".md") {
                continue;
            }
            let mid = &s[prefix.len()..s.len() - 3];
            if let Ok(rev) = mid.parse::<u32>() {
                if rev > latest {
                    latest = rev;
                }
            }
        }
        latest
    }

    /// Atomically write `content` to `path` (via `.tmp` + rename).
    fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
        let mut tmp = path.to_path_buf();
        let mut fname = tmp.file_name().unwrap_or_default().to_os_string();
        fname.push(".tmp");
        tmp.set_file_name(fname);
        std::fs::write(&tmp, content).context("atomic_write: write .tmp")?;
        std::fs::rename(&tmp, path).context("atomic_write: rename")?;
        Ok(())
    }

    /// Acquire an exclusive POSIX flock on `lock_path`, polling every 5 ms
    /// until `timeout_ms`. Returns the open file descriptor (caller must close).
    #[allow(deprecated)]
    fn acquire_flock(lock_path: &Path, timeout_ms: u64) -> Result<RawFd> {
        use nix::errno::Errno;
        use nix::fcntl::flock as nix_flock;
        use nix::fcntl::FlockArg;
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;

        let fd = open(
            lock_path,
            OFlag::O_RDWR | OFlag::O_CREAT,
            Mode::from_bits_truncate(0o644),
        )
        .context("open lock file")?;

        let deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match nix_flock(fd, FlockArg::LockExclusiveNonblock) {
                Ok(()) => return Ok(fd),
                Err(Errno::EWOULDBLOCK) => {
                    if Instant::now() >= deadline {
                        let _ = nix::unistd::close(fd);
                        bail!("lock contention (timed out after {}ms)", timeout_ms);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => {
                    let _ = nix::unistd::close(fd);
                    bail!("flock failed: {}", e);
                }
            }
        }
    }

    /// Write a new revision of `name`.  Returns the `WriteResult` on success.
    /// `lock_timeout_ms` == 0 fails immediately on contention.
    pub fn submit(&self, name: &str, content: &str, lock_timeout_ms: u64) -> Result<WriteResult> {
        if !is_safe_name(name) {
            bail!("invalid document name: {}", name);
        }
        std::fs::create_dir_all(self.docs_dir()).context("create docs/")?;
        std::fs::create_dir_all(self.history_dir()).context("create docs/.history/")?;
        std::fs::create_dir_all(self.locks_dir()).context("create docs/.locks/")?;

        let lock_path = self.locks_dir().join(format!("{}.lock", name));
        let fd = Self::acquire_flock(&lock_path, lock_timeout_ms)?;

        let result = (|| -> Result<WriteResult> {
            let next_rev = self.latest_revision(name) + 1;
            let doc_path = self.doc_path(name);
            let history_path = self.history_path(name, next_rev);
            let content_bytes = content.as_bytes();
            // History first — never lose a snapshot.
            Self::atomic_write(&history_path, content_bytes)?;
            Self::atomic_write(&doc_path, content_bytes)?;
            Ok(WriteResult {
                revision: next_rev,
                sha256_hex: sha256_hex(content),
                doc_path,
                history_path,
            })
        })();

        #[allow(deprecated)]
        let _ = nix::fcntl::flock(fd, nix::fcntl::FlockArg::Unlock);
        let _ = nix::unistd::close(fd);

        result
    }

    /// Read the current revision. Returns `None` if the document doesn't exist.
    pub fn read(&self, name: &str) -> Result<Option<String>> {
        if !is_safe_name(name) {
            bail!("invalid document name: {}", name);
        }
        match std::fs::read_to_string(self.doc_path(name)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Read a specific historical revision.
    pub fn read_revision(&self, name: &str, revision: u32) -> Result<Option<String>> {
        if !is_safe_name(name) || revision < 1 {
            bail!("invalid name or revision");
        }
        match std::fs::read_to_string(self.history_path(name, revision)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all documents in `docs/`.
    pub fn list(&self) -> Vec<DocInfo> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(self.docs_dir()) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if !is_safe_name(&stem) {
                continue;
            }
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let latest_revision = self.latest_revision(&stem);
            out.push(DocInfo {
                name: stem,
                latest_revision,
                size_bytes,
            });
        }
        out
    }

    /// Apply a suggestion to a block in the named document.
    ///
    /// Finds `<!-- block: <block_id> -->`, replaces the block body with
    /// `suggestion`, and submits a new revision. Returns the `WriteResult`.
    pub fn suggestion_apply(
        &self,
        name: &str,
        block_id: &str,
        suggestion: &str,
        lock_timeout_ms: u64,
    ) -> Result<WriteResult> {
        let current = self
            .read(name)?
            .ok_or_else(|| anyhow::anyhow!("document not found: {}", name))?;

        let new_content = apply_block_suggestion(&current, block_id, suggestion)
            .ok_or_else(|| anyhow::anyhow!("block_not_found_in_current_revision"))?;

        self.submit(name, &new_content, lock_timeout_ms)
    }
}

// ---------------------------------------------------------------------------
// Block-marker suggestion logic
// ---------------------------------------------------------------------------

/// Extract the UUID from a `<!-- block: <uuid> -->` line. Returns `None` if
/// the line doesn't match the pattern.
fn parse_block_marker(line: &str) -> Option<&str> {
    let s = line.trim_start();
    let s = s.strip_prefix("<!--")?.trim_start();
    let s = s.strip_prefix("block:")?.trim_start();
    // Collect hex/dash characters.
    let end = s
        .find(|c: char| !c.is_ascii_hexdigit() && c != '-')
        .unwrap_or(s.len());
    if end < 8 {
        return None;
    }
    Some(&s[..end])
}

fn apply_block_suggestion(md: &str, block_id: &str, suggestion: &str) -> Option<String> {
    let lines: Vec<&str> = md.lines().collect();

    // Find the marker line.
    let marker_idx = lines
        .iter()
        .position(|l| parse_block_marker(l) == Some(block_id))?;

    // Find the end of this block (next marker or end-of-file).
    let block_end = lines[marker_idx + 1..]
        .iter()
        .position(|l| parse_block_marker(l).is_some())
        .map(|i| marker_idx + 1 + i)
        .unwrap_or(lines.len());

    let trimmed = suggestion.trim_end_matches('\n');

    let mut new_content = String::new();
    for line in &lines[..=marker_idx] {
        new_content.push_str(line);
        new_content.push('\n');
    }
    new_content.push_str(trimmed);
    new_content.push_str("\n\n");
    for (i, line) in lines[block_end..].iter().enumerate() {
        new_content.push_str(line);
        if i + 1 < lines[block_end..].len() {
            new_content.push('\n');
        }
    }

    Some(new_content)
}
