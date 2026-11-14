// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Voyeur, the SexOS system search (KRunner, Spotlight, Raycast).
//!
//! Search providers are programs (or Shell-internal tables) that answer a
//! query with results taken only from what they already hold. A provider
//! never receives more than the query text; the Shell scores every result
//! itself against the query, adds its own frecency (how often and how
//! recently the user chose that result) and merges all providers into one
//! list. The wire protocol is `sex_pdx::search`.
//!
//! This crate is the pure, host-tested part: matching and ranking
//! ([`rank`]), the minimal Abacus calculator provider ([`abacus`]) and the
//! static tables the Shell's built-in providers search ([`tables`]).
//! Nothing here allocates, holds authority or calls PDX.
#![cfg_attr(not(test), no_std)]

pub mod abacus;
pub mod rank;
pub mod tables;
#[cfg(test)]
mod tests;

pub use rank::{merge, Candidate, Frecency, Ranked};

/// What a result is. The numbers are the wire values of `sex_pdx::search`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App = 1,
    File = 2,
    Content = 3,
    Setting = 4,
    Calculator = 5,
    Window = 6,
    History = 7,
    Bookmark = 8,
    Clipboard = 9,
    Emoji = 10,
    Command = 11,
    Web = 12,
    Other = 15,
}

impl Kind {
    pub const fn decode(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::App, 2 => Self::File, 3 => Self::Content, 4 => Self::Setting,
            5 => Self::Calculator, 6 => Self::Window, 7 => Self::History, 8 => Self::Bookmark,
            9 => Self::Clipboard, 10 => Self::Emoji, 11 => Self::Command, 12 => Self::Web,
            15 => Self::Other, _ => return None,
        })
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::App => "app", Self::File => "file", Self::Content => "content",
            Self::Setting => "setting", Self::Calculator => "calculator", Self::Window => "window",
            Self::History => "history", Self::Bookmark => "bookmark", Self::Clipboard => "clipboard",
            Self::Emoji => "emoji", Self::Command => "command", Self::Web => "web", Self::Other => "other",
        }
    }
}

/// What choosing a result does. The provider that answered performs it,
/// except the actions the Shell owns (focus, copy, reveal in sexfiles).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open = 1,
    OpenWith = 2,
    Reveal = 3,
    Copy = 4,
    Focus = 5,
    Run = 6,
}

impl Action {
    pub const fn decode(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Open, 2 => Self::OpenWith, 3 => Self::Reveal,
            4 => Self::Copy, 5 => Self::Focus, 6 => Self::Run, _ => return None,
        })
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open", Self::OpenWith => "open_with", Self::Reveal => "reveal",
            Self::Copy => "copy", Self::Focus => "focus", Self::Run => "run",
        }
    }
}
