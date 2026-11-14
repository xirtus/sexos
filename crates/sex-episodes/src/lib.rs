// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's pure-Rust media intelligence with no external dependencies (`#![no_std]`):
//! - [`episodes`]: which show, season and episode a file name is, and which file comes
//!   next by release-group/codec/resolution similarity.
//! - [`segments`]: intro/recap/credits skip cards and the "Up next" countdown.
//! - [`audio_dsp`]: real-time `50%..=300%` projector volume amplification, 3-band RBJ
//!   biquad equalizer, `5.1`/`7.1` centre-channel dialogue lift downmix, upward/downward
//!   dialogue DRC and soft-knee peak limiting for `[[i16; 2]]` stereo PCM.
//! - [`pack`]: clean open catalog defaults plus compile-time seed and runtime
//!   `provider-pack.json` hydration for Stremio addons, swarm indexers, featured feeds
//!   and `sex_kodi` repositories.
//! - [`streams`]: non-truncated stream telemetry (`👤` seeds, `AV1`/`HEVC`, `💾` size,
//!   `⚙️` source), multi-file season-pack episode matching, viability ranking and
//!   tracker-enriched magnet URI construction.
#![no_std]
extern crate alloc;

pub mod audio_dsp;
pub mod episodes;
pub mod pack;
pub mod segments;
pub mod streams;
