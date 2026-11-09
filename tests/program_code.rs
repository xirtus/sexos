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
