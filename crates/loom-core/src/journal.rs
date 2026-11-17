// SPDX-License-Identifier: GPL-3.0-only
//! Omanta-inspired transactional file operation undo/redo journal and transfer progress popover model.
//!
//! Every file mutation recorded in [`UndoJournal`] carries a timestamp and can be pre-flighted
//! with [`UndoJournal::plan_undo`] / [`UndoJournal::plan_redo`] against an [`FsProbe`] so an undo
//! or redo never silently overwrites newer files or deletes modified targets.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// A source -> destination path pair for copies, moves, or batch renames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovePair {
    pub from: Vec<u8>,
    pub to: Vec<u8>,
}

impl MovePair {
    pub fn new(from: &[u8], to: &[u8]) -> Self {
        Self {
            from: from.to_vec(),
            to: to.to_vec(),
        }
    }
}

/// An item moved to the trash with both its original location and trash location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashItem {
    pub original: Vec<u8>,
    pub trashed: Vec<u8>,
}

impl TrashItem {
    pub fn new(original: &[u8], trashed: &[u8]) -> Self {
        Self {
            original: original.to_vec(),
            trashed: trashed.to_vec(),
        }
    }
}

/// Reversible file operation recorded in the undo/redo journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOp {
    Copy {
        items: Vec<MovePair>,
    },
    Move {
        items: Vec<MovePair>,
    },
    RenameBatch {
        renames: Vec<MovePair>,
    },
    Trash {
        items: Vec<TrashItem>,
    },
    CreateDir {
        path: Vec<u8>,
    },
    ArchiveExtract {
        archive: Vec<u8>,
        dest_dir: Vec<u8>,
        created_roots: Vec<Vec<u8>>,
    },
}

impl FileOp {
    pub fn copy(from: &[u8], to: &[u8]) -> Self {
        Self::Copy {
            items: alloc::vec![MovePair::new(from, to)],
        }
    }

    pub fn move_one(from: &[u8], to: &[u8]) -> Self {
        Self::Move {
            items: alloc::vec![MovePair::new(from, to)],
        }
    }
}

/// One recorded entry in [`UndoJournal`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: u64,
    pub timestamp: u64,
    pub op: FileOp,
}

/// Lightweight filesystem metadata returned by [`FsProbe`] for conflict pre-checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathMeta {
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

impl PathMeta {
    pub const fn file(size: u64, mtime: u64) -> Self {
        Self {
            exists: true,
            is_dir: false,
            size,
            mtime,
        }
    }

    pub const fn dir(mtime: u64) -> Self {
        Self {
            exists: true,
            is_dir: true,
            size: 0,
            mtime,
        }
    }
}

/// Read-only filesystem probe implemented by Loom (or an in-memory map in tests).
pub trait FsProbe {
    fn stat(&mut self, path: &[u8]) -> Option<PathMeta>;
}

/// Concrete step to run when applying an undo or redo plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlannedAction {
    Move { from: Vec<u8>, to: Vec<u8> },
    Copy { from: Vec<u8>, to: Vec<u8> },
    RemoveFileOrTree { path: Vec<u8> },
    RemoveEmptyDir { path: Vec<u8> },
    CreateDir { path: Vec<u8> },
    ExtractArchive { archive: Vec<u8>, dest_dir: Vec<u8> },
}

/// Why an undo or redo step cannot run cleanly without overwriting or losing newer data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    DestinationExists,
    SourceMissing,
    ModifiedSinceRecord,
    KindMismatch,
}

/// One detected conflict on a path during [`UndoJournal::plan_undo`] or [`UndoJournal::plan_redo`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub path: Vec<u8>,
    pub kind: ConflictKind,
}

/// Result of planning an undo or redo operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoPlan {
    pub entry_id: u64,
    pub actions: Vec<PlannedAction>,
    pub conflicts: Vec<Conflict>,
}

impl UndoPlan {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Bounded transactional undo/redo history for file operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoJournal {
    max_entries: usize,
    entries: Vec<JournalEntry>,
    /// Number of entries currently in the undoable prefix (`entries[..cursor]`).
    cursor: usize,
    next_id: u64,
}

impl Default for UndoJournal {
    fn default() -> Self {
        Self::new(64)
    }
}

impl UndoJournal {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: Vec::new(),
            cursor: 0,
            next_id: 1,
        }
    }

    pub const fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn undo_len(&self) -> usize {
        self.cursor
    }

    pub fn redo_len(&self) -> usize {
        self.entries.len().saturating_sub(self.cursor)
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    pub fn peek_undo(&self) -> Option<&JournalEntry> {
        if self.cursor == 0 {
            None
        } else {
            self.entries.get(self.cursor - 1)
        }
    }

    pub fn peek_redo(&self) -> Option<&JournalEntry> {
        self.entries.get(self.cursor)
    }

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }

    /// Records a completed operation, discarding any invalidated redo tail and enforcing the
    /// bounded capacity by evicting the oldest entries when full.
    pub fn record(&mut self, op: FileOp, timestamp: u64) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        if self.max_entries == 0 {
            return id;
        }

        self.entries.truncate(self.cursor);
        self.entries.push(JournalEntry { id, timestamp, op });

        if self.entries.len() > self.max_entries {
            let overflow = self.entries.len() - self.max_entries;
            self.entries.drain(..overflow);
        }
        self.cursor = self.entries.len();
        id
    }

    /// Builds an undo plan with conflict pre-checks without advancing the journal cursor.
    pub fn plan_undo(&self, probe: &mut dyn FsProbe) -> Option<UndoPlan> {
        let entry = self.peek_undo()?;
        Some(build_undo_plan(entry, probe))
    }

    /// Commits a previously planned undo by moving the cursor back one entry if `entry_id` matches.
    pub fn commit_undo(&mut self, entry_id: u64) -> bool {
        let Some(entry) = self.peek_undo() else {
            return false;
        };
        if entry.id != entry_id {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Builds a redo plan with conflict pre-checks without advancing the journal cursor.
    pub fn plan_redo(&self, probe: &mut dyn FsProbe) -> Option<UndoPlan> {
        let entry = self.peek_redo()?;
        Some(build_redo_plan(entry, probe))
    }

    /// Commits a previously planned redo by advancing the cursor forward one entry if `entry_id` matches.
    pub fn commit_redo(&mut self, entry_id: u64) -> bool {
        let Some(entry) = self.peek_redo() else {
            return false;
        };
        if entry.id != entry_id {
            return false;
        }
        self.cursor += 1;
        true
    }
}

fn path_exists(probe: &mut dyn FsProbe, path: &[u8]) -> Option<PathMeta> {
    probe.stat(path).filter(|m| m.exists)
}

fn build_undo_plan(entry: &JournalEntry, probe: &mut dyn FsProbe) -> UndoPlan {
    let mut actions = Vec::new();
    let mut conflicts = Vec::new();

    match &entry.op {
        FileOp::Copy { items } => {
            for pair in items.iter().rev() {
                match path_exists(probe, &pair.to) {
                    None => conflicts.push(Conflict {
                        path: pair.to.clone(),
                        kind: ConflictKind::SourceMissing,
                    }),
                    Some(meta) if meta.mtime > entry.timestamp => conflicts.push(Conflict {
                        path: pair.to.clone(),
                        kind: ConflictKind::ModifiedSinceRecord,
                    }),
                    Some(_) => {}
                }
                actions.push(PlannedAction::RemoveFileOrTree {
                    path: pair.to.clone(),
                });
            }
        }
        FileOp::Move { items } => {
            check_reverse_moves(items, entry.timestamp, probe, &mut actions, &mut conflicts);
        }
        FileOp::RenameBatch { renames } => {
            check_reverse_moves(renames, entry.timestamp, probe, &mut actions, &mut conflicts);
        }
        FileOp::Trash { items } => {
            let pairs: Vec<MovePair> = items
                .iter()
                .map(|t| MovePair {
                    from: t.original.clone(),
                    to: t.trashed.clone(),
                })
                .collect();
            check_reverse_moves(&pairs, entry.timestamp, probe, &mut actions, &mut conflicts);
        }
        FileOp::CreateDir { path } => {
            match path_exists(probe, path) {
                None => conflicts.push(Conflict {
                    path: path.clone(),
                    kind: ConflictKind::SourceMissing,
                }),
                Some(meta) if !meta.is_dir => conflicts.push(Conflict {
                    path: path.clone(),
                    kind: ConflictKind::KindMismatch,
                }),
                Some(meta) if meta.mtime > entry.timestamp => conflicts.push(Conflict {
                    path: path.clone(),
                    kind: ConflictKind::ModifiedSinceRecord,
                }),
                Some(_) => {}
            }
            actions.push(PlannedAction::RemoveEmptyDir { path: path.clone() });
        }
        FileOp::ArchiveExtract { created_roots, .. } => {
            for root in created_roots.iter().rev() {
                match path_exists(probe, root) {
                    None => conflicts.push(Conflict {
                        path: root.clone(),
                        kind: ConflictKind::SourceMissing,
                    }),
                    Some(meta) if meta.mtime > entry.timestamp => conflicts.push(Conflict {
                        path: root.clone(),
                        kind: ConflictKind::ModifiedSinceRecord,
                    }),
                    Some(_) => {}
                }
                actions.push(PlannedAction::RemoveFileOrTree { path: root.clone() });
            }
        }
    }

    UndoPlan {
        entry_id: entry.id,
        actions,
        conflicts,
    }
}

fn check_reverse_moves(
    pairs: &[MovePair],
    recorded_ts: u64,
    probe: &mut dyn FsProbe,
    actions: &mut Vec<PlannedAction>,
    conflicts: &mut Vec<Conflict>,
) {
    let destinations: BTreeSet<&[u8]> = pairs.iter().map(|p| p.to.as_slice()).collect();
    for pair in pairs.iter().rev() {
        match path_exists(probe, &pair.to) {
            None => conflicts.push(Conflict {
                path: pair.to.clone(),
                kind: ConflictKind::SourceMissing,
            }),
            Some(meta) if meta.mtime > recorded_ts => conflicts.push(Conflict {
                path: pair.to.clone(),
                kind: ConflictKind::ModifiedSinceRecord,
            }),
            Some(_) => {}
        }
        if !destinations.contains(pair.from.as_slice()) && path_exists(probe, &pair.from).is_some() {
            conflicts.push(Conflict {
                path: pair.from.clone(),
                kind: ConflictKind::DestinationExists,
            });
        }
        actions.push(PlannedAction::Move {
            from: pair.to.clone(),
            to: pair.from.clone(),
        });
    }
}

fn check_forward_moves(
    pairs: &[MovePair],
    probe: &mut dyn FsProbe,
    actions: &mut Vec<PlannedAction>,
    conflicts: &mut Vec<Conflict>,
    is_copy: bool,
) {
    let sources: BTreeSet<&[u8]> = pairs.iter().map(|p| p.from.as_slice()).collect();
    for pair in pairs {
        if path_exists(probe, &pair.from).is_none() {
            conflicts.push(Conflict {
                path: pair.from.clone(),
                kind: ConflictKind::SourceMissing,
            });
        }
        let collides = if is_copy {
            path_exists(probe, &pair.to).is_some()
        } else {
            !sources.contains(pair.to.as_slice()) && path_exists(probe, &pair.to).is_some()
        };
        if collides {
            conflicts.push(Conflict {
                path: pair.to.clone(),
                kind: ConflictKind::DestinationExists,
            });
        }
        actions.push(if is_copy {
            PlannedAction::Copy {
                from: pair.from.clone(),
                to: pair.to.clone(),
            }
        } else {
            PlannedAction::Move {
                from: pair.from.clone(),
                to: pair.to.clone(),
            }
        });
    }
}

fn build_redo_plan(entry: &JournalEntry, probe: &mut dyn FsProbe) -> UndoPlan {
    let mut actions = Vec::new();
    let mut conflicts = Vec::new();

    match &entry.op {
        FileOp::Copy { items } => {
            check_forward_moves(items, probe, &mut actions, &mut conflicts, true);
        }
        FileOp::Move { items } => {
            check_forward_moves(items, probe, &mut actions, &mut conflicts, false);
        }
        FileOp::RenameBatch { renames } => {
            check_forward_moves(renames, probe, &mut actions, &mut conflicts, false);
        }
        FileOp::Trash { items } => {
            let pairs: Vec<MovePair> = items
                .iter()
                .map(|t| MovePair {
                    from: t.original.clone(),
                    to: t.trashed.clone(),
                })
                .collect();
            check_forward_moves(&pairs, probe, &mut actions, &mut conflicts, false);
        }
        FileOp::CreateDir { path } => {
            if path_exists(probe, path).is_some() {
                conflicts.push(Conflict {
                    path: path.clone(),
                    kind: ConflictKind::DestinationExists,
                });
            }
            actions.push(PlannedAction::CreateDir { path: path.clone() });
        }
        FileOp::ArchiveExtract {
            archive,
            dest_dir,
            created_roots,
        } => {
            if path_exists(probe, archive).is_none() {
                conflicts.push(Conflict {
                    path: archive.clone(),
                    kind: ConflictKind::SourceMissing,
                });
            }
            for root in created_roots {
                if path_exists(probe, root).is_some() {
                    conflicts.push(Conflict {
                        path: root.clone(),
                        kind: ConflictKind::DestinationExists,
                    });
                }
            }
            actions.push(PlannedAction::ExtractArchive {
                archive: archive.clone(),
                dest_dir: dest_dir.clone(),
            });
        }
    }

    UndoPlan {
        entry_id: entry.id,
        actions,
        conflicts,
    }
}

/// Execution state for Loom's transfer progress popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferState {
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

/// Progress model for active transfers shown in Loom's progress popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferProgress {
    pub id: u64,
    pub label: Vec<u8>,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub items_done: u32,
    pub items_total: u32,
    /// Exponentially smoothed throughput in bytes per second.
    pub throughput_bps: u64,
    /// Estimated remaining time in milliseconds when running with known throughput.
    pub eta_ms: Option<u64>,
    pub state: TransferState,
    last_update_ms: u64,
    last_bytes_done: u64,
}

impl TransferProgress {
    pub fn new(
        id: u64,
        label: &[u8],
        bytes_total: u64,
        items_total: u32,
        started_ms: u64,
    ) -> Self {
        Self {
            id,
            label: label.to_vec(),
            bytes_done: 0,
            bytes_total,
            items_done: 0,
            items_total,
            throughput_bps: 0,
            eta_ms: None,
            state: TransferState::Running,
            last_update_ms: started_ms,
            last_bytes_done: 0,
        }
    }

    pub const fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TransferState::Completed | TransferState::Cancelled | TransferState::Failed
        )
    }

    /// Progress in permille (`0..=1000`) for UI progress bars.
    pub fn fraction_permille(&self) -> u16 {
        if self.bytes_total == 0 {
            return if self.state == TransferState::Completed {
                1000
            } else {
                0
            };
        }
        let done = self.bytes_done.min(self.bytes_total);
        ((done as u128 * 1000) / self.bytes_total as u128) as u16
    }

    /// Updates transferred counts and recomputes EWMA throughput and ETA.
    pub fn update(&mut self, bytes_done: u64, items_done: u32, now_ms: u64) {
        if self.is_terminal() {
            return;
        }
        let clamped_bytes = if self.bytes_total > 0 {
            bytes_done.min(self.bytes_total)
        } else {
            bytes_done
        };
        self.bytes_done = clamped_bytes;
        self.items_done = items_done.min(self.items_total.max(items_done));

        if self.state == TransferState::Running {
            let dt_ms = now_ms.saturating_sub(self.last_update_ms);
            if dt_ms > 0 {
                let d_bytes = clamped_bytes.saturating_sub(self.last_bytes_done);
                let inst_bps = (d_bytes as u128 * 1000 / dt_ms as u128) as u64;
                self.throughput_bps = if self.throughput_bps == 0 {
                    inst_bps
                } else {
                    (self.throughput_bps.saturating_mul(3).saturating_add(inst_bps)) / 4
                };
                self.last_update_ms = now_ms;
                self.last_bytes_done = clamped_bytes;
            }
            self.recompute_eta();
        }
    }

    pub fn pause(&mut self) {
        if self.state == TransferState::Running {
            self.state = TransferState::Paused;
            self.eta_ms = None;
        }
    }

    pub fn resume(&mut self, now_ms: u64) {
        if self.state == TransferState::Paused {
            self.state = TransferState::Running;
            self.last_update_ms = now_ms;
            self.last_bytes_done = self.bytes_done;
            self.recompute_eta();
        }
    }

    pub fn cancel(&mut self) {
        if !self.is_terminal() {
            self.state = TransferState::Cancelled;
            self.eta_ms = None;
        }
    }

    pub fn complete(&mut self) {
        if !self.is_terminal() {
            self.state = TransferState::Completed;
            self.bytes_done = self.bytes_total;
            self.items_done = self.items_total;
            self.eta_ms = Some(0);
        }
    }

    pub fn fail(&mut self) {
        if !self.is_terminal() {
            self.state = TransferState::Failed;
            self.eta_ms = None;
        }
    }

    fn recompute_eta(&mut self) {
        let remaining = self.bytes_total.saturating_sub(self.bytes_done);
        if remaining == 0 && self.bytes_total > 0 {
            self.eta_ms = Some(0);
        } else if self.throughput_bps > 0 {
            self.eta_ms = Some(((remaining as u128 * 1000) / self.throughput_bps as u128) as u64);
        } else {
            self.eta_ms = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    struct MemFs(BTreeMap<Vec<u8>, PathMeta>);
    impl FsProbe for MemFs {
        fn stat(&mut self, path: &[u8]) -> Option<PathMeta> {
            self.0.get(path).copied()
        }
    }

    #[test]
    fn bounded_history_and_redo_truncation() {
        let mut journal = UndoJournal::new(3);
        let id1 = journal.record(FileOp::CreateDir { path: b"/d1".to_vec() }, 10);
        let id2 = journal.record(FileOp::CreateDir { path: b"/d2".to_vec() }, 20);
        let id3 = journal.record(FileOp::CreateDir { path: b"/d3".to_vec() }, 30);
        let id4 = journal.record(FileOp::CreateDir { path: b"/d4".to_vec() }, 40);
        assert_eq!(journal.undo_len(), 3);
        assert_eq!(journal.entries()[0].id, id2);
        assert_ne!(id1, id2);
        assert_eq!(journal.peek_undo().unwrap().id, id4);

        assert!(journal.commit_undo(id4));
        assert_eq!(journal.undo_len(), 2);
        assert_eq!(journal.redo_len(), 1);
        assert_eq!(journal.peek_undo().unwrap().id, id3);
        assert_eq!(journal.peek_redo().unwrap().id, id4);

        // Recording a new op after undo clears redo tail.
        let id5 = journal.record(FileOp::CreateDir { path: b"/d5".to_vec() }, 50);
        assert_eq!(journal.redo_len(), 0);
        assert_eq!(journal.peek_undo().unwrap().id, id5);
    }

    #[test]
    fn undo_detects_newer_file_conflicts_without_overwriting() {
        let mut fs = MemFs(BTreeMap::new());
        fs.0.insert(b"/dst/a.txt".to_vec(), PathMeta::file(100, 100));

        let mut journal = UndoJournal::new(16);
        let mv_id = journal.record(FileOp::move_one(b"/src/a.txt", b"/dst/a.txt"), 100);

        // Clean undo when /src/a.txt does not exist and /dst/a.txt is unmodified.
        let clean = journal.plan_undo(&mut fs).unwrap();
        assert!(clean.is_clean());
        assert_eq!(
            clean.actions,
            alloc::vec![PlannedAction::Move {
                from: b"/dst/a.txt".to_vec(),
                to: b"/src/a.txt".to_vec(),
            }]
        );

        // Now simulate a newer file written at /src/a.txt and an edit on /dst/a.txt.
        fs.0.insert(b"/src/a.txt".to_vec(), PathMeta::file(200, 150));
        fs.0.insert(b"/dst/a.txt".to_vec(), PathMeta::file(120, 160));
        let conflicted = journal.plan_undo(&mut fs).unwrap();
        assert!(!conflicted.is_clean());
        assert!(conflicted.conflicts.contains(&Conflict {
            path: b"/src/a.txt".to_vec(),
            kind: ConflictKind::DestinationExists,
        }));
        assert!(conflicted.conflicts.contains(&Conflict {
            path: b"/dst/a.txt".to_vec(),
            kind: ConflictKind::ModifiedSinceRecord,
        }));
        assert!(journal.commit_undo(mv_id));
    }

    #[test]
    fn rename_batch_trash_and_archive_extract_undo_redo() {
        let mut fs = MemFs(BTreeMap::new());
        fs.0.insert(b"/dir/b".to_vec(), PathMeta::file(10, 10));
        fs.0.insert(b"/dir/c".to_vec(), PathMeta::file(10, 10));
        fs.0.insert(b"/trash/1".to_vec(), PathMeta::file(50, 20));
        fs.0.insert(b"/dl/pkg".to_vec(), PathMeta::dir(30));

        let mut journal = UndoJournal::new(16);
        let id_ren = journal.record(
            FileOp::RenameBatch {
                renames: alloc::vec![
                    MovePair::new(b"/dir/b", b"/dir/c"),
                    MovePair::new(b"/dir/a", b"/dir/b"),
                ],
            },
            10,
        );
        let id_trash = journal.record(
            FileOp::Trash {
                items: alloc::vec![TrashItem::new(b"/home/doc.txt", b"/trash/1")],
            },
            20,
        );
        let id_arc = journal.record(
            FileOp::ArchiveExtract {
                archive: b"/dl/pkg.tar".to_vec(),
                dest_dir: b"/dl".to_vec(),
                created_roots: alloc::vec![b"/dl/pkg".to_vec()],
            },
            30,
        );

        let arc_undo = journal.plan_undo(&mut fs).unwrap();
        assert!(arc_undo.is_clean());
        assert_eq!(
            arc_undo.actions,
            alloc::vec![PlannedAction::RemoveFileOrTree {
                path: b"/dl/pkg".to_vec()
            }]
        );
        assert!(journal.commit_undo(id_arc));

        let trash_undo = journal.plan_undo(&mut fs).unwrap();
        assert!(trash_undo.is_clean());
        assert_eq!(
            trash_undo.actions,
            alloc::vec![PlannedAction::Move {
                from: b"/trash/1".to_vec(),
                to: b"/home/doc.txt".to_vec(),
            }]
        );
        assert!(journal.commit_undo(id_trash));

        let ren_undo = journal.plan_undo(&mut fs).unwrap();
        assert!(ren_undo.is_clean());
        assert_eq!(
            ren_undo.actions,
            alloc::vec![
                PlannedAction::Move {
                    from: b"/dir/b".to_vec(),
                    to: b"/dir/a".to_vec(),
                },
                PlannedAction::Move {
                    from: b"/dir/c".to_vec(),
                    to: b"/dir/b".to_vec(),
                },
            ]
        );
        assert!(journal.commit_undo(id_ren));

        // Redo rename batch: /dir/a and /dir/b exist in fs now.
        fs.0.remove(b"/dir/c".as_slice());
        fs.0.insert(b"/dir/a".to_vec(), PathMeta::file(10, 10));
        let ren_redo = journal.plan_redo(&mut fs).unwrap();
        assert!(ren_redo.is_clean());
        assert!(journal.commit_redo(id_ren));
    }

    #[test]
    fn transfer_progress_ewma_eta_and_pause_resume() {
        let mut tp = TransferProgress::new(7, b"Copying clip.mkv", 10_000, 2, 1000);
        assert_eq!(tp.fraction_permille(), 0);
        assert_eq!(tp.eta_ms, None);

        // 2000 bytes in 1000 ms => 2000 B/s, remaining 8000 B => 4000 ms ETA.
        tp.update(2_000, 1, 2000);
        assert_eq!(tp.throughput_bps, 2_000);
        assert_eq!(tp.eta_ms, Some(4_000));
        assert_eq!(tp.fraction_permille(), 200);

        // Pause clears ETA, resume resets baseline timestamp.
        tp.pause();
        assert_eq!(tp.state, TransferState::Paused);
        assert_eq!(tp.eta_ms, None);
        tp.resume(10_000);
        assert_eq!(tp.state, TransferState::Running);

        // 4000 more bytes in 1000 ms => inst 4000 B/s, smoothed = (2000*3 + 4000)/4 = 2500 B/s.
        tp.update(6_000, 1, 11_000);
        assert_eq!(tp.throughput_bps, 2_500);
        assert_eq!(tp.eta_ms, Some(1_600));

        tp.complete();
        assert_eq!(tp.state, TransferState::Completed);
        assert_eq!(tp.fraction_permille(), 1000);
        assert_eq!(tp.eta_ms, Some(0));
    }
}
