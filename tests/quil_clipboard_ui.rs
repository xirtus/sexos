// SPDX-License-Identifier: GPL-3.0-only
//! Actual Quil editor, client-event and undo functions extracted by the gate.
//! Host callbacks replace only rendering, document actions and timer acquisition.
//! This is no native IPC authority, durable-save or pixel proof.
#![allow(dead_code, unused_variables, static_mut_refs)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/text_encoding.rs"]
pub mod text_encoding;
#[macro_export]
macro_rules! serial_record {
    ($($x:tt)*) => {
        ()
    };
}
macro_rules! serial_println {
    ($($x:tt)*) => {
        ()
    };
}
#[path = "../crates/sex-pdx/src/app_key.rs"]
pub mod app_key;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
pub mod app_lifecycle;
#[path = "../crates/sex-pdx/src/clipboard.rs"]
pub mod clipboard;
#[path = "../crates/sex-pdx/src/clipboard_client.rs"]
pub mod clipboard_client;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[path = "../crates/sex-pdx/src/document_view.rs"]
pub mod document_view;
#[path = "../crates/sex-pdx/src/text_selection.rs"]
pub mod text_selection;
#[path = "../servers/quil/src/view.rs"]
pub mod view;
#[path = "../servers/quil/src/unicode_input.rs"]
pub mod unicode_input;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
pub fn get_ticks() -> u64 {
    1
}
const QUIL_BUFFER_MAX_LEN: usize = 12288;
const UNDO_DEPTH: usize = 6;
static mut QUIL_BUFFER: [u8; 12288] = [0; 12288];
static mut QUIL_BUFFER_LEN: usize = 0;
static mut QUIL_UNICODE_INPUT: unicode_input::Input = unicode_input::Input::new();
static mut QUIL_UNICODE_SNAPSHOT: Option<text_selection::Snapshot> = None;
static mut QUIL_INPUT_NOTICE: Option<&'static [u8]> = None;
static mut QUIL_CURSOR_POS: usize = 0;
static mut QUIL_REVISION: u64 = 0;
static mut QUIL_GENERATION: u64 = 1;
static mut QUIL_CLIPBOARD: clipboard_client::Client = clipboard_client::Client::new();
static mut QUIL_SELECTION: text_selection::Selection = text_selection::Selection::new();
static mut QUIL_COPY_SNAPSHOT: Option<(u32, text_selection::Snapshot)> = None;
static mut QUIL_PASTE_SNAPSHOT: Option<(u32, text_selection::Snapshot)> = None;
static mut QUIL_CLIPBOARD_NOTICE: bool = false;
static mut QUIL_CLIPBOARD_ERROR: Option<&'static [u8]> = None;
static mut QUIL_GOAL_COL: usize = usize::MAX;
static mut QUIL_DIRTY: bool = false;
static mut DIRTY: bool = false;
static mut UNDO_RING: [[u8; 12288]; 6] = [[0; 12288]; 6];
static mut UNDO_LENS: [usize; 6] = [0; 6];
static mut UNDO_CURSORS: [usize; 6] = [0; 6];
static mut UNDO_HEAD: usize = 0;
static mut UNDO_COUNT: usize = 0;
static mut UNDO_REDO_COUNT: usize = 0;
struct DocumentIo;
impl DocumentIo {
    fn busy(&self) -> bool {
        false
    }
}
static mut DOCUMENT_IO: DocumentIo = DocumentIo;
static mut QUIL_NAME_MODE: bool = false;
static mut SHIFT_HELD: bool = false;
static mut DOCUMENT_VIEW: document_view::Layout = document_view::Layout {
    w: 640,
    h: 480,
    scale: 2,
    cols: 51,
    rows: 25,
    top: 12,
    toolbar_rows: 1,
};
static mut ACTION: Option<u8> = None;
static mut PALETTE_CALLS: u32 = 0;
fn document_action(action: u8, _: &mut bool, _: &mut u8) {
    unsafe { ACTION = Some(action) }
}
fn quil_dispatch_palette_key(_: u64, _: u64, _: &mut bool, _: &mut u8) {
    unsafe { PALETTE_CALLS += 1 }
}
fn draw_text_lines(_: &[u8]) {}
unsafe fn reset(bytes: &[u8]) {
    QUIL_UNICODE_INPUT.clear();QUIL_UNICODE_SNAPSHOT=None;QUIL_INPUT_NOTICE=None;
    QUIL_BUFFER.fill(0);
    QUIL_BUFFER[..bytes.len()].copy_from_slice(bytes);
    QUIL_BUFFER_LEN = bytes.len();
    QUIL_CURSOR_POS = bytes.len();
    QUIL_GENERATION = 1;
    QUIL_REVISION = 0;
    UNDO_COUNT = 0;
    UNDO_REDO_COUNT = 0;
    UNDO_HEAD = 0;
    QUIL_DIRTY = false;
    QUIL_SELECTION.clear();
    QUIL_COPY_SNAPSHOT = None;
    QUIL_PASTE_SNAPSHOT = None;
    QUIL_CLIPBOARD_NOTICE = false;
    QUIL_CLIPBOARD_ERROR = None;
    QUIL_CLIPBOARD = clipboard_client::Client::new();
    QUIL_CLIPBOARD.bind(clipboard::Participant {
        subject: clipboard::Subject { pd: 9, task: 701 },
        surface: 156,
        route: 41,
        can_read: true,
    });
    quil_selection_bind();
}

include!(env!("QUIL_CLIPBOARD_UI_SOURCE"));

fn key(code: u8, shift: bool, control: bool) {
    quil_dispatch_key(
        app_key::Key {
            code,
            shift,
            control,
        },
        &mut false,
        &mut 0,
    )
}
fn reply(r: clipboard::Response) {
    let p = r.wire();
    unsafe {
        assert!(QUIL_CLIPBOARD.receive(clipboard::SHELL, p.opcode, p.args, 1));
    }
}
fn admitted() {
    unsafe {
        QUIL_CLIPBOARD.poll(1, |_| clipboard_client::Admission::Accepted);
    }
}
fn incoming(ticket: u32, bytes: &[u8], before_finish: impl FnOnce()) {
    reply(clipboard::Response::Intent {
        ticket,
        action: clipboard::Action::Paste,
    });
    quil_clipboard_events(false);
    admitted();
    reply(clipboard::Response::Meta {
        ticket,
        phase: clipboard::Phase::Read,
        status: clipboard::Status::Ok,
        len: bytes.len(),
        generation: 19,
        hash: clipboard::hash(bytes),
    });
    for (offset, chunk) in bytes.chunks(16).enumerate() {
        admitted();
        let mut data = [0; 16];
        data[..chunk.len()].copy_from_slice(chunk);
        reply(clipboard::Response::Data {
            ticket,
            offset: offset * 16,
            count: chunk.len(),
            bytes: data,
        });
    }
    admitted();
    before_finish();
    reply(clipboard::Response::Meta {
        ticket,
        phase: clipboard::Phase::Finish,
        status: clipboard::Status::Ok,
        len: bytes.len(),
        generation: 19,
        hash: clipboard::hash(bytes),
    });
    quil_clipboard_events(false);
}
#[test]
fn select_all_replacement_is_one_undo_revision_and_restored_bytes_stay_new_revision() {
    unsafe {
        reset(b"original");
        let old = QUIL_SELECTION.snapshot().unwrap();
        QUIL_COPY_SNAPSHOT = Some((1, old));
        key(0x1e, false, true);
        assert_eq!(QUIL_SELECTION.range(), Some((0, 8)));
        key(0x2d, false, false);
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"x");
        assert_eq!(UNDO_COUNT, 1);
        assert_eq!(QUIL_REVISION, 1);
        assert!(QUIL_DIRTY);
        assert!(QUIL_COPY_SNAPSHOT.is_none());
        key(0x2c, false, true);
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"original");
        assert_eq!(QUIL_REVISION, 2);
        assert!(!QUIL_SELECTION.unchanged(old));
        assert_eq!(UNDO_REDO_COUNT, 1);
        key(0x2c, true, true);
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"x");
        assert_eq!(QUIL_REVISION, 3);
    }
}
#[test]
fn usb_and_ps2_shift_selection_and_save_as_use_complete_key_snapshot() {
    unsafe {
        reset(b"AB");
        key(105, true, false);
        assert_eq!(QUIL_SELECTION.range(), Some((1, 2)));
        key(0x4b, true, false);
        assert_eq!(QUIL_SELECTION.range(), Some((0, 2)));
        key(106, false, false);
        assert_eq!(QUIL_SELECTION.range(), Some((2, 2)));
        assert_eq!(QUIL_CURSOR_POS, 2);
        key(0x1f, true, true);
        assert_eq!(ACTION, Some(document_view::SAVE_AS));
        key(0x1f, false, true);
        assert_eq!(ACTION, Some(document_view::SAVE));
        for code in [0x2e, 0x2d, 0x2f, 0x10] {
            key(code, false, true);
            assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"AB");
            assert_eq!(QUIL_REVISION, 0);
        }
    }
}
#[test]
fn new_document_identity_differs_even_when_revision_and_bytes_match() {
    unsafe {
        reset(b"same");
        let old = QUIL_SELECTION.snapshot().unwrap();
        let generation = QUIL_GENERATION;
        quil_fresh_source();
        assert!(QUIL_GENERATION > generation);
        assert_eq!(QUIL_REVISION, 0);
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"same");
        assert!(!QUIL_SELECTION.unchanged(old));
        assert_eq!(UNDO_COUNT, 0);
    }
}
#[test]
fn capacity_rejection_preserves_whole_body_and_has_visible_reason_then_erases() {
    unsafe {
        reset(&[b'A'; 12288]);
        let original = QUIL_BUFFER;
        incoming(1, b"X", || {});
        assert_eq!(QUIL_BUFFER, original);
        assert_eq!(QUIL_REVISION, 0);
        assert_eq!(UNDO_COUNT, 0);
        assert_eq!(quil_clipboard_status(), Some(&b"PASTE TOO LARGE"[..]));
        assert!(QUIL_CLIPBOARD.bytes().is_none());
        assert!(QUIL_CLIPBOARD.storage_erased());
    }
}
#[test]
fn accepted_paste_while_user_edits_cannot_overwrite_new_revision() {
    unsafe {
        reset(b"before");
        incoming(1, b"after", || key(0x1e, false, false));
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"beforea");
        assert_eq!(QUIL_REVISION, 1);
        assert_eq!(UNDO_COUNT, 1);
        assert_eq!(quil_clipboard_status(), Some(&b"SELECTION CHANGED"[..]));
        assert!(QUIL_CLIPBOARD.bytes().is_none());
        assert!(QUIL_CLIPBOARD.storage_erased());
    }
}
#[test]
fn full_12k_selected_replacement_pastes_whole_and_undoes_in_one_step() {
    unsafe {
        reset(&[b'A'; 12288]);
        key(0x1e, false, true);
        incoming(1, &[b'B'; 12288], || {});
        assert!(QUIL_BUFFER.iter().all(|&b| b == b'B'));
        assert_eq!(QUIL_REVISION, 1);
        assert_eq!(UNDO_COUNT, 1);
        assert!(QUIL_CLIPBOARD.storage_erased());
        key(0x2c, false, true);
        assert!(QUIL_BUFFER.iter().all(|&b| b == b'A'));
        assert_eq!(QUIL_REVISION, 2);
    }
}
fn cut(before_publish: impl FnOnce()) {
    unsafe {
        let source = quil_source();
        QUIL_SELECTION.move_to(source, 1, false).unwrap();
        QUIL_SELECTION.move_to(source, 4, true).unwrap();
        QUIL_CURSOR_POS = 4;
        reply(clipboard::Response::Intent {
            ticket: 1,
            action: clipboard::Action::Cut,
        });
        quil_clipboard_events(false);
        admitted();
        reply(clipboard::Response::Meta {
            ticket: 1,
            phase: clipboard::Phase::Begin,
            status: clipboard::Status::Ok,
            len: 3,
            generation: 0,
            hash: 0,
        });
        admitted();
        before_publish();
        reply(clipboard::Response::Meta {
            ticket: 1,
            phase: clipboard::Phase::Publish,
            status: clipboard::Status::Ok,
            len: 3,
            generation: 9,
            hash: clipboard::hash(b"bcd"),
        });
        quil_clipboard_events(false);
    }
}
#[test]
fn cut_only_deletes_unchanged_exact_selection_after_positive_publish() {
    unsafe {
        reset(b"abcdef");
        cut(|| {});
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"aef");
        assert_eq!(QUIL_CURSOR_POS, 1);
        assert_eq!(QUIL_REVISION, 1);
        assert_eq!(UNDO_COUNT, 1);
        assert!(QUIL_CLIPBOARD.storage_erased());
        key(0x2c, false, true);
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"abcdef");
    }
}
#[test]
fn publication_ack_after_user_edit_reports_copy_without_cutting_new_bytes() {
    unsafe {
        reset(b"abcdef");
        cut(|| {
            assert!(quil_replace((6, 6), b"x"));
        });
        assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN], b"abcdefx");
        assert_eq!(QUIL_REVISION, 1);
        assert_eq!(UNDO_COUNT, 1);
        assert_eq!(quil_clipboard_status(), Some(&b"COPIED / CUT SKIPPED"[..]));
        assert!(QUIL_CLIPBOARD.storage_erased());
    }
}
#[test]
fn sequence_exhaustion_fences_without_partial_edit_or_identity_reuse() {
    unsafe {
        reset(b"safe");
        QUIL_REVISION = u64::MAX;
        let original = QUIL_BUFFER;
        assert!(!quil_replace((0, 4), b"unsafe"));
        assert_eq!(QUIL_BUFFER, original);
        assert_eq!(UNDO_COUNT, 0);
        QUIL_GENERATION = u64::MAX;
        quil_fresh_source();
        assert_eq!(QUIL_GENERATION, 0);
        assert_eq!(QUIL_SELECTION.snapshot(), None);
        assert!(!quil_replace((0, 4), b"x"));
    }
}


#[test]
fn unicode_entry_is_one_atomic_revision_and_undo_restores_exact_utf8() { unsafe {
    reset("aé".as_bytes());
    key(0x16,true,true); // Ctrl Shift U
    assert!(QUIL_UNICODE_INPUT.active());
    for c in [0x02,0x21,0x07,0x05,0x03] { key(c,false,false); } // 1F642
    assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé".as_bytes());
    key(0x1c,false,false);
    assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé🙂".as_bytes());
    assert_eq!(QUIL_REVISION,1);assert_eq!(UNDO_COUNT,1);assert!(!QUIL_UNICODE_INPUT.active());
    key(0x2c,false,true);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé".as_bytes());
    key(0x2c,true,true);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé🙂".as_bytes());
}}
#[test]
fn utf8_left_right_backspace_delete_and_shift_selection_never_split_a_scalar() { unsafe {
    reset("Aé€🙂Z".as_bytes());
    for want in [10,6,3,1,0,0] {key(0x4b,false,false);assert_eq!(QUIL_CURSOR_POS,want);}
    for want in [1,3,6,10,11,11] {key(0x4d,false,false);assert_eq!(QUIL_CURSOR_POS,want);}
    key(0x4b,true,false);key(0x4b,true,false);
    assert_eq!(QUIL_SELECTION.range(),Some((6,11)));
    key(0x0e,false,false);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"Aé€".as_bytes());
    key(0x0e,false,false);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"Aé".as_bytes());
    key(0x47,false,false);key(0x4d,false,false);key(0x53,false,false);
    assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],b"A");
    assert!(text_encoding::valid_text(&QUIL_BUFFER[..QUIL_BUFFER_LEN]));
}}
#[test]
fn unicode_invalid_cancel_capacity_and_changed_snapshot_have_no_partial_effect() { unsafe {
    reset(b"draft");key(0x16,true,true);
    for c in [0x20,0x09,0x0b,0x0b] {key(c,false,false);} // surrogate D800
    key(0x1c,false,false);assert!(QUIL_UNICODE_INPUT.active());assert_eq!(QUIL_REVISION,0);
    key(0x01,false,false);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],b"draft");
    key(0x16,true,true);key(0x12,false,false);key(0x0a,false,false); // E9
    assert!(quil_replace((5,5),b"!"));
    key(0x1c,false,false);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],b"draft!");assert_eq!(QUIL_REVISION,1);
    reset(&vec![b'a';12287]);key(0x16,true,true);key(0x12,false,false);key(0x0a,false,false);key(0x1c,false,false);
    assert_eq!(QUIL_BUFFER_LEN,12287);assert_eq!(QUIL_REVISION,0);assert_eq!(UNDO_COUNT,0);
}}
#[test]
fn mid_scalar_replacement_is_rejected_before_undo_and_tabs_are_exact() { unsafe {
    reset("aé€z".as_bytes());
    for range in [(2,3),(1,2),(3,4),(4,6)] {assert!(!quil_replace(range,b"q"));}
    assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé€z".as_bytes());assert_eq!(QUIL_REVISION,0);assert_eq!(UNDO_COUNT,0);
    key(0x0f,false,false);assert_eq!(&QUIL_BUFFER[..QUIL_BUFFER_LEN],"aé€z\t".as_bytes());
}}

#[test]
fn unicode_digit_overflow_requires_recovery_and_cannot_commit_a_silent_prefix() {
    let mut input=unicode_input::Input::new();input.begin();
    for code in [0x0b,0x0b,0x0b,0x0b,0x12,0x0a] {assert_eq!(input.key(code),unicode_input::Action::Pending);}
    assert_eq!(input.key(0x1e),unicode_input::Action::Refused);
    assert_eq!(input.key(0x1c),unicode_input::Action::Refused);
    let (prompt,n)=input.prompt();assert!(core::str::from_utf8(&prompt[..n]).unwrap().contains("invalid"));
    input.key(0x0e);input.key(0x0a);
    match input.key(0x1c) {unicode_input::Action::Commit{bytes,len}=>assert_eq!(&bytes[..len],"é".as_bytes()),other=>panic!("{other:?}")}
    input.begin();input.key(0x12);input.clear();assert!(!input.active());
    input.begin();assert_eq!(input.key(0x1c),unicode_input::Action::Refused);
    assert_eq!(input.key(0x01),unicode_input::Action::Cancelled);
}
