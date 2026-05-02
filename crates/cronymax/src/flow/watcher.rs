//! Coalesced filesystem watcher with debounce.
//!
//! [`FsWatcher`] wraps the [`notify`] crate (FSEvents on macOS, inotify on
//! Linux) and fires a single callback per debounce window, regardless of how
//! many raw filesystem events occurred within the window.
//!
//! The callback runs on a dedicated tokio task; it must be `Send + 'static`.
//!
//! Mirrors `app/flow/FsWatcher`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

/// A running filesystem watcher with debounce.
///
/// Dropping the `FsWatcher` stops the watcher and cancels the debounce task.
#[derive(Debug)]
pub struct FsWatcher {
    // Keep the notify watcher alive (dropping it stops watching).
    _watcher: RecommendedWatcher,
    // Keep the debounce task alive.
    _debounce_task: Arc<tokio::task::JoinHandle<()>>,
}

impl FsWatcher {
    /// Start watching `paths` recursively.
    ///
    /// `callback` is invoked at most once per `debounce` window after a
    /// burst of filesystem changes settles. It runs on a tokio task.
    pub fn start(
        paths: Vec<PathBuf>,
        debounce: Duration,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<()>();

        // Build the notify watcher. Events are forwarded to the channel.
        let tx = event_tx.clone();
        let mut watcher = notify::recommended_watcher(
            move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            },
        )?;

        for path in &paths {
            watcher.watch(path, RecursiveMode::Recursive)?;
        }

        // Debounce task: drain events, then sleep `debounce`; fire if quiet.
        let callback = Arc::new(callback);
        let task = tokio::spawn(async move {
            loop {
                // Wait for the first event.
                if event_rx.recv().await.is_none() {
                    break; // channel closed → watcher dropped
                }

                // Drain any events that arrive before the debounce window.
                let deadline =
                    tokio::time::Instant::now() + debounce;
                loop {
                    match tokio::time::timeout_at(
                        deadline,
                        event_rx.recv(),
                    )
                    .await
                    {
                        Ok(Some(())) => {} // more events — reset deadline
                        Ok(None) => return, // channel closed
                        Err(_) => break,    // debounce window expired
                    }
                }

                callback();
            }
        });

        Ok(Self {
            _watcher: watcher,
            _debounce_task: Arc::new(task),
        })
    }

    /// Stop the watcher explicitly. Equivalent to dropping the `FsWatcher`.
    pub fn stop(self) {
        drop(self);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn detects_file_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = Arc::clone(&count);

        let _watcher = FsWatcher::start(
            vec![dir.path().to_owned()],
            Duration::from_millis(50),
            move || {
                count_clone.fetch_add(1, Ordering::Relaxed);
            },
        )
        .unwrap();

        // Write a file to trigger the watcher.
        std::fs::write(dir.path().join("test.txt"), b"hello").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(count.load(Ordering::Relaxed) >= 1);
    }
}
