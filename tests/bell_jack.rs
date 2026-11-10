// SPDX-License-Identifier: GPL-3.0-only
//! Bell follows the headphone jack: speakers quiet while headphones are in.
#[path="../servers/sexbell/src/jack.rs"]mod jack;
use jack::*;

#[test] fn unplugged_plays_home() { assert_eq!(wanted(0b011, &[SPEAKER, HEADPHONES], false), 0b011); }
#[test] fn plugged_quiets_speakers() { assert_eq!(wanted(0b011, &[SPEAKER, HEADPHONES], true), 0b010); }
#[test] fn line_out_keeps_playing() { assert_eq!(wanted(0b111, &[SPEAKER, HEADPHONES, 0], true), 0b110); }
#[test] fn speakers_alone_keep_playing() { assert_eq!(wanted(0b001, &[SPEAKER], true), 0b001); }
#[test] fn outputs_outside_home_stay_off() { assert_eq!(wanted(0b001, &[SPEAKER, HEADPHONES], true), 0b001); }
