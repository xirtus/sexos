// A program file is untrusted bytes: everything the loader will touch is bounded
// before the first write, and no input panics the checker.
#[path = "../kernel/src/program_image.rs"]
mod program_image;
#[path = "../crates/sex-pdx/src/program_store.rs"]
mod program_store;
use program_image::{parse, relocation, Reject};

const WINDOW: u64 = 1024 * 1024;
fn put(b: &mut [u8], at: usize, v: &[u8]) { b[at..at + v.len()].copy_from_slice(v); }
/// text at 0x1000 (R+X), data at 0x2000 (R+W) with a dynamic table and one relocation.
fn elf() -> Vec<u8> {
    let mut b = vec![0u8; 0x3000];
    put(&mut b, 0, &[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    put(&mut b, 16, &3u16.to_le_bytes());
    put(&mut b, 18, &62u16.to_le_bytes());
    put(&mut b, 24, &0x1000u64.to_le_bytes());
    put(&mut b, 32, &64u64.to_le_bytes());
    put(&mut b, 54, &56u16.to_le_bytes());
    put(&mut b, 56, &3u16.to_le_bytes());
    let header = |b: &mut Vec<u8>, i: usize, kind: u32, flags: u32, at: u64, file: u64, memory: u64| {
        let o = 64 + i * 56;
        put(b, o, &kind.to_le_bytes());
        put(b, o + 4, &flags.to_le_bytes());
        put(b, o + 8, &at.to_le_bytes());
        put(b, o + 16, &at.to_le_bytes());
        put(b, o + 32, &file.to_le_bytes());
        put(b, o + 40, &memory.to_le_bytes());
    };
    header(&mut b, 0, 1, 5, 0x1000, 0x100, 0x100);
    header(&mut b, 1, 1, 6, 0x2000, 0x200, 0x4000);
    header(&mut b, 2, 2, 6, 0x2000, 0x40, 0x40);
    for (i, (tag, value)) in [(7u64, 0x2100u64), (8, 24), (9, 24), (0, 0)].iter().enumerate() {
        put(&mut b, 0x2000 + i * 16, &tag.to_le_bytes());
        put(&mut b, 0x2008 + i * 16, &value.to_le_bytes());
    }
    put(&mut b, 0x2100, &0x2180u64.to_le_bytes());
    put(&mut b, 0x2108, &8u64.to_le_bytes());
    put(&mut b, 0x2110, &0x1040u64.to_le_bytes());
    b
}
#[test]
fn a_good_program_is_laid_out_within_its_window() {
    let image = parse(&elf(), WINDOW).expect("valid");
    assert_eq!(image.count, 2);
    assert_eq!(image.entry, 0);
    assert!(image.segments[0].executable && !image.segments[0].writable);
    assert!(image.segments[1].writable && !image.segments[1].executable);
    assert_eq!(image.segments[1].window, 0x1000);
    assert_eq!(image.rela_count, 1);
    let (at, value) = relocation(&elf(), &image, 0, 0x4000_0000).expect("relocation");
    assert_eq!((at, value), (0x1180, 0x4000_0040));
}
#[test]
fn the_requested_stack_size_is_read() {
    let mut b = elf();
    assert_eq!(parse(&b, WINDOW).expect("valid").stack, 0);
    put(&mut b, 56, &4u16.to_le_bytes());
    put(&mut b, 64 + 3 * 56, &0x6474_e551u32.to_le_bytes());
    put(&mut b, 64 + 3 * 56 + 40, &(2u64 << 20).to_le_bytes());
    assert_eq!(parse(&b, WINDOW).expect("valid").stack, 2 << 20);
}
#[test]
fn writable_code_is_refused() {
    let mut b = elf();
    put(&mut b, 64 + 4, &7u32.to_le_bytes());
    assert_eq!(parse(&b, WINDOW), Err(Reject::Permissions));
}
#[test]
fn a_program_larger_than_its_window_is_refused() {
    assert!(parse(&elf(), 0x2000).is_err());
    let mut b = elf();
    put(&mut b, 64 + 56 + 40, &(u64::MAX - 8).to_le_bytes());
    assert!(parse(&b, WINDOW).is_err());
}
#[test]
fn a_foreign_relocation_is_refused() {
    let mut b = elf();
    put(&mut b, 0x2108, &1u64.to_le_bytes());
    assert_eq!(parse(&b, WINDOW), Err(Reject::Relocation));
}
#[test]
fn entry_outside_code_is_refused() {
    let mut b = elf();
    put(&mut b, 24, &0x2000u64.to_le_bytes());
    assert_eq!(parse(&b, WINDOW), Err(Reject::Entry));
}
#[test]
fn every_truncation_and_corruption_stays_bounded() {
    let good = elf();
    for len in 0..good.len() { let _ = parse(&good[..len], WINDOW); }
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut next = || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
    let mut accepted = 0;
    for _ in 0..200_000 {
        let mut b = good.clone();
        for _ in 0..1 + next() % 6 {
            let at = (next() as usize) % if next() % 2 == 0 { 0x180 } else { b.len() };
            b[at] = next() as u8;
        }
        let Ok(image) = parse(&b, WINDOW) else { continue };
        accepted += 1;
        for s in &image.segments[..image.count] {
            assert!(s.memory > 0 && s.window + s.memory <= WINDOW);
            assert!(s.offset + s.file <= b.len() as u64 && s.file <= s.memory);
            assert!(!(s.writable && s.executable));
        }
        for (i, s) in image.segments[..image.count].iter().enumerate() {
            for t in &image.segments[i + 1..image.count] { 
                assert!(s.window + s.memory <= t.window && (s.window + s.memory - 1) / 4096 < t.window / 4096, "segments share a page");
            }
        }
        assert!(image.segments[..image.count].iter().any(|s| s.executable && image.entry >= s.window && image.entry < s.window + s.memory));
        for index in 0..image.rela_count {
            let (at, _) = relocation(&b, &image, index, 0x4000_0000).expect("every counted relocation resolves");
            assert!(image.segments[..image.count].iter().any(|s| at >= s.window && at + 8 <= s.window + s.memory));
        }
    }
    assert!(accepted > 0, "the corruption never left a loadable file, so this checked nothing");
}
#[test]
fn the_directory_finds_names_and_nothing_else() {
    let mut d = [0u8; 512];
    assert_eq!(program_store::find(&d, [0, 0]), None);
    d[..8].copy_from_slice(&program_store::MAGIC);
    let name = program_store::pack_name(b"fortune").unwrap();
    d[16..23].copy_from_slice(b"fortune");
    d[32..36].copy_from_slice(&2049u32.to_le_bytes());
    d[36..40].copy_from_slice(&77u32.to_le_bytes());
    d[40..48].copy_from_slice(&5u64.to_le_bytes());
    assert_eq!(program_store::find(&d, name), Some(program_store::Entry { lba: 2049, len: 77, hash: 5 }));
    assert_eq!(program_store::find(&d, program_store::pack_name(b"fortun").unwrap()), None);
    assert_eq!(program_store::pack_name(b""), None);
    assert_eq!(program_store::pack_name(b"seventeen-letters"), None);
    assert_eq!(program_store::decode_reply(program_store::loaded(9)), Some(Ok(9)));
    assert_eq!(program_store::decode_reply(program_store::failed(4)), Some(Err(4)));
    assert_eq!(program_store::decode_reply(9), None);
    assert_eq!(program_store::fnv(b"a"), 0xaf63dc4c8601ec8c);
}
