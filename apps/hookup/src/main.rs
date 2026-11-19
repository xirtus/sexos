// SPDX-License-Identifier: GPL-3.0-only
//! Hookup: your phone and this computer, linked over the KDE Connect protocol
//! (crates/sex-kdeconnect, written from the protocol's public description). A phone
//! running KDE Connect finds this computer, pairs with it, and then its
//! notifications, clipboard and media player show here; files it shares land in
//! `received/` in Hookup's folder, and files given to Hookup go the other way.
//!
//! `run +net hookup [HOST:PORT ...] [@FILE ...]`: each HOST:PORT also gets the identity
//! announcement (besides the broadcast address); each @FILE is offered to the paired
//! device with `s`.
//!
//! Keys: Up and Down pick a device; `p` asks it to pair, `y` accepts its request and
//! `n` refuses it, `u` unpairs; `s` sends the given files; `c` sends the clipboard text
//! received last back as ours; Space plays or pauses its media, Left and Right go to
//! the previous and next track; `r` announces again; F11 closes.
//!
//! Links are not encrypted yet (see sex-kdeconnect): the window says so.
#[cfg(target_os = "hermit")]
mod draw;

#[cfg(target_os = "hermit")]
mod app;

fn main() {
    #[cfg(target_os = "hermit")]
    app::main();
}
