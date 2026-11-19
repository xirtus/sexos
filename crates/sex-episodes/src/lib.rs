// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's "Keep watching" logic with no dependencies: which show, season and episode a
//! file name is, which file comes next ([`episodes`]), and the skip and "Up next" cards
//! ([`segments`]). Shared by Qupid (through `sex_media`), its library and the torrent
//! client's indexer search, which ranks results with [`episodes::similarity`].
#![no_std]
extern crate alloc;

pub mod episodes;
pub mod segments;
