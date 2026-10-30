#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
#[path = "../servers/quil/src/display_delivery.rs"]
mod delivery;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[path = "../crates/sex-pdx/src/document_view.rs"]
pub mod document_view;
#[path = "../crates/sex-pdx/src/text_batch.rs"]
pub mod text_batch;
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
        if let Some(&paint) = completed.last() {
            o.receive(
                1,
                text_batch::Applied {
                    surface: 156,
                    sequence: paint as u32,
                }
                .encode()
                .unwrap(),
            );
        }
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

#[test]
fn vertical_navigation_has_no_768_line_limit() {
    let buf = vec![b'\n'; 12_288];
    let mut goal = usize::MAX;
    assert_eq!(
        view::vertical(&buf, buf.len(), 51, &mut goal, true),
        Some(12_287)
    );
    assert_eq!(
        view::vertical(&buf, 12_287, 51, &mut goal, false),
        Some(12_288)
    );
    assert_eq!(view::vertical(&buf, 0, 51, &mut goal, true), None);
    assert_eq!(view::vertical(&buf, buf.len(), 51, &mut goal, false), None);
}
#[test]
fn vertical_sticky_column_and_soft_wrap_stay_on_target_line() {
    let mut goal = usize::MAX;
    let buf = b"abcde\nx\nabcde";
    let short = view::vertical(buf, 4, 51, &mut goal, false).unwrap();
    assert_eq!(short, 7);
    assert_eq!(view::vertical(buf, short, 51, &mut goal, false), Some(12));
    // width=4: position 7 on second line moves to column 3, not the
    // following line's start at byte4. Existing stored bytes are unchanged.
    goal = usize::MAX;
    assert_eq!(
        view::vertical(b"abcdefghij", 7, 5, &mut goal, true),
        Some(3)
    );
    assert_eq!(
        view::vertical(b"abcdefghij", 3, 5, &mut goal, false),
        Some(7)
    );
}

#[path = "../servers/sexdisplay/src/text_transaction.rs"]
mod text_transaction;
#[test]
fn outbox_end_to_end_keeps_old_text_until_commit_at_every_full_boundary() {
    // Real client and server state machines, only the delivery result is fake.
    for len in [0, 1, 512] {
        for full_at in 0..(2 + (len + 7) / 8) {
            let mut t = view::Text::new();
            t.len = len;
            t.bytes[..len].fill(b'x');
            let mut out = delivery::Outbox::new();
            out.queue(t, 1);
            let mut stage = text_transaction::Stage::EMPTY;
            let mut visible = b"previous".to_vec();
            let mut admitted = vec![];
            let mut delivered = 0;
            let mut full = false;
            while out.pending() {
                out.poll(
                    156,
                    |op, sid, a, b| {
                        assert_eq!(sid, 156);
                        if delivered == full_at && !full {
                            full = true;
                            return Err(ERR_SERVICE_NOT_READY);
                        }
                        let req = text_batch::Request::decode(op, a, b).unwrap();
                        if let Some((n, _, bytes)) = stage.receive(9, sid, req) {
                            visible = bytes[..n].to_vec();
                        } else {
                            assert_eq!(visible, b"previous");
                        }
                        delivered += 1;
                        Ok(())
                    },
                    |paint, n| admitted.push((paint, n)),
                );
                stage.poll_reply(|target, value| {
                    assert_eq!(target, 9);
                    assert_eq!(out.receive(1, value), Some((1, len)));
                    0
                });
            }
            assert!(full);
            assert_eq!(visible, vec![b'x'; len]);
            assert_eq!(admitted, vec![(1, len)]);
        }
    }
}
#[test]
fn exhausted_paint_never_wraps_or_publishes_a_reused_transaction() {
    let mut out = delivery::Outbox::new();
    let t = view::Text::new();
    out.queue(t, u32::MAX as u64 + 1);
    assert!(!out.pending());
    out.queue(t, 1);
    out.poll(156, |_, _, _, _| panic!(), |_, _| panic!());
}
#[test]
fn only_exact_display_receipt_releases_batch_and_latest_coalesces_while_waiting() {
    let mut out = delivery::Outbox::new();
    let mut t = view::Text::new();
    t.len = 1;
    t.bytes[0] = b'a';
    out.queue(t, 1);
    let mut sent = vec![];
    out.poll(
        156,
        |op, _, _, _| {
            sent.push(op);
            Ok(())
        },
        |paint, _| assert_eq!(paint, 1),
    );
    assert_eq!(
        sent,
        [text_batch::BEGIN, text_batch::CHUNK, text_batch::COMMIT]
    );
    for paint in 2..100 {
        t.bytes[0] = b'z';
        out.queue(t, paint);
    }
    out.poll(
        156,
        |_, _, _, _| panic!("must await receipt"),
        |_, _| panic!(),
    );
    let exact = text_batch::Applied {
        surface: 156,
        sequence: 1,
    }
    .encode()
    .unwrap();
    for (caller, value) in [
        (3, exact),
        (1, exact | 1 << 56),
        (
            1,
            text_batch::Applied {
                surface: 301,
                sequence: 1,
            }
            .encode()
            .unwrap(),
        ),
        (
            1,
            text_batch::Applied {
                surface: 156,
                sequence: 2,
            }
            .encode()
            .unwrap(),
        ),
    ] {
        assert_eq!(out.receive(caller, value), None);
    }
    assert_eq!(out.receive(1, exact), Some((1, 1)));
    assert_eq!(out.receive(1, exact), None);
    out.poll(
        156,
        |op, _, a, b| {
            if op == text_batch::BEGIN {
                assert!(matches!(
                    text_batch::Request::decode(op, a, b),
                    Some(text_batch::Request::Begin { sequence: 99, .. })
                ));
            }
            Ok(())
        },
        |paint, _| assert_eq!(paint, 99),
    );
    out.close();
    assert_eq!(
        out.receive(
            1,
            text_batch::Applied {
                surface: 156,
                sequence: 99
            }
            .encode()
            .unwrap()
        ),
        None
    );
}
#[test]
fn ack_then_new_queue_supersedes_older_latest_and_never_replays_behind_display() {
    let mut out = delivery::Outbox::new();
    let mut stage = text_transaction::Stage::EMPTY;
    let mut t = view::Text::new();
    t.len = 1;
    t.bytes[0] = b'a';
    out.queue(t, 1);
    out.poll(
        156,
        |op, sid, a, b| {
            stage.receive(9, sid, text_batch::Request::decode(op, a, b).unwrap());
            Ok(())
        },
        |_, _| {},
    );
    t.bytes[0] = b'b';
    out.queue(t, 2);
    stage.poll_reply(|_, value| {
        assert_eq!(out.receive(1, value), Some((1, 1)));
        0
    });
    // The real main loop may process another input before its next poll.
    t.bytes[0] = b'c';
    out.queue(t, 3);
    let mut completed = vec![];
    out.poll(
        156,
        |op, sid, a, b| {
            if let Some((n, _, bytes)) =
                stage.receive(9, sid, text_batch::Request::decode(op, a, b).unwrap())
            {
                assert_eq!(&bytes[..n], b"c");
            }
            Ok(())
        },
        |paint, _| completed.push(paint),
    );
    stage.poll_reply(|_, value| {
        assert_eq!(out.receive(1, value), Some((3, 1)));
        0
    });
    out.poll(
        156,
        |_, _, _, _| panic!("superseded paint must not reappear"),
        |_, _| panic!(),
    );
    assert_eq!(completed, [3]);
    assert!(!out.pending());
    out.queue(t, 2);
    out.queue(t, 3);
    assert!(!out.pending());
}

#[test]
fn third_geometry_binds_catalog_task_without_domain_task_alias() {
    use document_view::{Delivery,Notice};
    let mut d=Delivery::new();d.queue(18,180,320,300);
    let mut n=None;d.poll(true,true,|v|{n=Some(v);Ok(())});let n=n.unwrap();
    assert!(Notice::decode(3,18,180,n.args(),0).is_some());
    assert!(Notice::decode(3,18,18,n.args(),0).is_none());
    assert!(Notice::decode(16,18,180,n.args(),0).is_none());
    assert_eq!(Layout::new(18,320,300).top,36);
}

#[test]fn recreated_binding_discards_obsolete_upload_and_old_ack_without_reusing_sequence(){
 for(target,sid)in[(9,156),(16,301),(18,307)]{for cut in 1..=12{
  let mut out=delivery::Outbox::new();let mut stage=text_transaction::Stage::EMPTY;
  let mut old=view::Text::new();old.len=80;old.bytes[..80].fill(b'a');out.queue(old,10);
  let mut operations=0;let mut admitted=false;
  for _ in 0..5{out.poll(sid,|op,sid,a,b|{operations+=1;stage.receive(target,sid,text_batch::Request::decode(op,a,b).unwrap());if operations==cut{stage.reset();}Ok(())},|_,_|admitted=true);}
  assert!(admitted);assert!(out.pending());
  out.rebind();assert!(!out.pending());
  let receipt=text_batch::Applied{surface:sid as u16,sequence:10}.encode().unwrap();
  assert_eq!(out.receive(1,receipt),None);
  let mut new=view::Text::new();new.len=3;new.bytes[..3].copy_from_slice(b"new");out.queue(new,11);
  // A queued/replayed old paint cannot return after the new binding notice.
  out.queue(old,10);
  let mut commits=0;let mut full=false;
  for _ in 0..6{out.poll(sid,|op,sid,a,b|{
   if !full{full=true;return Err(ERR_SERVICE_NOT_READY)}
   if let Some((n,_,bytes))=stage.receive(target,sid,text_batch::Request::decode(op,a,b).unwrap()){assert_eq!(&bytes[..n],b"new");commits+=1;}
   Ok(())
  },|paint,_|assert_eq!(paint,11));}
  assert_eq!(out.receive(1,receipt),None);
  stage.poll_reply(|pd,v|{assert_eq!(pd,target);assert_eq!(out.receive(1,v),Some((11,3)));0});
  assert_eq!(commits,1);assert!(!out.pending());
  out.close();out.rebind();out.queue(new,12);out.poll(sid,|_,_,_,_|panic!(),|_,_|panic!());
 }}
}

#[test]
fn browser_geometry_binds_actual_task_and_retains_until_ready() {
    use document_view::{Delivery, Notice};
    let mut delivery = Delivery::new();
    delivery.queue(15, 150, 320, 240);
    delivery.poll(true, false, |_| panic!("not ready"));
    assert!(delivery.pending());
    let mut first = None;
    delivery.poll(true, true, |n| { first = Some(n); Err(ERR_SERVICE_NOT_READY) });
    assert!(delivery.pending());
    let first = first.unwrap();
    delivery.poll(true, true, |n| { assert_eq!(n, first); Ok(()) });
    assert!(!delivery.pending());
    assert!(Notice::decode(3, 15, 150, first.args(), 0).is_some());
    for (caller, target, task, last) in [(3,15,15,0),(15,15,150,0),(3,16,150,0),(3,15,0,0),(3,15,150,first.sequence)] {
        assert!(Notice::decode(caller, target, task, first.args(), last).is_none());
    }
    assert_eq!(Layout::new(15,320,240).top,36);
    delivery.queue(15,150,220,150);
    delivery.poll(false,true,|_|panic!("terminal subject"));
    delivery.queue(15,150,640,480);
    assert!(!delivery.pending());
}
