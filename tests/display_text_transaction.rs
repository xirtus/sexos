#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
#[path = "../crates/sex-pdx/src/text_batch.rs"]
pub mod text_batch;
#[path = "../servers/sexdisplay/src/text_transaction.rs"]
mod transaction;
use text_batch::{hash, Request, CAPACITY};
use transaction::Stage;
fn begin(s: &mut Stage, seq: u32, len: usize) {
    assert!(s
        .receive(
            9,
            156,
            Request::Begin {
                sequence: seq,
                len,
                color: 0xffe0f0ff
            }
        )
        .is_none());
}
fn chunk(s: &mut Stage, seq: u32, offset: usize, input: &[u8]) {
    let mut bytes = [0; 8];
    bytes[..input.len()].copy_from_slice(input);
    assert!(s
        .receive(
            9,
            156,
            Request::Chunk {
                sequence: seq,
                offset,
                count: input.len(),
                bytes
            }
        )
        .is_none());
}
#[test]
fn every_partial_prefix_is_private_and_empty_or_full_commit_is_exact() {
    for len in [0, 1, 7, 8, 9, 511, 512] {
        let text = vec![b'a'; len];
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, len);
        for (i, c) in text.chunks(8).enumerate() {
            chunk(&mut s, 1, i * 8, c);
        }
        let (actual, color, bytes) = s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(&text),
                },
            )
            .unwrap();
        assert_eq!(actual, len);
        assert_eq!(color, 0xffe0f0ff);
        assert_eq!(&bytes[..len], &text);
        assert!(bytes[len..].iter().all(|&v| v == 0));
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(&text)
                }
            )
            .is_none());
    }
}
#[test]
fn incomplete_bad_hash_and_contradictory_duplicate_poison_until_fresh_sequence() {
    for mode in 0..3 {
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, 9);
        chunk(&mut s, 1, 0, b"abcdefgh");
        match mode {
            0 => {}
            1 => chunk(&mut s, 1, 8, b"i"),
            _ => {
                chunk(&mut s, 1, 0, b"Abcdefgh");
                chunk(&mut s, 1, 8, b"i");
            }
        }
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: if mode == 1 { 0 } else { hash(b"abcdefghi") }
                }
            )
            .is_none());
        chunk(&mut s, 1, 8, b"i");
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"abcdefghi")
                }
            )
            .is_none());
        begin(&mut s, 2, 1);
        chunk(&mut s, 2, 0, b"z");
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 2,
                    hash: hash(b"z")
                }
            )
            .is_some());
        begin(&mut s, 1, 0);
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"")
                }
            )
            .is_none());
    }
}
#[test]
fn actual_owner_surface_retirement_and_slot_rebinding_fence_partial_data() {
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 1);
    chunk(&mut s, 1, 0, b"a");
    for (owner, sid) in [(16, 156), (9, 301), (0, 156)] {
        assert!(s
            .receive(
                owner,
                sid,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a")
                }
            )
            .is_none());
    }
    s.retire(16);
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_some());
    s.retire(9);
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_none());
    begin(&mut s, 2, 1);
    chunk(&mut s, 2, 0, b"a");
    s.receive(
        16,
        301,
        Request::Begin {
            sequence: 1,
            len: 1,
            color: 0,
        },
    );
    assert!(s
        .receive(
            16,
            301,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_none());
}
#[test]
fn wire_rejects_reserved_bits_zero_wrapping_sequences_bounds_and_padding() {
    for len in [0, 1, 512] {
        let r = Request::Begin {
            sequence: u32::MAX,
            len,
            color: 0xffe0f0ff,
        };
        let (op, a, b) = r.wire();
        assert_eq!(Request::decode(op, a, b), Some(r));
    }
    for (op, a, b) in [
        (text_batch::BEGIN, 1 << 48, 0),
        (text_batch::BEGIN, 513 << 32 | 1, 0),
        (text_batch::BEGIN, 1, 1 << 32),
        (text_batch::BEGIN, 0, 0),
        (text_batch::COMMIT, 1 << 32, 0),
        (text_batch::CHUNK, 0, 1 << 32 | 1 << 24 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 512 << 8 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 1 << 8 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 9),
        (text_batch::CHUNK, 0x6161, 1 << 32 | 1),
    ] {
        assert!(Request::decode(op, a, b).is_none(), "{op:x} {a:x} {b:x}");
    }
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, CAPACITY);
    assert!(s
        .receive(
            9,
            156,
            Request::Chunk {
                sequence: 1,
                offset: 0,
                count: usize::MAX,
                bytes: [0; 8]
            }
        )
        .is_none());
}
#[test]
fn identical_duplicates_and_reverse_delivery_are_idempotent() {
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 9);
    chunk(&mut s, 1, 8, b"i");
    begin(&mut s, 1, 9);
    chunk(&mut s, 1, 0, b"abcdefgh");
    chunk(&mut s, 1, 0, b"abcdefgh");
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"abcdefghi")
            }
        )
        .is_some());
}
#[test]
fn completion_retained_on_legacy_full_once_per_pass_and_scrubbed_on_exit() {
    for retire in [false, true] {
        let mut s = Stage::EMPTY;
        begin(&mut s, 7, 1);
        chunk(&mut s, 7, 0, b"x");
        s.receive(
            9,
            156,
            Request::Commit {
                sequence: 7,
                hash: hash(b"x"),
            },
        )
        .unwrap();
        let mut original = None;
        for _ in 0..8 {
            let mut attempts = 0;
            s.poll_reply(|owner, value| {
                assert_eq!(owner, 9);
                attempts += 1;
                if let Some(old) = original {
                    assert_eq!(old, value)
                } else {
                    original = Some(value)
                }
                1 // Actual syscall-29 Full status, unlike enqueue Full.
            });
            assert_eq!(attempts, 1);
        }
        if retire {
            s.retire(9);
            s.poll_reply(|_, _| panic!());
        } else {
            s.poll_reply(|owner, value| {
                assert_eq!(owner, 9);
                assert_eq!(Some(value), original);
                0
            });
            s.poll_reply(|_, _| panic!());
        }
    }
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 0);
    s.receive(
        9,
        156,
        Request::Commit {
            sequence: 1,
            hash: hash(b""),
        },
    )
    .unwrap();
    s.poll_reply(|_, _| 7);
    s.poll_reply(|_, _| 1);
    s.reset();
    s.poll_reply(|_, _| panic!("authoritative cleanup must cancel retry"));
}
#[test]
fn surface_destruction_and_vacant_rebinding_scrub_partial_bytes_and_pending_receipt() {
    for complete in [false, true] {
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, 1);
        chunk(&mut s, 1, 0, b"a");
        if complete {
            s.receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a"),
                },
            )
            .unwrap();
        }
        s.reset(); // Actual operation used by authorized destruction/vacant binding.
        s.poll_reply(|_, _| panic!("destroyed surface receipt escaped"));
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a")
                }
            )
            .is_none());
        begin(&mut s, 2, 1);
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 2,
                    hash: hash(b"a")
                }
            )
            .is_none());
        begin(&mut s, 3, 1);
        chunk(&mut s, 3, 0, b"z");
        let (n, _, bytes) = s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 3,
                    hash: hash(b"z"),
                },
            )
            .unwrap();
        assert_eq!(&bytes[..n], b"z");
    }
}

fn selected(s: &mut Stage, seq: u32, start: usize, end: usize) {
    assert!(s.receive(9, 156, Request::Select { sequence: seq, start, end }).is_none());
}
#[test]
fn selection_wire_is_canonical_and_integrity_binds_range_length_and_text() {
    for (start,end) in [(0,0),(0,512),(123,511)] {
        let request=Request::Select{sequence:u32::MAX,start,end};
        let(op,a,b)=request.wire();assert_eq!(Request::decode(op,a,b),Some(request));
    }
    for(a,b)in[(0,0),(1<<32,0),(1,1<<32),(1,513<<16),(1,2|(1<<16)),(1,1|(1<<16))]{
        assert!(Request::decode(text_batch::SELECT,a,b).is_none());
    }
    let h=text_batch::selected_hash(b"abcdef",1,4).unwrap();
    assert_ne!(h,hash(b"abcdef"));
    assert_ne!(h,text_batch::selected_hash(b"abcdef",1,5).unwrap());
    assert_ne!(h,text_batch::selected_hash(b"abcde",1,4).unwrap());
    assert_ne!(h,text_batch::selected_hash(b"abcdeg",1,4).unwrap());
    for(start,end)in[(1,1),(2,1),(0,7)]{assert_eq!(text_batch::selected_hash(b"abcdef",start,end),None);}
}
#[test]
fn selection_is_private_until_same_transaction_commits_and_receipt_never_reapplies() {
    let text=b"one two\nthree";let mut s=Stage::EMPTY;
    begin(&mut s,5,text.len());for(i,b)in text.chunks(8).enumerate(){chunk(&mut s,5,i*8,b);}
    selected(&mut s,5,1,11);assert_eq!(s.selection(),(0,0));
    selected(&mut s,5,1,11); // exact duplicate cannot change meaning
    s.receive(9,156,Request::Commit{sequence:5,hash:text_batch::selected_hash(text,1,11).unwrap()}).unwrap();
    assert_eq!(s.selection(),(1,11));
    selected(&mut s,5,0,4);assert_eq!(s.selection(),(1,11));
    for _ in 0..4{s.poll_reply(|owner,word|{assert_eq!(owner,9);assert_eq!(text_batch::Applied::decode(1,word).unwrap().sequence,5);1});}
    s.poll_reply(|_,_|0);s.poll_reply(|_,_|panic!("duplicate receipt"));
    assert!(s.receive(9,156,Request::Commit{sequence:5,hash:text_batch::selected_hash(text,1,11).unwrap()}).is_none());
    // A fresh legacy replacement clears selection atomically with its text.
    begin(&mut s,6,1);chunk(&mut s,6,0,b"z");
    s.receive(9,156,Request::Commit{sequence:6,hash:hash(b"z")}).unwrap();assert_eq!(s.selection(),(0,0));
}
#[test]
fn premature_invalid_contradictory_or_post_selection_chunks_poison_until_new_sequence() {
    for mode in 0..5 {
        let mut s=Stage::EMPTY;begin(&mut s,1,3);
        if mode==0 {selected(&mut s,1,0,2);}
        chunk(&mut s,1,0,b"abc");
        match mode {
            0=>{},
            1=>selected(&mut s,1,0,4),
            2=>{selected(&mut s,1,0,2);selected(&mut s,1,1,3);},
            3=>{selected(&mut s,1,0,2);chunk(&mut s,1,0,b"abc");},
            _=>selected(&mut s,1,1,1),
        }
        assert!(s.receive(9,156,Request::Commit{sequence:1,hash:text_batch::selected_hash(b"abc",0,2).unwrap()}).is_none());
        assert_eq!(s.selection(),(0,0));s.poll_reply(|_,_|panic!("poisoned receipt"));
        begin(&mut s,2,3);chunk(&mut s,2,0,b"abc");selected(&mut s,2,0,2);
        assert!(s.receive(9,156,Request::Commit{sequence:2,hash:text_batch::selected_hash(b"abc",0,2).unwrap()}).is_some());
    }
}
#[test]
fn foreign_stale_selection_cannot_poison_current_owner_and_selected_legacy_hash_is_rejected() {
    let mut s=Stage::EMPTY;begin(&mut s,8,3);chunk(&mut s,8,0,b"abc");
    for(owner,sid,sequence)in[(16,156,8),(9,301,8),(9,156,7),(0,156,8)]{
        assert!(s.receive(owner,sid,Request::Select{sequence,start:0,end:3}).is_none());
    }
    selected(&mut s,8,0,2);
    assert!(s.receive(9,156,Request::Commit{sequence:8,hash:hash(b"abc")}).is_none());
    assert_eq!(s.selection(),(0,0));
    begin(&mut s,9,0);selected(&mut s,9,0,0);
    assert!(s.receive(9,156,Request::Commit{sequence:9,hash:text_batch::selected_hash(b"",0,0).unwrap()}).is_some());
}
#[test]
fn selected_full_capacity_and_retirement_clear_metadata_bytes_and_pending_receipt() {
    let mut s=Stage::EMPTY;let text=[b'x';CAPACITY];begin(&mut s,1,CAPACITY);
    for(i,b)in text.chunks(8).enumerate(){chunk(&mut s,1,i*8,b);}
    selected(&mut s,1,0,CAPACITY);
    s.receive(9,156,Request::Commit{sequence:1,hash:text_batch::selected_hash(&text,0,CAPACITY).unwrap()}).unwrap();
    assert_eq!(s.selection(),(0,512));s.retire(16);assert_eq!(s.selection(),(0,512));
    s.retire(9);assert_eq!(s.selection(),(0,0));s.poll_reply(|_,_|panic!("retired receipt"));
    assert_eq!(s.diagnostic(),(0,0,0,0,0,false,false));
    // Actual production reset includes volatile erasure of every inline byte and
    // both selection offsets, rather than relying only on later slot reuse.
    let source=include_str!("../servers/sexdisplay/src/text_transaction.rs");
    let reset=&source[source.find("pub fn reset").unwrap()..source.find("/// Only expose selection").unwrap()];
    assert!(reset.contains("for byte in &mut self.bytes"));assert!(reset.contains("write_volatile(&mut self.selection_start, 0)"));assert!(reset.contains("write_volatile(&mut self.selection_end, 0)"));
}

#[test]
fn production_dispatch_keeps_selection_owned_atomic_damaged_and_terminally_erased() {
    let source=include_str!("../servers/sexdisplay/src/main.rs");
    let begin=source.find("                sex_pdx::text_batch::BEGIN |").unwrap();
    let end=source[begin..].find("                // ── Text rendering opcodes").unwrap()+begin;
    let route=&source[begin..end];
    assert!(route.contains("sex_pdx::text_batch::SELECT"));
    assert!(route.find("slot.owner_pd != msg.caller_pd").unwrap()<route.find("TEXT_STAGES[index].receive").unwrap());
    assert!(route.find("if let Some((len,color,bytes))").unwrap()<route.find("slot.selection_start = selection_start").unwrap());
    assert!(route.contains("TEXT_STAGES[index].selection()"));
    assert!(route.contains("changed |= (slot.selection_start, slot.selection_end) != (selection_start, selection_end)"));
    assert!(route.contains("text_damage.add(old_damage"));
    let lifecycle=include_str!("../servers/sexdisplay/src/surface_lifecycle.rs");
    assert!(lifecycle.contains("surface.erase_selection();\n                *surface = SURFACE_EMPTY;"));
    assert!(lifecycle.contains("write_volatile(&mut self.selection_start, 0)"));
    assert!(lifecycle.contains("write_volatile(&mut self.selection_end, 0)"));
    assert!(source.contains("slot.erase_selection();\n                                *slot = SURFACE_EMPTY;"));
}

#[test]
fn utf8_chunks_cross_wire_boundaries_but_only_complete_valid_text_and_scalar_selection_seal(){
    let text="1234567é€😀\t\nlast".as_bytes();
    let mut stage=Stage::EMPTY;begin(&mut stage,1,text.len());
    for(i,part)in text.chunks(8).enumerate(){
        let mut bytes=[0;8];bytes[..part.len()].copy_from_slice(part);
        let request=Request::Chunk{sequence:1,offset:i*8,count:part.len(),bytes};
        let(op,a,b)=request.wire();assert_eq!(Request::decode(op,a,b),Some(request));
        assert!(stage.receive(9,156,request).is_none());
    }
    selected(&mut stage,1,7,16);
    assert_eq!(stage.selection(),(0,0));
    let digest=text_batch::selected_hash(text,7,16).unwrap();
    let(len,_,bytes)=stage.receive(9,156,Request::Commit{sequence:1,hash:digest}).unwrap();
    assert_eq!(len,text.len());assert_eq!(&bytes[..len],text);assert_eq!(stage.selection(),(7,16));
    for(start,end)in[(8,9),(7,8),(10,12),(13,16)]{
        let mut stage=Stage::EMPTY;begin(&mut stage,2,text.len());
        for(i,part)in text.chunks(8).enumerate(){chunk(&mut stage,2,i*8,part);}
        selected(&mut stage,2,start,end);
        assert!(stage.receive(9,156,Request::Commit{sequence:2,hash:text_batch::selected_hash(text,start,end).unwrap()}).is_none());
        assert_eq!(stage.selection(),(0,0));stage.poll_reply(|_,_|panic!("invalid selection ACK"));
    }
}
#[test]
fn whole_text_validation_rejects_controls_and_malformed_utf8_without_rewriting_or_receipt(){
    for text in[&b"\0"[..],&b"\x1b"[..],&b"\x7f"[..],&b"\xc2\x85"[..],&b"\xc0\xaf"[..],&b"\xed\xa0\x80"[..],&b"\xf4\x90\x80\x80"[..],&b"\xe2\x82"[..],&b"\xff"[..]]{
        let mut stage=Stage::EMPTY;begin(&mut stage,1,text.len());
        chunk(&mut stage,1,0,text);
        assert!(stage.receive(9,156,Request::Commit{sequence:1,hash:hash(text)}).is_none());
        stage.poll_reply(|_,_|panic!("invalid text ACK"));
        assert!(stage.receive(9,156,Request::Commit{sequence:1,hash:hash(b"?")}).is_none());
    }
    for complete in[false,true]{
        let mut text=[b'a';512];text[510]=0xc3;text[511]=if complete{0xa9}else{0x20};
        let mut stage=Stage::EMPTY;begin(&mut stage,1,text.len());
        for(i,part)in text.chunks(8).enumerate(){chunk(&mut stage,1,i*8,part);}
        let result=stage.receive(9,156,Request::Commit{sequence:1,hash:hash(&text)});
        assert_eq!(result.is_some(),complete);
        if let Some((len,_,bytes))=result{assert_eq!(len,512);assert_eq!(&bytes[..len],&text);}
    }
}
