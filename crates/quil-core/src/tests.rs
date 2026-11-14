// SPDX-License-Identifier: GPL-3.0-only
use crate::buffer::{Buffer, Source, BLOCK};
use crate::editor::{Editor, Motion, Sel};
use crate::fuzzy;
use crate::highlight::{self, Kind, Lang};
use crate::keymap::{Command, Keymap, Keys, Lookup, Named, Press, K};
use crate::project::{self, Entry, Fs, Query, Tree};
use crate::vim::{Action, Mode, VKey, Vim};
use crate::workspace::{Tab, Tabs};
use std::boxed::Box;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::string::String;
use std::vec::Vec;
use std::{format, vec};

fn s(ed: &Editor) -> String { String::from_utf8(ed.text()).unwrap() }

// ---- Buffer ----

#[test]
fn buffer_matches_a_plain_vec_through_random_edits() {
    let mut model: Vec<u8> = b"hello\nworld\n".to_vec();
    let mut b = Buffer::from_bytes(&model);
    let mut seed = 12345u64;
    let mut rnd = |n: usize| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed as usize) % n.max(1) };
    for step in 0..3000 {
        let len = model.len();
        let a = rnd(len + 1);
        let e = (a + rnd(8)).min(len);
        let text: Vec<u8> = (0..rnd(6)).map(|i| if (i + step) % 5 == 0 { b'\n' } else { b'a' + (rnd(26) as u8) }).collect();
        model.splice(a..e, text.iter().copied());
        b.replace(a..e, &text);
        assert_eq!(b.len(), model.len());
        if step % 97 == 0 {
            assert_eq!(b.to_vec(), model, "step {step}");
            let nl = model.iter().filter(|c| **c == b'\n').count();
            assert_eq!(b.line_count(), Some(nl + 1));
            let at = rnd(len + 1).min(model.len());
            let want = model[..at].iter().filter(|c| **c == b'\n').count();
            assert_eq!(b.line_of(at), Some(want));
            let ln = rnd(nl + 1);
            let want_off = if ln == 0 { 0 } else { model.iter().enumerate().filter(|x| *x.1 == b'\n').nth(ln - 1).map(|x| x.0 + 1).unwrap() };
            assert_eq!(b.offset_of_line(ln), Some(want_off), "line {ln}");
        }
    }
    assert_eq!(b.to_vec(), model);
}

#[test]
fn snapshots_restore_the_text_exactly() {
    let mut b = Buffer::from_bytes(b"abc");
    let s0 = b.snapshot();
    b.insert(1, b"XYZ");
    let s1 = b.snapshot();
    b.delete(0..4);
    assert_eq!(b.to_vec(), b"bc");
    b.restore(&s1);
    assert_eq!(b.to_vec(), b"aXYZbc");
    b.restore(&s0);
    assert_eq!(b.to_vec(), b"abc");
}

/// A file that only exists as a formula, counting how many bytes were read from it.
struct Synthetic { len: usize, reads: Rc<Cell<usize>> }
fn synth_byte(i: usize) -> u8 { if i % 80 == 79 { b'\n' } else { b'a' + (i % 26) as u8 } }
impl Source for Synthetic {
    fn len(&self) -> usize { self.len }
    fn read(&self, at: usize, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len.saturating_sub(at));
        for (k, o) in out[..n].iter_mut().enumerate() { *o = synth_byte(at + k); }
        self.reads.set(self.reads.get() + n);
        n
    }
}

#[test]
fn a_one_gigabyte_file_opens_jumps_to_the_end_and_edits_reading_only_a_few_blocks() {
    let reads = Rc::new(Cell::new(0));
    let len = 1usize << 30;
    let mut ed = Editor::new(Buffer::new(Box::new(Synthetic { len, reads: reads.clone() })));
    assert_eq!(reads.get(), 0, "opening reads nothing");
    ed.move_all(Motion::DocEnd, false);
    assert_eq!(ed.cursor(), len);
    ed.type_text(b"THE END");
    ed.move_all(Motion::DocStart, false);
    ed.type_text(b"START ");
    assert!(reads.get() <= 4 * BLOCK, "read {} bytes", reads.get());
    assert_eq!(ed.buf.len(), len + 13);
    assert_eq!(ed.buf.slice(0..8), b"START ab");
    assert_eq!(ed.buf.slice(len + 6..len + 13), b"THE END");
    // Line numbers are unknown until the index has seen every block, then exact.
    assert_eq!(ed.buf.line_count(), None);
    while !ed.buf.index_step(1024) {}
    assert_eq!(ed.buf.line_count(), Some(len / 80 + 1));
    ed.undo();
    assert_eq!(ed.buf.slice(0..6), b"abcdef");
}

#[test]
fn a_200_mb_file_saves_byte_for_byte() {
    let reads = Rc::new(Cell::new(0));
    let len = 200 << 20;
    let mut ed = Editor::new(Buffer::new(Box::new(Synthetic { len, reads })));
    ed.move_all(Motion::DocEnd, false);
    ed.type_text(b"\nend\n");
    // Stream it out as a save would, hashing as we go, and compare with the formula.
    let (mut h, mut n) = (0xcbf29ce484222325u64, 0usize);
    ed.buf.chunks::<()>(|c| { for b in c { h = (h ^ *b as u64).wrapping_mul(0x100000001b3); } n += c.len(); Ok(()) }).unwrap();
    let mut want = 0xcbf29ce484222325u64;
    for i in 0..len { want = (want ^ synth_byte(i) as u64).wrapping_mul(0x100000001b3); }
    for b in b"\nend\n" { want = (want ^ *b as u64).wrapping_mul(0x100000001b3); }
    assert_eq!((n, h), (len + 5, want));
}

#[test]
fn find_and_rfind_cross_block_edges() {
    let mut t = vec![b'x'; BLOCK * 3];
    t[BLOCK - 2..BLOCK + 3].copy_from_slice(b"Hello");
    t[2 * BLOCK + 10..2 * BLOCK + 15].copy_from_slice(b"hello");
    let b = Buffer::from_bytes(&t);
    assert_eq!(b.find(b"Hello", 0, false), Some(BLOCK - 2));
    assert_eq!(b.find(b"hello", 0, false), Some(2 * BLOCK + 10));
    assert_eq!(b.find(b"hello", 0, true), Some(BLOCK - 2));
    assert_eq!(b.rfind(b"hello", b.len(), true), Some(2 * BLOCK + 10));
    assert_eq!(b.rfind(b"hello", 2 * BLOCK + 10, true), Some(BLOCK - 2));
    assert_eq!(b.rfind(b"Hello", BLOCK - 2, false), None);
}

#[test]
fn characters_and_lines_handle_utf8() {
    let b = Buffer::from_bytes("aé€😀\nz".as_bytes());
    assert_eq!(b.next_char(1), 3);
    assert_eq!(b.next_char(3), 6);
    assert_eq!(b.next_char(6), 10);
    assert_eq!(b.prev_char(10), 6);
    assert_eq!(b.prev_char(6), 3);
    assert_eq!(b.column(10), 4);
    assert_eq!(b.at_column(11, 5), 12);
    assert_eq!(b.char_at(6), Some('😀'));
}

// ---- Editor ----

#[test]
fn several_cursors_type_and_delete_together() {
    let mut ed = Editor::from_bytes(b"one\ntwo\nthree\n");
    ed.add_cursor(true);
    ed.add_cursor(true);
    assert_eq!(ed.sels.len(), 3);
    ed.type_text(b"> ");
    assert_eq!(s(&ed), "> one\n> two\n> three\n");
    ed.move_all(Motion::End, false);
    ed.type_text(b";");
    assert_eq!(s(&ed), "> one;\n> two;\n> three;\n");
    ed.backspace();
    ed.backspace();
    assert_eq!(s(&ed), "> on\n> tw\n> thre\n");
    ed.undo();
    assert_eq!(s(&ed), "> one;\n> two;\n> three;\n");
    ed.collapse();
    assert_eq!(ed.sels.len(), 1);
}

#[test]
fn ctrl_d_selects_the_word_then_each_next_occurrence() {
    let mut ed = Editor::from_bytes(b"let foo = foo + bar(foo);");
    ed.set_cursor(5);
    assert!(ed.add_next_occurrence());
    assert_eq!(ed.selected(&ed.primary_sel()), b"foo");
    assert!(ed.add_next_occurrence());
    assert!(ed.add_next_occurrence());
    assert!(!ed.add_next_occurrence());
    assert_eq!(ed.sels.len(), 3);
    ed.type_text(b"x");
    assert_eq!(s(&ed), "let x = x + bar(x);");
}

#[test]
fn select_all_occurrences_and_paste_one_line_per_cursor() {
    let mut ed = Editor::from_bytes(b"a1 a2 a3");
    ed.set_selection(0, 1);
    assert_eq!(ed.select_all_occurrences(), 3);
    ed.paste(b"X\nY\nZ", false);
    assert_eq!(s(&ed), "X1 Y2 Z3");
}

#[test]
fn column_selection_with_shift_extends_every_line() {
    let mut ed = Editor::from_bytes(b"abcd\nefgh\nijkl\n");
    ed.set_selection(1, 3);
    ed.add_cursor(true);
    ed.add_cursor(true);
    let picked: Vec<Vec<u8>> = ed.sels.iter().map(|x| ed.selected(x)).collect();
    assert_eq!(picked, vec![b"bc".to_vec(), b"fg".to_vec(), b"jk".to_vec()]);
    ed.delete_forward();
    assert_eq!(s(&ed), "ad\neh\nil\n");
}

#[test]
fn split_selection_into_lines() {
    let mut ed = Editor::from_bytes(b"aa\nbb\ncc");
    ed.select_all();
    ed.split_into_lines();
    assert_eq!(ed.sels.len(), 3);
    ed.move_all(Motion::End, false);
    ed.type_text(b",");
    assert_eq!(s(&ed), "aa,\nbb,\ncc,");
}

#[test]
fn line_commands() {
    let mut ed = Editor::from_bytes(b"fn a() {\n    x();\n}\n");
    ed.set_cursor(10);
    ed.toggle_comment(b"//");
    assert_eq!(s(&ed), "fn a() {\n    // x();\n}\n");
    ed.toggle_comment(b"//");
    assert_eq!(s(&ed), "fn a() {\n    x();\n}\n");
    ed.duplicate_lines();
    assert_eq!(s(&ed), "fn a() {\n    x();\n    x();\n}\n");
    ed.delete_lines();
    assert_eq!(s(&ed), "fn a() {\n    x();\n}\n");
    ed.set_cursor(10);
    ed.move_lines(false);
    assert_eq!(s(&ed), "    x();\nfn a() {\n}\n");
    ed.move_lines(true);
    assert_eq!(s(&ed), "fn a() {\n    x();\n}\n");
    ed.outdent_lines();
    assert_eq!(s(&ed), "fn a() {\nx();\n}\n");
    ed.indent_lines();
    assert_eq!(s(&ed), "fn a() {\n    x();\n}\n");
    ed.set_cursor(0);
    ed.join_lines();
    assert_eq!(s(&ed), "fn a() { x();\n}\n");
}

#[test]
fn enter_keeps_indent_and_typing_undoes_as_one_step() {
    let mut ed = Editor::from_bytes(b"    let a;");
    ed.move_all(Motion::DocEnd, false);
    ed.type_text(b"\n");
    for c in b"let b;" { ed.type_text(&[*c]); }
    assert_eq!(s(&ed), "    let a;\n    let b;");
    ed.undo();
    assert_eq!(s(&ed), "    let a;\n    ");
    ed.undo();
    assert_eq!(s(&ed), "    let a;");
    ed.redo();
    ed.redo();
    assert_eq!(s(&ed), "    let a;\n    let b;");
    assert!(ed.modified());
}

#[test]
fn copy_without_selection_copies_the_line_and_pastes_it_above() {
    let mut ed = Editor::from_bytes(b"one\ntwo\n");
    ed.set_cursor(5);
    let (t, whole) = ed.copy();
    assert_eq!((t.as_slice(), whole), (&b"two\n"[..], true));
    ed.set_cursor(1);
    ed.paste(&t, whole);
    assert_eq!(s(&ed), "two\none\ntwo\n");
}

#[test]
fn word_motions_and_smart_home() {
    let mut ed = Editor::from_bytes(b"  foo.bar baz");
    ed.move_all(Motion::End, false);
    ed.move_all(Motion::WordLeft, false);
    assert_eq!(ed.cursor(), 10);
    ed.move_all(Motion::WordLeft, false);
    assert_eq!(ed.cursor(), 6);
    ed.move_all(Motion::Home, false);
    assert_eq!(ed.cursor(), 2);
    ed.move_all(Motion::Home, false);
    assert_eq!(ed.cursor(), 0);
    ed.move_all(Motion::WordRight, true);
    assert_eq!(ed.primary_sel(), Sel { anchor: 0, head: 2, col: None });
}

// ---- Vim ----

fn vim(text: &str, at: usize, keys: &str) -> (Editor, Vim) {
    let mut ed = Editor::from_bytes(text.as_bytes());
    ed.set_cursor(at);
    let mut v = Vim::new();
    feed(&mut v, &mut ed, keys);
    (ed, v)
}
/// Keys as Vim writes them: <Esc>, <CR>, <C-r>, <BS>.
fn feed(v: &mut Vim, ed: &mut Editor, keys: &str) -> Vec<Action> {
    let mut acts = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        let (k, n) = if let Some(r) = rest.strip_prefix("<Esc>") { (VKey::Esc, keys.len() - r.len()) }
            else if let Some(r) = rest.strip_prefix("<CR>") { (VKey::Enter, keys.len() - r.len()) }
            else if let Some(r) = rest.strip_prefix("<BS>") { (VKey::Backspace, keys.len() - r.len()) }
            else if rest.starts_with("<C-") { let c = rest.chars().nth(3).unwrap(); (VKey::Ctrl(c), keys.len() - rest.len() + 5) }
            else { let c = rest.chars().next().unwrap(); (VKey::Char(c), keys.len() - rest.len() + c.len_utf8()) };
        let a = v.key(ed, k);
        if a != Action::None { acts.push(a); }
        rest = &keys[n..];
    }
    acts
}

#[test]
fn vim_dd_deletes_a_line_and_p_puts_it_back() {
    let (ed, _) = vim("one\ntwo\nthree\n", 5, "dd");
    assert_eq!(s(&ed), "one\nthree\n");
    let (ed, _) = vim("one\ntwo\nthree\n", 0, "ddp");
    assert_eq!(s(&ed), "two\none\nthree\n");
    let (ed, _) = vim("one\ntwo\nthree", 9, "dd");
    assert_eq!(s(&ed), "one\ntwo");
    let (ed, _) = vim("a\nb\nc\nd\n", 0, "2dd");
    assert_eq!(s(&ed), "c\nd\n");
    let (ed, _) = vim("a\nb\nc\nd\n", 2, "yyjp");
    assert_eq!(s(&ed), "a\nb\nc\nb\nd\n");
}

#[test]
fn vim_operators_with_motions_and_text_objects() {
    assert_eq!(s(&vim("foo bar baz", 0, "dw").0), "bar baz");
    assert_eq!(s(&vim("foo bar baz", 0, "d2w").0), "baz");
    assert_eq!(s(&vim("foo bar baz", 4, "cwXY<Esc>").0), "foo XY baz");
    assert_eq!(s(&vim("foo bar baz", 5, "ciwQ<Esc>").0), "foo Q baz");
    assert_eq!(s(&vim("foo bar baz", 5, "daw").0), "foo baz");
    assert_eq!(s(&vim("call(a, b)", 6, "di(").0), "call()");
    assert_eq!(s(&vim("x = \"hello\";", 6, "ci\"bye<Esc>").0), "x = \"bye\";");
    assert_eq!(s(&vim("abc def", 1, "D").0), "a");
    assert_eq!(s(&vim("abc def", 0, "x").0), "bc def");
    assert_eq!(s(&vim("abc def", 0, "3x").0), " def");
    assert_eq!(s(&vim("a,b,c", 0, "dt,").0), ",b,c");
    assert_eq!(s(&vim("a,b,c", 0, "df,").0), "b,c");
    assert_eq!(s(&vim("one two", 0, "de").0), " two");
    assert_eq!(s(&vim("one two", 4, "d0").0), "two");
    assert_eq!(s(&vim("x\n    y\n", 0, ">>").0), "    x\n    y\n");
    assert_eq!(s(&vim("    x\n", 4, "<<").0), "x\n");
}

#[test]
fn vim_insert_commands_and_dot_repeat() {
    assert_eq!(s(&vim("ab", 0, "ix<Esc>").0), "xab");
    assert_eq!(s(&vim("ab", 0, "ax<Esc>").0), "axb");
    assert_eq!(s(&vim("  ab", 3, "I-<Esc>").0), "  -ab");
    assert_eq!(s(&vim("ab", 0, "A;<Esc>").0), "ab;");
    assert_eq!(s(&vim("  ab", 0, "onew<Esc>").0), "  ab\n  new");
    assert_eq!(s(&vim("ab", 0, "Onew<Esc>").0), "new\nab");
    assert_eq!(s(&vim("a b c d", 0, "x..").0), " c d");
    assert_eq!(s(&vim("one\ntwo\nthree\n", 0, "A!<Esc>j.j.").0), "one!\ntwo!\nthree!\n");
    assert_eq!(s(&vim("foo foo foo", 0, "cwbar<Esc>w.w.").0), "bar bar bar");
    assert_eq!(s(&vim("a\nb\nc\n", 0, "dd.").0), "c\n");
}

#[test]
fn vim_undo_takes_a_whole_change() {
    let (ed, _) = vim("foo bar", 0, "cwhello<Esc>u");
    assert_eq!(s(&ed), "foo bar");
    let (ed, _) = vim("foo bar", 0, "cwhello<Esc>u<C-r>");
    assert_eq!(s(&ed), "hello bar");
}

#[test]
fn vim_motions_counts_and_finds() {
    let (ed, _) = vim("one two three four", 0, "2w");
    assert_eq!(ed.cursor(), 8);
    let (ed, _) = vim("one two three four", 0, "$");
    assert_eq!(ed.cursor(), 17);
    let (ed, _) = vim("one two three four", 17, "b0");
    assert_eq!(ed.cursor(), 0);
    let (ed, _) = vim("a\nb\nc\nd\n", 0, "G");
    assert_eq!(ed.cursor(), 8);
    let (ed, _) = vim("a\nb\nc\nd\n", 6, "gg");
    assert_eq!(ed.cursor(), 0);
    let (ed, _) = vim("a\nb\nc\nd\n", 0, "3G");
    assert_eq!(ed.cursor(), 4);
    let (ed, _) = vim("a.b.c.d", 0, "f.;;");
    assert_eq!(ed.cursor(), 5);
    let (ed, _) = vim("a.b.c.d", 6, "F.;");
    assert_eq!(ed.cursor(), 3);
    let (ed, _) = vim("f(a[b]c)", 1, "%");
    assert_eq!(ed.cursor(), 7);
    let (ed, _) = vim("abc\nabcdef\nab", 2, "jj");
    assert_eq!(ed.cursor(), 12, "j clamps to the line's last char");
}

#[test]
fn vim_visual_modes() {
    assert_eq!(s(&vim("hello world", 0, "vllld").0), "o world");
    assert_eq!(s(&vim("hello world", 0, "veyP").0), "hellohello world");
    assert_eq!(s(&vim("a\nb\nc\n", 0, "Vjd").0), "c\n");
    assert_eq!(s(&vim("abc", 0, "vlU").0), "ABc");
    assert_eq!(s(&vim("one\ntwo\n", 0, "Vj>").0), "    one\n    two\n");
    // Ctrl+V then j makes a cursor per line; I types on each.
    assert_eq!(s(&vim("ab\ncd\nef\n", 0, "<C-v>jjI# <Esc>").0), "# ab\n# cd\n# ef\n");
}

#[test]
fn vim_search_and_ex_commands() {
    let (ed, _) = vim("alpha beta alpha gamma", 0, "/alpha<CR>");
    assert_eq!(ed.cursor(), 11);
    let (ed, _) = vim("alpha beta alpha gamma", 0, "/alpha<CR>n");
    assert_eq!(ed.cursor(), 0);
    let (ed, _) = vim("ab ab ab", 0, "*");
    assert_eq!(ed.cursor(), 3);
    let (ed, _) = vim("a b a\na\n", 0, ":%s/a/X/g<CR>");
    assert_eq!(s(&ed), "X b X\nX\n");
    let (ed, _) = vim("a b a\na\n", 0, ":s/a/X/<CR>");
    assert_eq!(s(&ed), "X b a\na\n");
    let (ed, _) = vim("1\n2\n3\n", 0, ":3<CR>");
    assert_eq!(ed.cursor(), 4);
    let mut ed = Editor::from_bytes(b"x");
    let mut v = Vim::new();
    assert_eq!(feed(&mut v, &mut ed, ":w<CR>"), vec![Action::Save]);
    assert_eq!(feed(&mut v, &mut ed, ":wq<CR>"), vec![Action::SaveQuit]);
    feed(&mut v, &mut ed, "x");
    assert!(matches!(feed(&mut v, &mut ed, ":q<CR>")[0], Action::Message(_)));
    assert_eq!(feed(&mut v, &mut ed, ":q!<CR>"), vec![Action::ForceQuit]);
    assert_eq!(v.mode, Mode::Normal);
}

#[test]
fn vim_works_with_several_cursors() {
    let mut ed = Editor::from_bytes(b"foo 1\nfoo 2\nfoo 3\n");
    ed.add_cursor(true);
    ed.add_cursor(true);
    let mut v = Vim::new();
    feed(&mut v, &mut ed, "cwbar<Esc>");
    assert_eq!(s(&ed), "bar 1\nbar 2\nbar 3\n");
    feed(&mut v, &mut ed, "A!<Esc>");
    assert_eq!(s(&ed), "bar 1!\nbar 2!\nbar 3!\n");
}

// ---- Keymaps ----

#[test]
fn vscode_and_emacs_keys() {
    let mut k = Keys::default();
    assert_eq!(k.lookup(Keymap::VsCode, Press::ctrl_shift('p')), Lookup::Run(Command::Palette));
    assert_eq!(k.lookup(Keymap::VsCode, Press::ctrl('d')), Lookup::Run(Command::AddNextOccurrence));
    assert_eq!(k.lookup(Keymap::VsCode, Press { k: K::Named(Named::Down), ctrl: true, shift: true }), Lookup::Run(Command::AddCursorBelow));
    assert_eq!(k.lookup(Keymap::Emacs, Press::ctrl('x')), Lookup::Pending);
    assert_eq!(k.lookup(Keymap::Emacs, Press::ctrl('s')), Lookup::Run(Command::Save));
    assert_eq!(k.lookup(Keymap::Emacs, Press::ctrl('s')), Lookup::Run(Command::Find));
    assert_eq!(k.lookup(Keymap::Emacs, Press::named(Named::Esc)), Lookup::Pending);
    assert_eq!(k.lookup(Keymap::Emacs, Press::ch('x')), Lookup::Run(Command::Palette));
    assert_eq!(k.lookup(Keymap::Emacs, Press::ctrl_shift('p')), Lookup::Run(Command::Palette));
    // Every command has a palette name, and names are unique.
    let mut names: Vec<&str> = Command::ALL.iter().map(|c| c.name()).collect();
    names.sort();
    let n = names.len();
    names.dedup();
    assert_eq!(names.len(), n);
}

// ---- Fuzzy ----

#[test]
fn fuzzy_prefers_file_names_word_starts_and_runs() {
    let files = ["src/main.rs", "src/buffer.rs", "crates/quil-core/src/buffer.rs", "docs/build.md", "apps/quil/src/main.rs", "README.md"];
    let r = fuzzy::rank("buf", &files, 10);
    assert_eq!(files[r[0].0], "src/buffer.rs");
    let r = fuzzy::rank("qmain", &files, 10);
    assert_eq!(files[r[0].0], "apps/quil/src/main.rs");
    assert!(fuzzy::score("xyz", b"src/main.rs").is_none());
    let cmds: Vec<&str> = Command::ALL.iter().map(|c| c.name()).collect();
    let r = fuzzy::rank("togcom", &cmds, 3);
    assert_eq!(cmds[r[0].0], "Edit: Toggle Line Comment");
    let r = fuzzy::rank("save", &cmds, 3);
    assert_eq!(cmds[r[0].0], "File: Save");
    let (_, pos) = fuzzy::score("mr", b"main.rs").unwrap();
    assert_eq!(pos, vec![0, 5]);
}

// ---- Project ----

#[derive(Default)]
struct MemFs { files: BTreeMap<Vec<u8>, Vec<u8>> }
impl Fs for MemFs {
    fn list(&mut self, path: &[u8]) -> Option<Vec<Entry>> {
        let prefix = if path.is_empty() { Vec::new() } else { let mut p = path.to_vec(); p.push(b'/'); p };
        let mut seen = BTreeMap::new();
        for (k, v) in &self.files {
            if !k.starts_with(&prefix) { continue; }
            let rest = &k[prefix.len()..];
            match rest.iter().position(|b| *b == b'/') {
                Some(i) => { seen.insert(rest[..i].to_vec(), (true, 0)); }
                None => { seen.insert(rest.to_vec(), (false, v.len() as u64)); }
            }
        }
        if seen.is_empty() && !path.is_empty() { return None; }
        Some(seen.into_iter().map(|(name, (dir, size))| Entry { name, dir, size }).collect())
    }
    fn read(&mut self, path: &[u8]) -> Option<Vec<u8>> { self.files.get(path).cloned() }
    fn write(&mut self, path: &[u8], data: &[u8]) -> bool { self.files.insert(path.to_vec(), data.to_vec()); true }
}

fn sample() -> MemFs {
    let mut fs = MemFs::default();
    for (p, t) in [("proj/src/main.rs", "fn main() {\n    hello();\n}\n"), ("proj/src/lib.rs", "pub fn hello() {}\n// Hello again\n"),
                   ("proj/README.md", "Say hello.\n"), ("proj/target/debug/out", "hello build output\n"), ("proj/.git/HEAD", "hello\n"),
                   ("proj/bin.dat", "hello\0binary")] {
        fs.files.insert(p.as_bytes().to_vec(), t.as_bytes().to_vec());
    }
    fs
}

#[test]
fn project_walk_skips_build_and_vcs_folders() {
    let mut fs = sample();
    let files: Vec<String> = project::walk(&mut fs, b"proj", 100).into_iter().map(|(p, _)| String::from_utf8(p).unwrap()).collect();
    assert_eq!(files, vec!["README.md", "bin.dat", "src/lib.rs", "src/main.rs"]);
}

#[test]
fn project_search_and_replace() {
    let mut fs = sample();
    let q = Query { needle: b"hello".to_vec(), fold_case: false, whole_word: true };
    let (hits, _) = project::search(&mut fs, b"proj", &q, 100);
    let got: Vec<(String, usize, usize)> = hits.iter().map(|h| (String::from_utf8(h.path.clone()).unwrap(), h.line, h.col)).collect();
    assert_eq!(got, vec![("README.md".into(), 0, 4), ("src/lib.rs".into(), 0, 7), ("src/main.rs".into(), 1, 4)]);
    let q2 = Query { needle: b"hello".to_vec(), fold_case: true, whole_word: false };
    assert_eq!(project::search(&mut fs, b"proj", &q2, 100).0.len(), 4);
    let (files, n) = project::replace_all(&mut fs, b"proj", &q, b"greet");
    assert_eq!((files, n), (3, 3));
    assert_eq!(fs.files[&b"proj/src/main.rs"[..]], b"fn main() {\n    greet();\n}\n");
    assert_eq!(fs.files[&b"proj/target/debug/out"[..]], b"hello build output\n");
}

#[test]
fn project_search_covers_ten_thousand_files_in_under_a_second() {
    let mut fs = MemFs::default();
    for i in 0..10_000 {
        let body = format!("// file {i}\nfn f{i}() -> u32 {{ {i} }}\n{}", if i % 1000 == 7 { "let needle_here = 1;\n" } else { "" });
        fs.files.insert(format!("p/m{}/f{i}.rs", i % 50).into_bytes(), body.into_bytes());
    }
    let t = std::time::Instant::now();
    let (hits, read) = project::search(&mut fs, b"p", &Query { needle: b"needle_here".to_vec(), ..Query::default() }, 1000);
    let took = t.elapsed();
    assert_eq!((hits.len(), read), (10, 10_000));
    assert!(took.as_millis() < 1000, "took {took:?}");
    let names: Vec<Vec<u8>> = project::walk(&mut fs, b"p", 20_000).into_iter().map(|x| x.0).collect();
    let t = std::time::Instant::now();
    let r = fuzzy::rank("m7f1007", &names, 20);
    assert!(t.elapsed().as_millis() < 1000);
    assert_eq!(names[r[0].0], b"m7/f1007.rs");
}

#[test]
fn file_tree_opens_folders_and_reveals_files() {
    let mut fs = sample();
    let mut t = Tree::new(b"proj");
    t.refresh(&mut fs);
    let names: Vec<&[u8]> = t.rows.iter().map(|r| &r.name[..]).collect();
    assert_eq!(names, vec![&b"src"[..], b"target", b"bin.dat", b"README.md"]);
    t.selected = 0;
    assert_eq!(t.activate(&mut fs), None);
    assert_eq!(t.rows.len(), 6);
    t.down();
    assert_eq!(t.activate(&mut fs), Some(b"src/lib.rs".to_vec()));
    let mut t2 = Tree::new(b"proj");
    t2.reveal(&mut fs, b"target/debug/out");
    assert_eq!(t2.rows[t2.selected].path, b"target/debug/out");
}

// ---- Highlight and tabs ----

#[test]
fn highlighting_marks_keywords_strings_and_block_comments() {
    let mut out = Vec::new();
    let cont = highlight::line(Lang::Rust, b"let s = \"hi\"; /* start", false, &mut out);
    assert!(cont);
    assert_eq!(out[0], (0, 3, Kind::Keyword));
    assert!(out.contains(&(8, 12, Kind::String)));
    let cont = highlight::line(Lang::Rust, b"end */ fn", true, &mut out);
    assert!(!cont);
    assert_eq!(out[0], (0, 6, Kind::Comment));
    assert_eq!(out[1], (7, 9, Kind::Keyword));
    highlight::line(Lang::Python, b"def f(): # note", false, &mut out);
    assert_eq!(out[0], (0, 3, Kind::Keyword));
    assert_eq!(*out.last().unwrap(), (9, 15, Kind::Comment));
    assert_eq!(Lang::from_path(b"a/b.rs"), Lang::Rust);
}

#[test]
fn tabs_open_close_and_cycle() {
    let mut t = Tabs::default();
    t.add(Tab::new(b"a.rs", Editor::from_bytes(b"a")));
    t.add(Tab::new(b"b.rs", Editor::from_bytes(b"b")));
    t.add(Tab::new(b"c.rs", Editor::from_bytes(b"c")));
    assert_eq!(t.active, 2);
    assert_eq!(t.find(b"a.rs"), Some(0));
    t.next();
    assert_eq!(t.active, 0);
    t.prev();
    t.close(2);
    assert_eq!(t.active, 1);
    assert_eq!(t.current().unwrap().name(), b"b.rs");
    t.shift(false);
    assert_eq!(t.list[0].name(), b"b.rs");
}

#[test]
fn view_follows_the_cursor() {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let mut tab = Tab::new(b"x.txt", Editor::from_bytes(text.as_bytes()));
    tab.ed.set_cursor(tab.ed.buf.offset_of_line(50).unwrap());
    tab.follow(10);
    assert_eq!(tab.ed.buf.line_of(tab.top), Some(41));
    tab.ed.set_cursor(0);
    tab.follow(10);
    assert_eq!(tab.top, 0);
}
