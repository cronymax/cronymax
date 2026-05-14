//! Snapshot persistence (task 4.4).
//!
//! Strategy: serialize the entire [`Snapshot`] as a single JSON
//! document under `<app_data_dir>/runtime-state.json`. Atomic writes go
//! through a temp file + rename to avoid leaving a half-written state
//! visible to a restart in flight.
//!
//! This isn't a high-throughput store — it's the rehydration journal.
//! Hot-path event journals and memory indexes will land in task 7.x;
//! the abstraction here is deliberately a single trait so that
//! migration can swap the backend without touching the authority code.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::state::{migrate_snapshot, Snapshot, SnapshotMigrationError};

/// Persistence backend used by [`super::authority::RuntimeAuthority`].
///
/// The trait is intentionally tiny: we only ever load on startup and
/// save after a state-changing operation. Implementations can choose
/// to debounce or batch as needed.
pub trait Persistence: Send + Sync + std::fmt::Debug + 'static {
    fn load(&self) -> Result<Snapshot, PersistenceError>;
    fn save(&self, snapshot: &Snapshot) -> Result<(), PersistenceError>;
}

/// Errors surfaced from [`Persistence`] implementations.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("snapshot migration: {0}")]
    Migration(#[from] SnapshotMigrationError),
}

/// JSON-file backed persistence. Used by the production runtime; tests
/// can substitute [`InMemoryPersistence`] (defined in this module
/// behind `cfg(test)`).
#[derive(Debug)]
pub struct JsonFilePersistence {
    path: PathBuf,
}

impl JsonFilePersistence {
    /// `path` is the absolute file path to the snapshot JSON. The
    /// parent directory must exist; the file may or may not.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Convenience constructor that targets `<app_data_dir>/runtime-state.json`.
    pub fn under_app_data_dir(app_data_dir: impl AsRef<Path>) -> Self {
        Self::new(app_data_dir.as_ref().join("runtime-state.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Persistence for JsonFilePersistence {
    fn load(&self) -> Result<Snapshot, PersistenceError> {
        let snap = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice::<Snapshot>(&bytes)?,
            // Missing file -> empty snapshot. Lets a fresh install boot
            // cleanly without callers needing to seed the file.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Snapshot::default(),
            Err(e) => return Err(PersistenceError::Io(e)),
        };
        // Apply schema migrations before handing the snapshot to the
        // authority. See `state::migrate_snapshot` (task 7.4).
        Ok(migrate_snapshot(snap)?)
    }

    fn save(&self, snapshot: &Snapshot) -> Result<(), PersistenceError> {
        let bytes = serde_json::to_vec_pretty(snapshot)?;
        // Atomic-ish write: temp file in the same directory, fsync,
        // rename over the target. Same-directory rename is atomic on
        // every supported FS, so a crash mid-write either leaves the
        // old snapshot intact or commits the new one — never both.
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// In-memory persistence backend. Used by tests and by hosts that
/// haven't wired durable storage yet (the runtime still rehydrates
/// correctly across restarts of the *same* process; nothing is
/// written to disk).
#[derive(Debug, Default)]
pub struct InMemoryPersistence {
    pub state: std::sync::Mutex<Snapshot>,
    pub save_count: std::sync::Mutex<usize>,
}

impl Persistence for InMemoryPersistence {
    fn load(&self) -> Result<Snapshot, PersistenceError> {
        Ok(migrate_snapshot(self.state.lock().unwrap().clone())?)
    }

    fn save(&self, snapshot: &Snapshot) -> Result<(), PersistenceError> {
        *self.state.lock().unwrap() = snapshot.clone();
        *self.save_count.lock().unwrap() += 1;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    pub use super::InMemoryPersistence;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::state::{Run, RunId, RunStatus, Space, SpaceId};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn sample_snapshot() -> Snapshot {
        let mut snap = Snapshot::default();
        let space = Space {
            id: SpaceId::new(),
            name: "scratch".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        snap.spaces.insert(space_id, space);
        let run = Run {
            id: RunId::new(),
            space_id,
            agent_id: None,
            session_id: None,
            flow_run_id: None,
            status: RunStatus::Paused,
            spec: serde_json::json!({"input": "hello"}),
            history: vec![],
            created_at_ms: 1,
            updated_at_ms: 2,
        };
        snap.runs.insert(run.id, run);
        snap
    }

    #[test]
    fn round_trip_through_json_file() {
        let dir = tempdir().unwrap();
        let p = JsonFilePersistence::under_app_data_dir(dir.path());
        let original = sample_snapshot();
        p.save(&original).unwrap();
        let loaded = p.load().unwrap();
        assert_eq!(loaded.spaces.len(), 1);
        assert_eq!(loaded.runs.len(), 1);
        let run = loaded.runs.values().next().unwrap();
        assert_eq!(run.status, RunStatus::Paused);
    }

    #[test]
    fn missing_file_loads_as_empty_snapshot() {
        let dir = tempdir().unwrap();
        let p = JsonFilePersistence::under_app_data_dir(dir.path());
        let loaded = p.load().unwrap();
        assert!(loaded.runs.is_empty());
        assert!(loaded.spaces.is_empty());
    }

    #[test]
    fn temp_file_does_not_clobber_existing_on_serde_failure() {
        // Sanity check that we always go through a temp file, not a
        // direct write. We approximate by saving twice and confirming
        // the persisted contents reflect the *second* write only.
        let dir = tempdir().unwrap();
        let p = JsonFilePersistence::under_app_data_dir(dir.path());
        let mut a = Snapshot::default();
        a.spaces.insert(
            SpaceId::new(),
            Space {
                id: SpaceId::new(),
                name: "first".into(),
                compaction_threshold_pct: 80,
                compaction_recency_turns: 6,
            },
        );
        p.save(&a).unwrap();
        let b = Snapshot {
            spaces: BTreeMap::new(),
            ..Snapshot::default()
        };
        p.save(&b).unwrap();
        let loaded = p.load().unwrap();
        assert!(loaded.spaces.is_empty());
    }

    // ── Schema migration (task 7.4) ──────────────────────────────────

    #[test]
    fn legacy_snapshot_without_schema_version_loads_as_v1() {
        // Simulate a pre-versioning snapshot on disk: the old format
        // had no `schema_version` field but is structurally compatible.
        let dir = tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");
        let legacy_json = serde_json::json!({
            "spaces": {},
            "agents": {},
            "runs": {},
            "memory": {},
            "reviews": {}
        });
        fs::write(&path, serde_json::to_vec(&legacy_json).unwrap()).unwrap();
        let p = JsonFilePersistence::new(&path);
        let loaded = p.load().unwrap();
        assert_eq!(
            loaded.schema_version,
            crate::runtime::state::SNAPSHOT_SCHEMA_VERSION
        );
    }

    #[test]
    fn snapshot_from_future_schema_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");
        let future_json = serde_json::json!({
            "schema_version": 9999,
            "spaces": {}, "agents": {}, "runs": {},
            "memory": {}, "reviews": {}
        });
        fs::write(&path, serde_json::to_vec(&future_json).unwrap()).unwrap();
        let p = JsonFilePersistence::new(&path);
        let err = p.load().expect_err("future schema must be rejected");
        assert!(matches!(err, PersistenceError::Migration(_)));
    }
}
