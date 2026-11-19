// SPDX-License-Identifier: GPL-3.0-only
//! Which HD Audio input is the microphone: pure, tested on the host (tests/bell_mic.rs).
//!
//! A laptop codec usually has an internal microphone (a fixed pin), a headset or
//! microphone jack, and sometimes a line in. Bell records from a plugged-in jack
//! microphone first, then the internal one, then a jack microphone it cannot sense,
//! then line in: the same order people expect from other systems.

/// Default-configuration device kinds (HDA 1.0a, section 7.3.3.31).
pub const LINE_IN: u32 = 0x8;
pub const MIC_IN: u32 = 0xa;

/// Pin capability bits (HDA 1.0a, section 7.3.4.9).
const CAP_PRESENCE: u32 = 1 << 2;
const CAP_INPUT: u32 = 1 << 5;
const CAP_HDMI: u32 = 1 << 7;
const CAP_DP: u32 = 1 << 24;

/// One input pin as the codec describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input { pub device: u32, pub internal: bool, pub sense: bool, pub rank: u32 }

/// The pin as a microphone candidate, or None when it cannot record or is not a
/// microphone or line in. `config` is its default configuration, `caps` its pin
/// capabilities. Rank 0 is the internal microphone, 1 a jack microphone, 2 line in.
pub fn input(config: u32, caps: u32) -> Option<Input> {
    if caps & CAP_INPUT == 0 || caps & (CAP_HDMI | CAP_DP) != 0 { return None; }
    // Port connectivity: 0 jack, 1 nothing wired, 2 fixed (built in), 3 both.
    let port = config >> 30;
    if port == 1 { return None; }
    let device = config >> 20 & 0xf;
    let internal = port == 2 || port == 3 && config >> 24 & 0x3f == 0x10;
    let rank = match device { MIC_IN if internal => 0, MIC_IN => 1, LINE_IN => 2, _ => return None };
    Some(Input { device, internal, sense: caps & CAP_PRESENCE != 0 && !internal, rank })
}

/// The microphone to record from: a plugged-in jack microphone (`plugged[k]` is
/// what its presence detect says), else the best ranked input. None with no inputs.
pub fn choose(inputs: &[Input], plugged: &[bool]) -> Option<usize> {
    let jack = inputs.iter().enumerate()
        .position(|(k, i)| i.device == MIC_IN && i.sense && plugged.get(k).copied().unwrap_or(false));
    jack.or_else(|| inputs.iter().enumerate().min_by_key(|(k, i)| {
        // A jack microphone known to be empty records nothing: after line in.
        let empty = i.sense && !plugged.get(*k).copied().unwrap_or(false);
        (if empty { 3 } else { i.rank }, *k)
    }).map(|(k, _)| k))
}

/// The pin-control word that enables recording, with microphone bias (VRef) for a
/// microphone: 80 % when the pin offers it, else 100 %, else 50 %.
pub fn pin_control(device: u32, caps: u32) -> u32 {
    const IN_ENABLE: u32 = 0x20;
    if device != MIC_IN { return IN_ENABLE; }
    let vref = caps >> 8 & 0xff;
    IN_ENABLE | if vref & 1 << 4 != 0 { 4 } else if vref & 1 << 5 != 0 { 5 } else if vref & 1 << 1 != 0 { 1 } else { 0 }
}
