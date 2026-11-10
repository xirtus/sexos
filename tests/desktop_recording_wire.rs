// SPDX-License-Identifier: GPL-3.0-only
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
mod wire;
use wire::*;

#[test]
fn sequences_bind_the_complete_shell_epoch_and_refuse_wrap() {
    for epoch in 1..16 {
        for count in [1, (1 << 20) - 1] {
            let seq = RoleSequence::new(epoch, count).unwrap();
            assert_eq!(RoleSequence::decode(seq.word() as u64), Some(seq));
            assert!(seq.matches(epoch));
            assert!(!seq.matches(epoch + 1));
        }
    }
    for (epoch, count) in [(0, 1), (16, 1), (1, 0), (1, 1 << 20)] {
        assert!(RoleSequence::new(epoch, count).is_none());
    }
    assert!(RoleSequence::decode(1 << 24).is_none());
    for epoch in [1, 9, 255] {
        let seq = SessionSequence::new(epoch, (1 << 24) - 1).unwrap();
        assert_eq!(SessionSequence::decode(seq.word() as u64), Some(seq));
        assert!(seq.matches(epoch));
        assert!(!seq.matches(epoch + 1));
    }
    assert!(SessionSequence::decode(1 << 32).is_none());
    assert!(SessionSequence::new(256, 1).is_none());
    assert!(SessionSequence::new(1, 1 << 24).is_none());
}

#[test]
fn role_request_and_retained_reply_require_provider_origin_and_exact_transaction() {
    let request = BindRequest::new(RoleSequence::new(2, 17).unwrap(), 0x1234_5678, 2, 9).unwrap();
    assert_eq!(BindRequest::decode(SHELL, request.args()), Some(request));
    assert!(BindRequest::decode(SPINDLE, request.args()).is_none());
    assert!(BindRequest::decode(TARGET, request.args()).is_none());
    let mut stale = request.args();
    stale[2] = 3;
    assert!(BindRequest::decode(SHELL, stale).is_none());
    for status in [
        BindStatus::Ok,
        BindStatus::Denied,
        BindStatus::Unavailable,
        BindStatus::Io,
        BindStatus::Busy,
    ] {
        let reply = BindReply::new(request, status);
        assert_eq!(BindReply::decode(FILES, reply.word()), Some(reply));
        assert!(BindReply::decode(DISPLAY, reply.word()).is_none());
        assert!(BindReply::decode(FILES, reply.word() ^ (1 << 60)).is_none());
        assert!(BindReply::decode(FILES, reply.word() | (15 << 56)).is_none());
    }
    assert!(BindRequest::new(request.sequence, 0, 2, 9).is_none());
    assert!(BindRequest::new(request.sequence, request.task, 2, 0).is_none());
}

fn words(bytes: &[u8]) -> [u64; 2] {
    let mut chunk = [0; 16];
    chunk[..bytes.len()].copy_from_slice(bytes);
    [
        u64::from_le_bytes(chunk[..8].try_into().unwrap()),
        u64::from_le_bytes(chunk[8..].try_into().unwrap()),
    ]
}
#[test]
fn filename_chunks_reject_forged_owner_reordering_and_nonzero_padding() {
    let mut name = Name::new();
    let file = b"an-explicit-recording.mkv";
    assert!(name.begin(97, 2, file.len()));
    assert!(!name.chunk(98, 2, 0, words(&file[..16])));
    assert!(!name.chunk(97, 3, 0, words(&file[..16])));
    assert!(!name.chunk(97, 2, 16, words(&file[16..])));
    assert!(name.chunk(97, 2, 0, words(&file[..16])));
    assert!(name.complete(97, 2).is_none());
    let mut bad = words(&file[16..]);
    bad[1] |= 0xff << 56;
    assert!(!name.chunk(97, 2, 16, bad));
    assert!(name.chunk(97, 2, 16, words(&file[16..])));
    assert_eq!(name.complete(97, 2), Some(file.as_slice()));
    assert!(name.complete(98, 2).is_none());
    name.clear();
    assert!(name.complete(97, 2).is_none());
}
#[test]
fn filenames_cannot_escape_the_explicit_recordings_root() {
    for invalid in [
        b"../a.mkv".as_slice(),
        b"nested/a.mkv",
        b"a\0.mkv",
        b"a\n.mkv",
        b"movie.mp4",
        b"@/picked.mkv",
        b"@private.mkv",
    ] {
        let mut name = Name::new();
        assert!(name.begin(3, 1, invalid.len()));
        assert!(name.chunk(3, 1, 0, words(invalid)));
        assert!(name.complete(3, 1).is_none());
    }
    let mut name = Name::new();
    assert!(!name.begin(0, 1, 1));
    assert!(!name.begin(3, 0, 1));
    assert!(!name.begin(3, 1, 0));
    assert!(!name.begin(3, 1, NAME_BYTES + 1));
}

#[test]
fn image_read_release_mode_and_epoch_transactions_are_distinct_and_bounded() {
    let seq = RoleSequence::new(3, 19).unwrap();
    let read = ImageRequest::new(ImageKind::Read, seq, 3, 7).unwrap();
    assert_eq!(ImageRequest::decode(SHELL, read.args()), Some(read));
    assert!(ImageRequest::decode(TARGET, read.args()).is_none());
    let mut args = read.args();
    args[0] |= 1 << 24;
    assert!(ImageRequest::decode(SHELL, args).is_none());
    let mut args = read.args();
    args[0] = (3 << 56) | seq.word() as u64;
    assert!(ImageRequest::decode(SHELL, args).is_none());
    let mut args = read.args();
    args[1] |= 1 << 32;
    assert!(ImageRequest::decode(SHELL, args).is_none());
    let mut args = read.args();
    args[2] = 4;
    assert!(ImageRequest::decode(SHELL, args).is_none());
    let ready = ImageReply::new(read, BindStatus::Ok, 1024).unwrap();
    assert_eq!(ImageReply::decode(FILES, ready.word()), Some(ready));
    assert!(ImageReply::decode(SHELL, ready.word()).is_none());
    assert!(ImageReply::new(read, BindStatus::Ok, 0).is_none());
    assert!(ImageReply::new(read, BindStatus::Ok, MAX_IMAGE_BYTES + 1).is_none());
    assert!(ImageReply::new(read, BindStatus::Denied, 1024).is_none());
    let release = ImageRequest::new(ImageKind::Release, seq, 3, 7).unwrap();
    let ack = ImageReply::new(release, BindStatus::Ok, 0).unwrap();
    assert_eq!(ImageReply::decode(FILES, ack.word()), Some(ack));
    assert_ne!(ack.word(), ready.word());
    assert!(ImageReply::new(release, BindStatus::Ok, 1).is_none());
    assert!(BindReply::decode(FILES, ready.word()).is_none());
    assert!(ImageReply::decode(
        FILES,
        BindReply::new(request_for_image_test(), BindStatus::Ok).word()
    )
    .is_none());
}
fn request_for_image_test() -> BindRequest {
    BindRequest::new(RoleSequence::new(3, 19).unwrap(), 107, 3, 7).unwrap()
}

#[test]
fn person_intents_preserve_epoch_and_reject_reserved_fields() {
    let intents=[Intent::Begin{sequence:17,length:200,shell_epoch:9},Intent::Chunk{sequence:17,offset:192,words:[123,0]},Intent::Commit{sequence:17},Intent::Stop{sequence:18}];
    for intent in intents {
        let args=intent.args();assert_eq!(Intent::decode(SPINDLE,args),Some(intent));
        assert!(Intent::decode(TARGET,args).is_none());
        for bit in [24,31,40,55] {let mut bad=args;bad[0]|=1<<bit;assert!(Intent::decode(SPINDLE,bad).is_none());}
        let mut zero=args;zero[0]&=!0xff_ffff;assert!(Intent::decode(SPINDLE,zero).is_none());
    }
    for intent in [Intent::Begin{sequence:1,length:0,shell_epoch:1},Intent::Begin{sequence:1,length:201,shell_epoch:1},Intent::Begin{sequence:1,length:1,shell_epoch:0},Intent::Chunk{sequence:1,offset:1,words:[0,0]},Intent::Chunk{sequence:1,offset:208,words:[0,0]}] {assert!(Intent::decode(SPINDLE,intent.args()).is_none());}
    let mut bad=Intent::Stop{sequence:1}.args();bad[1]=1;assert!(Intent::decode(SPINDLE,bad).is_none());
}
#[test]
fn person_status_is_typed_and_never_confuses_starting_with_active_or_saved() {
    for status in [IntentStatus::Denied,IntentStatus::Unavailable,IntentStatus::Busy,IntentStatus::Starting,IntentStatus::Stopped,IntentStatus::Saved,IntentStatus::Failed,IntentStatus::Active] {
        let word=intent_reply(0xff_ffff,status).unwrap();assert_eq!(decode_intent_reply(SHELL,word),Some((0xff_ffff,status)));
        assert!(decode_intent_reply(FILES,word).is_none());assert!(decode_intent_reply(SHELL,word|1<<24).is_none());
    }
    assert!(intent_reply(0,IntentStatus::Active).is_none());assert!(intent_reply(1<<24,IntentStatus::Saved).is_none());
    assert!(decode_intent_reply(SHELL,0x8809_0000_0000_0001).is_none());
}
