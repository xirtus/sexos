// Host tests for kernel/src/views_rules.rs (scripts/protection_views_host_gate.sh):
// the same page-table rules the kernel runs, on tables in ordinary memory.
#[path = "../kernel/src/views_rules.rs"]
mod rules;
use rules::*;
use std::collections::HashMap;

#[derive(Default)]
struct Memory { tables: HashMap<u64, [u64; 512]>, next: u64 }
impl Memory {
    fn table(&mut self) -> u64 { self.next += 0x1000; self.tables.insert(self.next, [0; 512]); self.next }
}
impl Tables for Memory {
    fn read(&self, table: u64, index: usize) -> u64 { self.tables[&table][index] }
    fn write(&mut self, table: u64, index: usize, value: u64) { self.tables.get_mut(&table).unwrap()[index] = value; }
    fn copy(&mut self, from: u64) -> Option<u64> { let t = self.tables[&from]; let c = self.table(); self.tables.insert(c, t); Some(c) }
}
const LINK: u64 = PRESENT | 2 | USER;
const CLOSED: u64 = PRESENT | 2;

/// A canonical root mapping `spans` (span-aligned addresses) closed, each with its own
/// page table holding one leaf.
fn canonical(m: &mut Memory, spans: &[u64]) -> u64 {
    let root = m.table();
    for &va in spans {
        let mut t = root;
        for level in [3, 2] {
            let i = index(va, level);
            if m.read(t, i) & PRESENT == 0 { let n = m.table(); m.write(t, i, n | LINK); }
            t = m.read(t, i) & FRAME;
        }
        let pt = m.table();
        m.write(pt, 0, 0xdead_000 | PRESENT | USER);
        m.write(t, index(va, 1), pt | CLOSED);
    }
    root
}
/// What user mode may reach at `va` through `root`: every level grants user.
fn user(m: &Memory, root: u64, va: u64) -> bool {
    let mut t = root;
    for level in [3, 2, 1] {
        let e = m.read(t, index(va, level));
        if e & PRESENT == 0 || e & USER == 0 { return false; }
        t = e & FRAME;
    }
    true
}
fn leaf_table(m: &Memory, root: u64, va: u64) -> u64 {
    let mut t = root;
    for level in [3, 2, 1] { t = m.read(t, index(va, level)) & FRAME; }
    t
}

const A: u64 = 0x4000_0000_0000;
const B: u64 = 0x4000_0020_0000; // the next span, same page directory
const C: u64 = 0x7000_2c00_0000; // another root slot

#[test]
fn a_view_opens_only_its_spans_and_shares_page_tables() {
    let mut m = Memory::default();
    let root = canonical(&mut m, &[A, B, C]);
    let view = m.copy(root).unwrap();
    let mut made = Vec::new();
    assert_eq!(open(&mut m, root, view, A + 0x1234, &mut |_, c, p| made.push((c, p))), Ok(true));
    assert_eq!(made.len(), 2, "a PDPT and a page directory are copied");
    assert!(user(&m, view, A));
    assert!(!user(&m, view, B) && !user(&m, view, C), "other spans stay closed");
    assert!(!user(&m, root, A), "the canonical tables keep the span closed");
    for va in [A, B, C] { assert_eq!(leaf_table(&m, root, va), leaf_table(&m, view, va), "page tables are shared"); }
}

#[test]
fn a_second_span_under_the_same_directory_copies_nothing() {
    let mut m = Memory::default();
    let root = canonical(&mut m, &[A, B]);
    let view = m.copy(root).unwrap();
    open(&mut m, root, view, A, &mut |_, _, _| {}).unwrap();
    let mut made = 0;
    open(&mut m, root, view, B, &mut |_, _, _| made += 1).unwrap();
    assert_eq!(made, 0);
    assert!(user(&m, view, A) && user(&m, view, B));
}

#[test]
fn two_views_never_see_each_others_spans() {
    let mut m = Memory::default();
    let root = canonical(&mut m, &[A, B, C]);
    let (one, two) = (m.copy(root).unwrap(), m.copy(root).unwrap());
    open(&mut m, root, one, A, &mut |_, _, _| {}).unwrap();
    open(&mut m, root, two, B, &mut |_, _, _| {}).unwrap();
    open(&mut m, root, two, C, &mut |_, _, _| {}).unwrap();
    assert!(user(&m, one, A) && !user(&m, one, B) && !user(&m, one, C));
    assert!(!user(&m, two, A) && user(&m, two, B) && user(&m, two, C));
}

#[test]
fn a_view_older_than_a_new_root_slot_still_opens_it() {
    let mut m = Memory::default();
    let root = canonical(&mut m, &[A]);
    let view = m.copy(root).unwrap();
    // The view's root is older than the canonical PDPT entry for a new slot.
    let late = 0x2000_0000_0000;
    let mut t = root;
    for level in [3, 2] { let n = m.table(); m.write(t, index(late, level), n | LINK); t = n; }
    let pt = m.table();
    m.write(t, index(late, 1), pt | CLOSED);
    m.write(view, index(late, 3), 0);
    assert_eq!(open(&mut m, root, view, late, &mut |_, _, _| {}), Ok(true));
    assert!(user(&m, view, late));
    let fresh = canonical(&mut m, &[]);
    let fresh_view = m.copy(fresh).unwrap();
    assert_eq!(open(&mut m, fresh, fresh_view, A, &mut |_, _, _| {}), Err(()), "an unmapped span cannot open");
}

#[test]
fn merged_keeps_private_copies_and_openings() {
    let (c, p) = (0x5000 | CLOSED, 0x9000 | LINK);
    assert_eq!(merged(c, 0), c, "a new canonical entry is taken");
    assert_eq!(merged(0x5000 | CLOSED, 0x5000 | CLOSED | USER), 0x5000 | CLOSED | USER, "the view keeps its opening");
    assert_eq!(merged(c | (1 << 5), p), 0x9000 | CLOSED | (1 << 5), "a private copy keeps its frame, takes canonical flags");
}

#[test]
fn mirror_after_a_new_table_reaches_the_view() {
    let mut m = Memory::default();
    let root = canonical(&mut m, &[A]);
    let view = m.copy(root).unwrap();
    let mut copies = vec![(root, view)];
    open(&mut m, root, view, A, &mut |_, c, p| copies.push((c, p))).unwrap();
    // Canonical gains a page table for B under the directory the view copied.
    let pt = m.table();
    let mut dir = root;
    for level in [3, 2] { dir = m.read(dir, index(B, level)) & FRAME; }
    m.write(dir, index(B, 1), pt | CLOSED);
    for &(c, p) in copies.iter().filter(|(c, _)| *c == dir) {
        let v = merged(m.read(c, index(B, 1)), m.read(p, index(B, 1)));
        m.write(p, index(B, 1), v);
    }
    assert_eq!(leaf_table(&m, view, B), pt);
    assert!(user(&m, view, A) && !user(&m, view, B));
}

#[test]
fn cr3_values_and_stale_marks() {
    let root = 0x12_3000;
    assert_eq!(cr3(root, 3, false, false), root, "without PCIDs a view is its root");
    assert_eq!(cr3(root, 3, true, false), root | 3 | NOFLUSH);
    assert_eq!(cr3(root, 3, true, true), root | 3);
    let stale = stale_after_invalidate(2);
    assert_eq!(stale & (1 << 2), 0, "the current view was invalidated in place");
    assert_eq!(stale & 0b1011, 0b1011);
}

#[test]
fn reaching_names_only_the_entries_a_mapping_touches() {
    assert_eq!(reaching(1, A, A, A + 1), index(A, 1)..index(A, 1) + 1);
    assert_eq!(reaching(1, A, A, A + SPAN + 1), index(A, 1)..index(A, 1) + 2);
    assert_eq!(reaching(1, A, C, C + 1), 0..0, "another directory's range");
    assert_eq!(reaching(3, 0, C, C + 4096), index(C, 3)..index(C, 3) + 1);
    assert_eq!(reaching(2, A, A - 1, A + 1), 0..1, "clipped at the table's start");
}
