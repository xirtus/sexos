// SPDX-License-Identifier: GPL-3.0-only
//! Bell picks the HD Audio microphone the way people expect.
#[path="../servers/sexbell/src/mic.rs"]mod mic;
use mic::*;

// Realtek ALC-style default configurations.
const INTERNAL_MIC: u32 = 0x90a6_0130; // fixed, internal, mic in
const JACK_MIC: u32 = 0x03a1_9020;     // jack, external left, mic in
const LINE: u32 = 0x0181_3040;         // jack, rear, line in
const UNWIRED: u32 = 0x4000_0000;
const SPEAKER: u32 = 0x9017_0110;
const IN_CAPS: u32 = 1 << 5 | 1 << 2 | 0x37 << 8; // input, presence, VRefs HiZ/50/Gnd/80/100

#[test] fn internal_mic_ranks_first() { assert_eq!(input(INTERNAL_MIC, IN_CAPS).unwrap().rank, 0); }
#[test] fn internal_mic_has_no_presence() { assert!(!input(INTERNAL_MIC, IN_CAPS).unwrap().sense); }
#[test] fn jack_mic_ranks_second() { let i = input(JACK_MIC, IN_CAPS).unwrap(); assert_eq!((i.rank, i.sense), (1, true)); }
#[test] fn line_in_ranks_third() { assert_eq!(input(LINE, IN_CAPS).unwrap().rank, 2); }
#[test] fn unwired_and_outputs_are_not_inputs() {
    assert_eq!(input(UNWIRED | 0xa << 20, IN_CAPS), None);
    assert_eq!(input(SPEAKER, IN_CAPS), None);
    assert_eq!(input(INTERNAL_MIC, 1 << 4), None); // output-only pin
    assert_eq!(input(JACK_MIC, IN_CAPS | 1 << 7), None); // HDMI
}
#[test] fn plugged_jack_mic_wins() {
    let ins = [input(INTERNAL_MIC, IN_CAPS).unwrap(), input(JACK_MIC, IN_CAPS).unwrap()];
    assert_eq!(choose(&ins, &[false, true]), Some(1));
    assert_eq!(choose(&ins, &[false, false]), Some(0));
}
#[test] fn empty_jack_falls_behind_line_in() {
    let ins = [input(JACK_MIC, IN_CAPS).unwrap(), input(LINE, 1 << 5).unwrap()];
    assert_eq!(choose(&ins, &[false, false]), Some(1));
}
#[test] fn qemu_line_in_alone() {
    // QEMU's hda-duplex: one line-in jack, no presence detect.
    let ins = [input(0x0081_3040 & !(3 << 30), 1 << 5).unwrap()];
    assert_eq!(choose(&ins, &[false]), Some(0));
}
#[test] fn none_without_inputs() { assert_eq!(choose(&[], &[]), None); }
#[test] fn mic_bias_prefers_80() {
    assert_eq!(pin_control(MIC_IN, IN_CAPS), 0x24);
    assert_eq!(pin_control(MIC_IN, 1 << 5 | 0x20 << 8), 0x25);
    assert_eq!(pin_control(MIC_IN, 1 << 5), 0x20);
    assert_eq!(pin_control(LINE_IN, IN_CAPS), 0x20);
}
