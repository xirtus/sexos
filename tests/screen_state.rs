//! Host tests for the lock/dim/off state Display and the Shell compile.
#[path = "../crates/sex-pdx/src/screen_state.rs"]
mod screen_state;

use screen_state::*;
    #[test]
    fn words_and_shades() {
        let l = Lock { locked: true, typed: 12, say: SAY_WRONG };
        assert_eq!(Lock::decode(l.word()), Some(l));
        assert_eq!(Lock::decode(1 << 30), None);
        assert_eq!(Lock::decode(2), None);
        assert_eq!(shade(0x00FF_8040, DIM), 0x007F_4020);
        assert_eq!(shade(0x00FF_FFFF, OFF), 0);
        assert_eq!(shade(0x0012_3456, NORMAL), 0x0012_3456);
        assert_eq!(set(3, 0), None);
        assert_eq!(set(DIM as u64, l.word()), Some(true));
        assert_eq!(set(DIM as u64, l.word()), Some(false));
        assert!(take_mode_change());
        assert!(!take_mode_change());
        assert_eq!(lock(), l);
    }
