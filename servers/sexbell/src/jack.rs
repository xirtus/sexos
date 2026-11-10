// SPDX-License-Identifier: GPL-3.0-only
//! Following the headphone jack: while headphones are plugged in, the speakers among
//! the outputs Bell plays to by default go quiet; when they come out, the speakers play
//! again. Pure, tested on the host (tests/bell_jack.rs).

/// Jack kinds, as in the pin's default configuration.
pub const SPEAKER: u32 = 1;
pub const HEADPHONES: u32 = 2;

/// The outputs to play to: `home` (the default set) without its speakers while any
/// headphones in it are plugged in. `kinds[k]` is output k's kind. Never empty: when
/// home holds only speakers they keep playing.
pub fn wanted(home: u32, kinds: &[u32], plugged: bool) -> u32 {
    if !plugged { return home; }
    let speakers = kinds.iter().enumerate()
        .filter(|(k, &kind)| kind == SPEAKER && home & 1 << k != 0)
        .fold(0u32, |m, (k, _)| m | 1 << k);
    let rest = home & !speakers;
    if rest == 0 { home } else { rest }
}
