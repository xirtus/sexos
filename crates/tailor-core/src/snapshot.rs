// SPDX-License-Identifier: GPL-3.0-only
//! Pure `#![no_std]` + `alloc` pre-edit snapshot & 1-click rollback engine.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::diff::format_unified_diff;
use crate::traits::WorkspaceFs;

/// Maximum number of retained snapshots to bound memory usage on SexOS.
pub const MAX_SNAPSHOTS: usize = 64;

/// A single file's pre-edit and post-edit state captured in a [`Snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub path: String,
    pub existed: bool,
    pub before: String,
    pub after: Option<String>,
}

/// An atomic pre-edit snapshot covering one or more files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub id: u64,
    pub label: String,
    pub entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    /// Returns a concise human-readable summary, e.g. `"#1 write_to_file src/main.rs (1 file)"`.
    pub fn summary(&self) -> String {
        let count = self.entries.len();
        let noun = if count == 1 { "file" } else { "files" };
        format!("#{} {} ({} {})", self.id, self.label, count, noun)
    }

    /// Formats a combined unified diff across all entries in this snapshot.
    pub fn unified_diff(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            let old = if entry.existed {
                entry.before.as_str()
            } else {
                ""
            };
            let new = entry.after.as_deref().unwrap_or("");
            let diff = format_unified_diff(&entry.path, old, new);
            if !diff.is_empty() {
                out.push_str(&diff);
            }
        }
        out
    }
}

/// Bounded in-memory store of pre-edit [`Snapshot`]s (capped at [`MAX_SNAPSHOTS`]).
#[derive(Clone, Debug, Default)]
pub struct SnapshotStore {
    next_id: u64,
    snapshots: Vec<Snapshot>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new snapshot, dropping the oldest snapshot when at capacity.
    pub fn record(&mut self, label: impl Into<String>, entries: Vec<SnapshotEntry>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        if self.snapshots.len() >= MAX_SNAPSHOTS {
            let overflow = self.snapshots.len() + 1 - MAX_SNAPSHOTS;
            self.snapshots.drain(0..overflow);
        }
        self.snapshots.push(Snapshot {
            id,
            label: label.into(),
            entries,
        });
        id
    }

    pub fn list(&self) -> &[Snapshot] {
        &self.snapshots
    }

    pub fn get(&self, id: u64) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    pub fn clear(&mut self) {
        self.snapshots.clear();
    }

    /// Restores the workspace to the pre-edit state captured in snapshot `id`.
    ///
    /// Files that existed before the edit are restored via [`WorkspaceFs::write_file`];
    /// files that did not exist before the edit are removed via [`WorkspaceFs::delete_file`].
    pub fn restore(&self, id: u64, ws: &mut dyn WorkspaceFs) -> Result<String, String> {
        let snap = self
            .get(id)
            .ok_or_else(|| format!("Snapshot #{} not found", id))?;

        for entry in snap.entries.iter().rev() {
            if entry.existed {
                ws.write_file(&entry.path, &entry.before)?;
            } else {
                ws.delete_file(&entry.path)?;
            }
        }

        let count = snap.entries.len();
        let noun = if count == 1 { "file" } else { "files" };
        Ok(format!(
            "Restored snapshot #{}: {} ({} {})",
            snap.id, snap.label, count, noun
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Workspace;
    use alloc::collections::BTreeMap;
    use alloc::vec;

    struct TestWs(BTreeMap<String, String>);

    impl TestWs {
        fn new() -> Self {
            Self(BTreeMap::new())
        }
    }

    impl Workspace for TestWs {
        fn read_file(&mut self, path: &str) -> Result<String, String> {
            self.0
                .get(path)
                .cloned()
                .ok_or_else(|| format!("not found: {}", path))
        }

        fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
            self.0.insert(String::from(path), String::from(content));
            Ok(())
        }

        fn list_dir(&mut self, _path: &str) -> Result<Vec<String>, String> {
            Ok(self.0.keys().cloned().collect())
        }
    }

    impl WorkspaceFs for TestWs {
        fn delete_file(&mut self, path: &str) -> Result<(), String> {
            self.0
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| format!("not found: {}", path))
        }
    }

    #[test]
    fn record_summary_diff_and_restore_modified_file() {
        let mut ws = TestWs::new();
        ws.write_file("src/main.rs", "fn main() { println!(\"new\"); }\n")
            .unwrap();

        let mut store = SnapshotStore::new();
        let id = store.record(
            "write_to_file src/main.rs",
            vec![SnapshotEntry {
                path: String::from("src/main.rs"),
                existed: true,
                before: String::from("fn main() { println!(\"old\"); }\n"),
                after: Some(String::from("fn main() { println!(\"new\"); }\n")),
            }],
        );
        assert_eq!(id, 1);
        assert_eq!(store.list().len(), 1);

        let snap = store.get(id).unwrap();
        assert_eq!(snap.summary(), "#1 write_to_file src/main.rs (1 file)");
        let diff = snap.unified_diff();
        assert!(diff.contains("--- a/src/main.rs"));
        assert!(diff.contains("-fn main() { println!(\"old\"); }"));
        assert!(diff.contains("+fn main() { println!(\"new\"); }"));

        let msg = store.restore(id, &mut ws).unwrap();
        assert!(msg.contains("Restored snapshot #1"));
        assert_eq!(
            ws.read_file("src/main.rs").unwrap(),
            "fn main() { println!(\"old\"); }\n"
        );
    }

    #[test]
    fn restore_deletes_newly_created_file() {
        let mut ws = TestWs::new();
        ws.write_file("src/new.rs", "pub const X: u32 = 42;\n")
            .unwrap();

        let mut store = SnapshotStore::new();
        let id = store.record(
            "write_file src/new.rs",
            vec![SnapshotEntry {
                path: String::from("src/new.rs"),
                existed: false,
                before: String::new(),
                after: Some(String::from("pub const X: u32 = 42;\n")),
            }],
        );

        let diff = store.get(id).unwrap().unified_diff();
        assert!(diff.contains("+pub const X: u32 = 42;"));

        store.restore(id, &mut ws).unwrap();
        assert!(ws.read_file("src/new.rs").is_err());
    }

    #[test]
    fn store_caps_at_64_snapshots_and_handles_missing_id() {
        let mut ws = TestWs::new();
        let mut store = SnapshotStore::new();
        for i in 0..70 {
            store.record(format!("edit {}", i), Vec::new());
        }
        assert_eq!(store.list().len(), MAX_SNAPSHOTS);
        assert!(store.get(1).is_none());
        assert!(store.get(6).is_none());
        assert!(store.get(7).is_some());
        assert!(store.get(70).is_some());
        assert!(store.restore(999, &mut ws).is_err());

        store.clear();
        assert!(store.list().is_empty());
    }
}
