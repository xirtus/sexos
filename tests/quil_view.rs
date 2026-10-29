#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY: u64 = 2;
#[path = "../servers/quil/src/display_delivery.rs"]
mod delivery;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[path = "../crates/sex-pdx/src/document_view.rs"]
pub mod document_view;
#[path = "../servers/quil/src/view.rs"]
mod view;
use document_view::Layout;
#[test]
fn caret_stays_in_bounded_visible_text_for_long_documents_and_small_tiles() {
    for target in [9, 16] {
        for (w, h) in [(1280, 700), (640, 335), (496, 319), (220, 150), (144, 90)] {
            let l = Layout::new(target, w, h);
            for buf in [vec![b'a'; 12288], b"line\n".repeat(1800)] {
                let mut top = 0;
                let t = view::render(
                    &buf,
                    buf.len(),
                    &mut top,
                    l,
                    b"long document",
                    true,
                    b"SAVING",
                    None,
                    None,
                );
                assert!(t.len <= 512);
                if l.body_rows() > 0 {
                    assert!(t.bytes[..t.len].contains(&b'#'), "{target} {w} {h}");
                }
                for line in t.bytes[..t.len].split(|&b| b == b'\n') {
                    assert!(line.len() <= l.cols, "{w} {}", line.len());
                }
            }
        }
    }
}
#[test]
fn toolbar_hit_uses_same_metrics_and_excludes_header_and_editor() {
    for w in [144, 220, 496, 624] {
        let l = Layout::new(9, w, 320);
        let per = 4 / l.toolbar_rows;
        for i in 0..4 {
            let x = 8 + (i % per * 5 + 1) as u32 * 6 * l.scale;
            let y = l.top + (1 + i / per) as u32 * 9 * l.scale + 1;
            assert_eq!(l.hit(x, y), Some(i as u8 + 1));
        }
        assert_eq!(l.hit(8, l.top), None);
        assert_eq!(
            l.hit(8, l.top + (1 + l.toolbar_rows) as u32 * 9 * l.scale),
            None
        );
    }
}
#[test]
fn notices_require_exact_sender_incarnation_task_and_fresh_sequence() {
    use document_view::Notice;
    let n = Notice {
        target: 9,
        task: 9,
        w: 624,
        h: 291,
        sequence: 4,
    };
    assert!(Notice::decode(3, 9, 9, n.args(), 3).is_some());
    assert!(Notice::decode(11, 9, 9, n.args(), 3).is_none());
    assert!(Notice::decode(3, 16, 16, n.args(), 3).is_none());
    assert!(Notice::decode(3, 9, 9, n.args(), 4).is_none());
}
#[test]
fn display_full_retains_exact_active_batch_then_latest_and_close_fences() {
    let mut o = delivery::Outbox::new();
    let a = view::render(
        b"old",
        3,
        &mut 0,
        Layout::new(9, 624, 291),
        b"note",
        true,
        b"SAVING",
        None,
        None,
    );
    o.queue(a, 1);
    let mut first = None;
    o.poll(
        156,
        |op, sid, a, b| {
            first = Some((op, sid, a, b));
            Err(ERR_SERVICE_NOT_READY)
        },
        |_, _| panic!(),
    );
    let b = view::render(
        b"new",
        3,
        &mut 0,
        Layout::new(9, 624, 291),
        b"note",
        false,
        b"READY",
        None,
        None,
    );
    o.queue(b, 2);
    let mut completed = vec![];
    let mut retried = false;
    for _ in 0..100 {
        o.poll(
            156,
            |op, sid, a, b| {
                if !retried {
                    assert_eq!(Some((op, sid, a, b)), first);
                    retried = true;
                }
                Ok(())
            },
            |paint, _| completed.push(paint),
        );
        if !o.pending() {
            break;
        }
    }
    assert_eq!(completed, vec![1, 2]);
    o.queue(a, 3);
    o.close();
    assert!(!o.pending());
    o.poll(156, |_, _, _, _| panic!(), |_, _| panic!());
}
