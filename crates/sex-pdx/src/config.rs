// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Settings, kept by Files.
//!
//! Files holds the one live settings table (the schema is `sex-config`) and its
//! text on disk. Anyone with a route to Files may read it; only the Shell (over its
//! program route), Spindle and the program run from `/data/settings` may change it.
//! A value's key and range are checked by Files, never trusted from the caller.
//!
//! Live reload is by generation: every accepted change grows Files' generation.
//! A watcher asks `GENERATION`, and when it grew, walks `NEXT` from the generation
//! it last saw to get each changed key once, with its value. Answers come back as
//! ordinary replies (`type_id` 1 from Files) tagged so they never read as another
//! protocol's.
//!
//! Request: `type_id` [`OP`], arg0 = verb | key << 8, arg1 = value or generation,
//! arg2 = 0 (reserved).

pub use sex_config::wire::*;
/// The settings schema and text format.
pub use sex_config as schema;
