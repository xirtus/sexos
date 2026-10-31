#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path = "../crates/sex-pdx/src/text_encoding.rs"]
pub mod text_encoding;
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
        let per = l.toolbar_per_row();
        for i in 0..4 {
            let x = 8 + (i % per * document_view::TOOLBAR_CELL + 1) as u32 * 6 * l.scale;
            let y = l.top + (l.header_rows() + i / per) as u32 * 9 * l.scale + 1;
            assert_eq!(l.hit(x, y), Some(i as u8 + 1));
        }
        assert_eq!(l.hit(8, l.top), None);
        assert_eq!(
            l.hit(8, l.top + (l.header_rows() + l.toolbar_rows) as u32 * 9 * l.scale),
            None
        );
    }
}
#[test]
fn every_drawable_short_view_keeps_a_caret_row_and_hides_nonfitting_controls() {
    for w in 1..=512 { for h in 1..=180 {
        let l=Layout::new(9,w,h);
        if l.rows>0 {assert!(l.body_rows()>0,"{w} {h}");}
        assert!(l.chrome_rows()+l.body_rows()<=l.rows);
        assert_eq!(l.hit(w,l.top),None);assert_eq!(l.hit(8,h),None);
        if l.toolbar_rows==0 {
            for y in 0..h {assert_eq!(l.hit(8,y),None,"{w} {h} {y}");}
        } else {
            let per=l.toolbar_per_row();assert!(per*document_view::TOOLBAR_CELL<=l.cols);
            for i in 0..4 {
                let x=8+(i%per*document_view::TOOLBAR_CELL)as u32*6*l.scale;
                let y=l.top+(l.header_rows()+i/per)as u32*9*l.scale;
                assert!(y+7*l.scale<=h);assert_eq!(l.hit(x,y),Some(i as u8+1));
            }
        }
    }}
}
#[test]
fn short_and_single_cell_rendering_never_exceeds_physical_rows_or_columns() {
    for target in [9,16,18] {for (w,h)in[(1,80),(16,80),(22,19),(22,50),(80,45),(144,45),(144,90),(496,319),(624,335)] {
        let l=Layout::new(target,w,h);
        for body in [b"A readable document with spaces\nand a second line".as_slice(),b"\n\n\nx\n\n",b""] {
        for cursor in [0,body.len()/2,body.len()] {
            let mut top=0;let text=view::render_selection(body,cursor,&mut top,l,b"long document name",true,b"CLIPBOARD UNCERTAIN",None,None,Some((0,body.len())));
            assert!(text.len<=512);
            if l.rows==0 {assert_eq!(text.len,0);continue;}
            assert!(text.bytes[..text.len].contains(&b'#'),"{target} {w} {h}");
            let rows=text.bytes[..text.len].iter().filter(|b|**b==b'\n').count()+usize::from(!text.bytes[..text.len].ends_with(b"\n"));
            assert!(rows<=l.rows,"{target} {w} {h}: {rows} > {}",l.rows);
            for line in text.bytes[..text.len].split(|b|*b==b'\n'){assert!(line.len()<=l.cols,"{target} {w} {h}: {} > {}",line.len(),l.cols);}
        }
    }}}
}
#[test]
fn command_menu_keeps_each_selected_command_visible_in_short_views() {
    for target in [9,16] {for (w,h)in[(22,19),(80,45),(144,45),(144,90),(496,319)] {
        let l=Layout::new(target,w,h);
        for selected in 0..5 {
            let mut top=7;let t=view::render(b"document",3,&mut top,l,b"note",false,b"READY",Some(selected),None);
            assert_eq!(top,7);
            if l.rows==0 {assert_eq!(t.len,0);continue;}
            assert_eq!(t.bytes[..t.len].iter().filter(|b|**b==b'>').count(),1,"{target} {w} {h} selected={selected}");
            let lines:Vec<_>=t.bytes[..t.len].split(|b|*b==b'\n').filter(|s|!s.is_empty()).collect();assert!(lines.len()<=l.rows);
            for line in lines {assert!(line.len()<=l.cols);}
        }
    }}
}
#[test]
fn normal_desktop_chrome_exposes_complete_shortcuts_and_matching_buttons() {
    for (w,h)in[(496,319),(624,335),(624,706)] {
        let l=Layout::new(9,w,h);let t=view::render(b"body",4,&mut 0,l,b"note",false,b"READY",None,None);
        let text=core::str::from_utf8(&t.bytes[..t.len]).unwrap();
        assert!(text.contains("New     Open    Save    Save As "));
        assert!(text.contains("Ctrl N/O/S | Ctrl Shift S | Esc menu"));
        let menu=view::render(b"body",4,&mut 0,l,b"note",false,b"READY",Some(4),None);
        let menu=core::str::from_utf8(&menu.bytes[..menu.len]).unwrap();
        assert!(menu.contains("> Save As  Ctrl Shift S"));
        assert!(menu.contains("Ctrl A select all"));assert!(menu.contains("Ctrl C/X/V copy/cut/paste"));
    }
}
#[test]
fn arbitrary_header_lengths_are_bounded_without_changing_document_bytes() {
    let body=b"document bytes";let source=*body;let l=Layout::new(9,496,319);
    for naming in [None,Some(&[b'n';200][..])] {
        let t=view::render(body,body.len(),&mut 0,l,&[b'x';200],true,&[b'Y';200],None,naming);
        assert!(t.len<=512);assert_eq!(*body,source);
        for line in t.bytes[..t.len].split(|b|*b==b'\n'){assert!(line.len()<=l.cols);}
        assert!(t.bytes[..t.len].contains(&b'#'));
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
    use document_view::{Delivery, Notice};
    let mut d = Delivery::new();
    d.queue(18, 180, 320, 300);
    let mut n = None;
    d.poll(true, true, |v| {
        n = Some(v);
        Ok(())
    });
    let n = n.unwrap();
    assert!(Notice::decode(3, 18, 180, n.args(), 0).is_some());
    assert!(Notice::decode(3, 18, 18, n.args(), 0).is_none());
    assert!(Notice::decode(16, 18, 180, n.args(), 0).is_none());
    assert_eq!(Layout::new(18, 320, 300).top, 36);
}

#[test]
fn recreated_binding_discards_obsolete_upload_and_old_ack_without_reusing_sequence() {
    for (target, sid) in [(9, 156), (16, 301), (18, 307)] {
        for cut in 1..=12 {
            let mut out = delivery::Outbox::new();
            let mut stage = text_transaction::Stage::EMPTY;
            let mut old = view::Text::new();
            old.len = 80;
            old.bytes[..80].fill(b'a');
            out.queue(old, 10);
            let mut operations = 0;
            let mut admitted = false;
            for _ in 0..5 {
                out.poll(
                    sid,
                    |op, sid, a, b| {
                        operations += 1;
                        stage.receive(target, sid, text_batch::Request::decode(op, a, b).unwrap());
                        if operations == cut {
                            stage.reset();
                        }
                        Ok(())
                    },
                    |_, _| admitted = true,
                );
            }
            assert!(admitted);
            assert!(out.pending());
            out.rebind();
            assert!(!out.pending());
            let receipt = text_batch::Applied {
                surface: sid as u16,
                sequence: 10,
            }
            .encode()
            .unwrap();
            assert_eq!(out.receive(1, receipt), None);
            let mut new = view::Text::new();
            new.len = 3;
            new.bytes[..3].copy_from_slice(b"new");
            out.queue(new, 11);
            // A queued/replayed old paint cannot return after the new binding notice.
            out.queue(old, 10);
            let mut commits = 0;
            let mut full = false;
            for _ in 0..6 {
                out.poll(
                    sid,
                    |op, sid, a, b| {
                        if !full {
                            full = true;
                            return Err(ERR_SERVICE_NOT_READY);
                        }
                        if let Some((n, _, bytes)) = stage.receive(
                            target,
                            sid,
                            text_batch::Request::decode(op, a, b).unwrap(),
                        ) {
                            assert_eq!(&bytes[..n], b"new");
                            commits += 1;
                        }
                        Ok(())
                    },
                    |paint, _| assert_eq!(paint, 11),
                );
            }
            assert_eq!(out.receive(1, receipt), None);
            stage.poll_reply(|pd, v| {
                assert_eq!(pd, target);
                assert_eq!(out.receive(1, v), Some((11, 3)));
                0
            });
            assert_eq!(commits, 1);
            assert!(!out.pending());
            out.close();
            out.rebind();
            out.queue(new, 12);
            out.poll(sid, |_, _, _, _| panic!(), |_, _| panic!());
        }
    }
}

#[test]
fn browser_geometry_binds_actual_task_and_retains_until_ready() {
    use document_view::{Delivery, Notice};
    let mut delivery = Delivery::new();
    delivery.queue(15, 150, 320, 240);
    delivery.poll(true, false, |_| panic!("not ready"));
    assert!(delivery.pending());
    let mut first = None;
    delivery.poll(true, true, |n| {
        first = Some(n);
        Err(ERR_SERVICE_NOT_READY)
    });
    assert!(delivery.pending());
    let first = first.unwrap();
    delivery.poll(true, true, |n| {
        assert_eq!(n, first);
        Ok(())
    });
    assert!(!delivery.pending());
    assert!(Notice::decode(3, 15, 150, first.args(), 0).is_some());
    for (caller, target, task, last) in [
        (3, 15, 15, 0),
        (15, 15, 150, 0),
        (3, 16, 150, 0),
        (3, 15, 0, 0),
        (3, 15, 150, first.sequence),
    ] {
        assert!(Notice::decode(caller, target, task, first.args(), last).is_none());
    }
    assert_eq!(Layout::new(15, 320, 240).top, 36);
    delivery.queue(15, 150, 220, 150);
    delivery.poll(false, true, |_| panic!("terminal subject"));
    delivery.queue(15, 150, 640, 480);
    assert!(!delivery.pending());
}

#[path = "../crates/sex-pdx/src/text_selection.rs"]
mod text_selection;
#[test]
fn selected_source_maps_only_visible_body_cells_not_header_or_caret() {
    for (w, h) in [(1280, 700), (624, 291), (320, 240), (144, 90)] {
        let l = Layout::new(9, w, h);
        let body = b"one two\nthree four\nfive six";
        for range in [(0, body.len()), (4, body.len()), (0, 3), (7, 8), (3, 3)] {
            for caret in [range.0, range.1] {
                let mut top = 0;
                let plain = view::render(
                    body, caret, &mut top, l, b"HEADER", false, b"READY", None, None,
                );
                let mut top = 0;
                let selected = view::render_selection(
                    body,
                    caret,
                    &mut top,
                    l,
                    b"HEADER",
                    false,
                    b"READY",
                    None,
                    None,
                    Some(range),
                );
                assert_eq!(selected.bytes, plain.bytes);
                assert_eq!(selected.len, plain.len);
                if let Some((start, end)) = selected.selection {
                    assert!(start < end && end <= selected.len);
                    let bytes = &selected.bytes[start..end];
                    assert!(!bytes.contains(&b'#'));
                    assert!(!bytes.windows(6).any(|w| w == b"HEADER"));
                    assert!(
                        start
                            >= selected.bytes[..selected.len]
                                .iter()
                                .position(|&b| b == b'\n')
                                .unwrap()
                    );
                }
                if range.0 == range.1 || range == (7, 8) {
                    assert_eq!(selected.selection, None)
                }
            }
        }
    }
}
#[test]
fn selected_range_scrolls_and_reflows_from_unchanged_source_without_mutating_bytes() {
    let body = b"a selected long document with spaces\n".repeat(300);
    let original = body.clone();
    for (w, h) in [(1280, 700), (624, 291), (320, 240), (144, 90)] {
        let l = Layout::new(9, w, h);
        let mut top = 0;
        let text = view::render_selection(
            &body,
            body.len(),
            &mut top,
            l,
            b"note",
            false,
            b"READY",
            None,
            None,
            Some((0, body.len())),
        );
        if l.body_rows() > 0 {
            let (start, end) = text.selection.unwrap();
            assert!(end <= text.len);
            assert!(text.bytes[start..end].iter().any(|&b| b == b' '));
            assert!(!text.bytes[start..end].contains(&b'#'));
        }
        assert_eq!(body, original);
    }
}
#[test]
fn replacement_is_whole_capacity_checked_and_zeroes_removed_tail() {
    let mut body = [0; 12288];
    body[..7].copy_from_slice(b"abc def");
    let mut len = 7;
    assert_eq!(
        view::replace(&mut body, &mut len, (4, 7), b"world"),
        Some(9)
    );
    assert_eq!(&body[..len], b"abc world");
    assert_eq!(
        view::replace(&mut body, &mut len, (0, 9), b"short"),
        Some(5)
    );
    assert_eq!(&body[..len], b"short");
    assert!(body[len..9].iter().all(|&b| b == 0));
    let full = [b'Z'; 12288];
    assert_eq!(
        view::replace(&mut body, &mut len, (0, 5), &full),
        Some(12288)
    );
    assert_eq!(len, 12288);
    assert_eq!(body, full);
    let before = body;
    for range in [(0, 0), (12288, 12288), (1, 0), (0, 12289)] {
        let n = len;
        assert_eq!(view::replace(&mut body, &mut len, range, b"x"), None);
        assert_eq!(body, before);
        assert_eq!(len, n);
    }
    assert_eq!(view::replace(&mut body, &mut len, (0, 12288), b""), Some(0));
    assert_eq!(len, 0);
    assert!(body.iter().all(|&b| b == 0));
}
#[test]
fn replacement_rejects_unsupported_text_and_stale_selection_capacity_without_partial_edit() {
    let source = text_selection::Source {
        generation: 7,
        revision: 0,
    };
    let mut selection = text_selection::Selection::new();
    selection.bind(source, 12288).unwrap();
    selection.move_to(source, 1, false).unwrap();
    selection.move_to(source, 5, true).unwrap();
    let snap = selection.snapshot().unwrap();
    assert_eq!(selection.replacement_len(snap, 4), Ok(12288));
    assert!(selection.replacement_len(snap, 5).is_err());
    selection
        .rebind(
            text_selection::Source {
                generation: 8,
                revision: 0,
            },
            12288,
        )
        .unwrap();
    assert!(!selection.unchanged(snap));
    assert!(selection.replacement_len(snap, 0).is_err());
    let mut body = [b'a'; 12288];
    let original = body;
    let mut len = 12288;
    for incoming in [&b"\r"[..], &b"\0"[..], &b"\xff"[..], &b"\xe2\x82"[..], &b"\xc2\x80"[..]] {
        assert_eq!(view::replace(&mut body, &mut len, (0, 3), incoming), None);
        assert_eq!(body, original);
        assert_eq!(len, 12288);
    }
}
#[test]
fn selected_outbox_sends_exact_style_after_chunks_and_full_before_commit() {
    let mut text = view::Text::new();
    text.bytes[..5].copy_from_slice(b"a b c");
    text.len = 5;
    text.selection = Some((1, 4));
    let mut out = delivery::Outbox::new();
    out.queue(text, 7);
    let mut stage = text_transaction::Stage::EMPTY;
    let mut sent = vec![];
    let mut full = false;
    let mut visible = false;
    for _ in 0..10 {
        out.poll(
            156,
            |op, sid, a, b| {
                let r = text_batch::Request::decode(op, a, b).unwrap();
                if op == text_batch::SELECT && !full {
                    full = true;
                    return Err(ERR_SERVICE_NOT_READY);
                }
                sent.push(r);
                if let Some((len, _, bytes)) = stage.receive(9, sid, r) {
                    assert_eq!(&bytes[..len], b"a b c");
                    visible = true;
                } else {
                    assert!(!visible)
                }
                Ok(())
            },
            |paint, len| assert_eq!((paint, len), (7, 5)),
        );
        if visible {
            break;
        }
    }
    assert!(full && visible);
    assert!(matches!(sent[0], text_batch::Request::Begin { .. }));
    assert!(matches!(sent[1], text_batch::Request::Chunk { .. }));
    assert_eq!(
        sent[2],
        text_batch::Request::Select {
            sequence: 7,
            start: 1,
            end: 4
        }
    );
    assert_eq!(
        sent[3],
        text_batch::Request::Commit {
            sequence: 7,
            hash: text_batch::selected_hash(b"a b c", 1, 4).unwrap()
        }
    );
    stage.poll_reply(|_, value| {
        assert_eq!(out.receive(1, value), Some((7, 5)));
        0
    });
    assert!(!out.pending());
    out.poll(
        156,
        |_, _, _, _| panic!("accepted Commit replay"),
        |_, _| panic!(),
    );
}
#[test]
fn page_navigation_clamps_to_real_lines_and_retains_goal_column() {
    let buf = b"abcde\nx\nabcde\nlast";
    let mut goal = usize::MAX;
    let next = view::vertical_steps(buf, 4, 51, &mut goal, false, 2).unwrap();
    assert_eq!(next, 12);
    assert_eq!(goal, 4);
    assert_eq!(
        view::vertical_steps(buf, next, 51, &mut goal, false, 22),
        Some(buf.len())
    );
    assert_eq!(
        view::vertical_steps(buf, buf.len(), 51, &mut goal, true, 22),
        Some(4)
    );
    assert_eq!(view::vertical_steps(buf, 0, 51, &mut goal, true, 22), None);
    assert_eq!(
        view::vertical_steps(buf, buf.len(), 51, &mut goal, false, 22),
        None
    );
}

/// A rejected clipboard operation changes chrome only. Status/naming bytes
/// must not reduce the body row budget or scroll an unchanged caret/document.
#[test]
fn status_and_name_changes_cannot_scroll_or_reflow_the_editor_body() {
    let body = b"000 SEXOS STREAM LINE / SOURCE BYTES ARE DATA...\n".repeat(256);
    let body = &body[..12288.min(body.len())];
    for (w, h) in [(624, 706), (496, 674), (640, 335), (320, 240), (144, 90)] {
        let layout = Layout::new(9, w, h);
        let selection = Some((0, body.len()));
        let mut top = 0;
        let original = view::render_selection(
            body,
            body.len(),
            &mut top,
            layout,
            b"UNTITLED",
            true,
            b"PASTED",
            None,
            None,
            selection,
        );
        let initial_top = top;
        let first_body = |text: &view::Text| {
            let mut offset = 0;
            for _ in 0..layout.chrome_rows() {
                offset += text.bytes[offset..text.len]
                    .iter()
                    .position(|&b| b == b'\n')
                    .unwrap()
                    + 1;
            }
            text.bytes[offset..text.len].to_vec()
        };
        let expected = first_body(&original);
        let body_selection = original.selection.map(|(start, end)| {
            let offset = original.len - expected.len();
            (start - offset, end - offset)
        });
        for (state, name, dirty, naming) in [
            (
                b"CLIPBOARD...".as_slice(),
                b"UNTITLED".as_slice(),
                true,
                None,
            ),
            (
                b"PASTE TOO LARGE".as_slice(),
                b"UNTITLED".as_slice(),
                true,
                None,
            ),
            (
                b"EMPTY SELECTION".as_slice(),
                b"UNTITLED".as_slice(),
                true,
                None,
            ),
            (b"UNKNOWN".as_slice(), b"renamed".as_slice(), false, None),
            (
                b"CLIPBOARD UNCERTAIN".as_slice(),
                b"sixteen-byte-nam".as_slice(),
                true,
                None,
            ),
            (
                b"READY".as_slice(),
                b"renamed".as_slice(),
                false,
                Some(b"save-as-name".as_slice()),
            ),
            (b"PASTED".as_slice(), b"UNTITLED".as_slice(), true, None),
        ] {
            let current = view::render_selection(
                body,
                body.len(),
                &mut top,
                layout,
                name,
                dirty,
                state,
                None,
                naming,
                selection,
            );
            assert_eq!(
                top, initial_top,
                "status/name moved viewport at {w}x{h}: {:?}",
                state
            );
            assert_eq!(
                first_body(&current),
                expected,
                "body changed at {w}x{h}: {:?}",
                state
            );
            let offset = current.len - expected.len();
            assert_eq!(
                current
                    .selection
                    .map(|(start, end)| (start - offset, end - offset)),
                body_selection
            );
            assert!(current.len <= 512);
            if layout.body_rows() > 0 {
                assert!(expected.contains(&b'#'));
            }
        }
    }
}


#[test]
fn utf8_views_preserve_source_bytes_scalar_columns_and_complete_selected_glyphs() {
    let body="aé€🙂漢\tZ\nö".as_bytes();let original=body.to_vec();
    for target in [9,16,18] {for (w,h) in [(624,706),(496,674),(220,150),(80,80),(22,50)] {
        let l=Layout::new(target,w,h);
        for caret in (0..=body.len()).filter(|&n|text_encoding::is_boundary(body,n)) {
            let t=view::render_selection(body,caret,&mut 0,l,b"note",true,b"READY",None,None,Some((1,body.len())));
            let text=core::str::from_utf8(&t.bytes[..t.len]).unwrap();assert_eq!(body,original);
            assert!(t.len<=512);if l.body_rows()>0 {assert!(text.contains('#'));}
            for row in text.split('\n'){assert!(row.chars().count()<=l.cols,"{w} {h}: {row:?}");}
            if let Some((a,b))=t.selection {assert!(text_encoding::is_boundary(&t.bytes[..t.len],a));assert!(text_encoding::is_boundary(&t.bytes[..t.len],b));}
        }
    }}
    let t=view::render_selection(body,body.len(),&mut 0,Layout::new(9,624,706),b"note",false,b"READY",None,None,Some((1,body.len())));
    let text=core::str::from_utf8(&t.bytes[..t.len]).unwrap();assert!(text.contains(core::str::from_utf8(body).unwrap()));
    let (a,b)=t.selection.unwrap();assert_eq!(&t.bytes[a..b],&body[1..]);
}
#[test]
fn utf8_full_capacity_view_keeps_caret_and_never_splits_frame_or_vertical_targets() {
    let text="é€🙂漢\tQ\n".repeat(700);let body=text.as_bytes();let body=&body[..text_encoding::prefix(body,12288)];
    for (w,h) in [(1280,700),(624,706),(496,319),(220,150),(80,80)] {
        let l=Layout::new(9,w,h);let mut top=0;
        let t=view::render(body,body.len(),&mut top,l,b"utf8",true,b"READY",None,None);
        assert!(core::str::from_utf8(&t.bytes[..t.len]).unwrap().contains('#'));assert!(t.len<=512);
        let mut goal=usize::MAX;let mut caret=body.len();
        for _ in 0..40 {if let Some(to)=view::vertical(body,caret,l.cols,&mut goal,true){assert!(text_encoding::is_boundary(body,to));assert!(to<=caret);caret=to;}}
    }
}
#[test]
fn utf8_replacement_rejects_partial_boundaries_without_touching_bytes_or_tail() {
    let mut buf=[0;12288];let source="aé€🙂z".as_bytes();buf[..source.len()].copy_from_slice(source);let original=buf;let mut len=source.len();
    for range in [(2,3),(1,2),(4,6),(6,7),(9,10)] {assert_eq!(view::replace(&mut buf,&mut len,range,b"x"),None);assert_eq!(buf,original);assert_eq!(len,source.len());}
    assert_eq!(view::replace(&mut buf,&mut len,(1,6),"ö\t".as_bytes()),Some(4));
    assert_eq!(&buf[..len],"aö\t🙂z".as_bytes());assert!(buf[len..source.len()].iter().all(|b|*b==0));
}

#[test]
fn tall_narrow_caret_stays_inside_the_actual_display_line_table() {
    for body in ["x\n".repeat(30),"é\n".repeat(40),"🙂\n".repeat(40)] {
        for w in [22,40,82,100,144,220,496,624] {
            let l=Layout::new(9,w,700);let t=view::render(body.as_bytes(),body.len(),&mut 0,l,b"note",false,b"READY",None,None);
            let text=core::str::from_utf8(&t.bytes[..t.len]).unwrap();
            assert!(text.lines().count()<=24,"{w}: {} rows",text.lines().count());
            if l.rows>0 {assert!(text.lines().position(|r|r.contains('#')).unwrap()<24);}
        }
    }
}

#[test]
fn one_four_byte_character_does_not_collapse_otherwise_ascii_viewport() {
    let mut body=("a".repeat(40)+"\n").repeat(20);body.push('🙂');
    let l=Layout::new(9,624,706);let t=view::render(body.as_bytes(),body.len(),&mut 0,l,b"note",false,b"READY",None,None);
    let text=core::str::from_utf8(&t.bytes[..t.len]).unwrap();
    assert!(text.lines().count()>=l.chrome_rows()+5);assert!(text.ends_with("🙂#"));assert!(t.len<=512);
}

#[test]
fn long_status_never_squeezes_the_document_name_below_eight_cells() {
    let l = Layout { w: 496, h: 315, scale: 2, cols: 39, rows: 17, top: 12, toolbar_rows: 1 };
    let mut top = 0;
    let t = view::render_selection(b"abc", 3, &mut top, l, b"uibeta", true, b"UNSAVED: CTRL S SAVES, F11 DISCARDS", None, None, None);
    let header = t.bytes[..t.len].split(|&b| b == b'\n').next().unwrap();
    assert!(header.starts_with(b"uibeta * UNSAVED"), "{:?}", core::str::from_utf8(header));
    assert!(header.len() <= l.cols);
    // When everything fits nothing changes.
    let t = view::render_selection(b"abc", 3, &mut top, l, b"uibeta", false, b"READY", None, None, None);
    assert!(t.bytes[..t.len].starts_with(b"uibeta READY\n"));
    // A long name still yields to the width bound.
    let t = view::render_selection(b"abc", 3, &mut top, l, b"a-very-long-document-name-indeed", true, b"UNSAVED: CTRL S SAVES, F11 DISCARDS", None, None, None);
    let header = t.bytes[..t.len].split(|&b| b == b'\n').next().unwrap();
    assert!(header.starts_with(b"a-very-l * UNSAVED") && header.len() <= l.cols, "{:?}", core::str::from_utf8(header));
}


#[test]
fn all_document_incarnations_receive_exact_fresh_viewports_and_retain_full_notice() {
    use document_view::{Delivery, Notice};
    assert_eq!(app_catalog::DOCUMENT_TARGETS.len(),9);
    assert!(app_catalog::DOCUMENT_TARGETS.contains(&23));
    for &target in &app_catalog::DOCUMENT_TARGETS {
        let task=target+1000;let mut delivery=Delivery::new();
        delivery.queue(target,task,496,319);assert!(delivery.pending());
        delivery.poll(true,false,|_|panic!("viewport consumer is not ready"));
        let mut first=None;
        for _ in 0..3 {delivery.poll(true,true,|n|{
            assert_eq!((n.target,n.task,n.w,n.h,n.sequence),(target,task,496,319,1));
            if let Some(previous)=first{assert_eq!(n,previous);}else{first=Some(n);}
            Err(ERR_SERVICE_NOT_READY)
        });assert!(delivery.pending());}
        let first=first.unwrap();
        assert_eq!(Notice::decode(3,target,task,first.args(),0),Some(first));
        for (caller,expected_target,expected_task,last) in [
            (target,target,task,0),(11,target,task,0),
            (3,if target==9{23}else{9},task,0),
            (3,target,target,0),(3,target,0,0),
            (3,target,task,first.sequence),(3,target,task,u64::MAX),
        ] {assert_eq!(Notice::decode(caller,expected_target,expected_task,first.args(),last),None);}
        let mut changed=first;changed.task+=1;
        assert_eq!(Notice::decode(3,target,task,changed.args(),0),None);
        changed=first;changed.sequence=0;assert_eq!(Notice::decode(3,target,task,changed.args(),0),None);
        delivery.poll(true,true,|n|{assert_eq!(n,first);Ok(())});assert!(!delivery.pending());
        delivery.queue(target,task,496,319);assert!(!delivery.pending());
        delivery.queue(target,task,640,335);
        delivery.poll(true,true,|n|{
            assert_eq!((n.target,n.task,n.w,n.h,n.sequence),(target,task,640,335,2));
            assert_eq!(Notice::decode(3,target,task,n.args(),first.sequence),Some(n));Ok(())
        });assert!(!delivery.pending());
    }
}

#[test]
fn invalid_geometry_membership_and_terminal_delivery_never_replace_valid_pending_notice() {
    use document_view::{Delivery,Notice};
    let mut delivery=Delivery::new();delivery.queue(23,2300,512,384);
    for (target,task,w,h) in [(0,2300,640,480),(12,1200,640,480),(17,1700,640,480),
        (24,2400,640,480),(u32::MAX,2300,640,480),(23,0,640,480),
        (23,2300,0,384),(23,2300,512,0),(23,2300,4097,384),(23,2300,512,u32::MAX)] {
        delivery.queue(target,task,w,h);
        delivery.poll(true,true,|n|{
            assert_eq!((n.target,n.task,n.w,n.h,n.sequence),(23,2300,512,384,1));Err(ERR_SERVICE_NOT_READY)
        });assert!(delivery.pending());
        let malformed=Notice{target,task,w,h,sequence:1};
        assert_eq!(Notice::decode(3,target,task,malformed.args(),0),None);
    }
    delivery.poll(false,true,|_|panic!("retired incarnation must not send"));
    assert!(!delivery.pending());delivery.queue(23,2300,640,480);
    delivery.poll(true,true,|_|panic!("terminal delivery cannot reopen"));assert!(!delivery.pending());
}
