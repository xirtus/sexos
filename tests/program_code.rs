// Code a program seals must not hold an instruction that rewrites its protection keys.
#[path = "../crates/sex-pdx/src/program_code.rs"]
mod program_code;
use program_code::*;
    #[test]
    fn finds_refused_instructions_at_any_offset() {
        assert_eq!(check(&[0x90, 0x0F, 0x01, 0xEF]), Err(1));
        // xrstor [rax] = 0F AE 28; xrstors [rax] = 0F C7 18.
        assert_eq!(check(&[0x0F, 0xAE, 0x28]), Err(0));
        assert_eq!(check(&[0x48, 0x0F, 0xC7, 0x18]), Err(1));
        // Hidden inside an immediate: mov eax, 0xEF010F90.
        assert_eq!(check(&[0xB8, 0x90, 0x0F, 0x01, 0xEF]), Err(2));
    }
    #[test]
    fn allows_ordinary_code() {
        // lfence (0F AE E8), xsave [rax] (0F AE 20), rdpkru (0F 01 EE), cmpxchg16b (0F C7 08), ret.
        assert_eq!(check(&[0x0F, 0xAE, 0xE8, 0x0F, 0xAE, 0x20, 0x0F, 0x01, 0xEE, 0x48, 0x0F, 0xC7, 0x08, 0xC3]), Ok(()));
    }
    #[test]
    fn spans_pieces_and_primed_neighbours() {
        let mut s = Scan::new();
        assert_eq!(s.feed(&[0x90, 0x0F], false), None);
        assert_eq!(s.feed(&[0x01, 0xEF], false), Some(1));
        let mut s = Scan::primed([0x0F, 0x01]);
        assert_eq!(s.feed(&[0xEF], true), Some(-2));
        let mut s = Scan::primed([0x90, 0x90]);
        assert_eq!(s.feed(&[0x0F, 0xAE, 0x28], true), Some(0));
    }
    /// The word skip finds exactly what a byte-at-a-time scan finds, at every split.
    #[test]
    fn word_skip_matches_the_byte_scan() {
        fn slow(code: &[u8]) -> Option<usize> { (0..code.len().saturating_sub(2)).find(|&i| code[i] == 0x0F && refused(code[i + 1], code[i + 2])) }
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
        for round in 0..20_000 {
            let len = (next() % 80) as usize;
            let sparse = round % 2 == 0;
            let code: Vec<u8> = (0..len).map(|_| { let r = next(); match r % if sparse { 64 } else { 6 } { 0 => 0x0F, 1 => [0x01, 0xAE, 0xC7][(r >> 8) as usize % 3], 2 => [0xEF, 0x28, 0x18, 0xE8][(r >> 16) as usize % 4], _ => (r >> 24) as u8 } }).collect();
            assert_eq!(check(&code).err(), slow(&code), "{code:02x?}");
            let cut = if len == 0 { 0 } else { (next() as usize) % len };
            let mut s = Scan::new();
            let got = s.feed(&code[..cut], false).or_else(|| s.feed(&code[cut..], false));
            assert_eq!(got.map(|a| a as usize), slow(&code), "cut {cut} {code:02x?}");
        }
    }
