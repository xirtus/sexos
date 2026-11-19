// SPDX-License-Identifier: GPL-3.0-only
//! Kodi addon repositories, natively in Rust, for Qupid (slice Q13). What brootlite's
//! Kodi bridge did in Python before an addon ran, done here without starting Python:
//!
//! - [`xml`]: the small, forgiving XML reader Kodi's files need.
//! - [`version`]: Kodi version order (`1.2.0~beta1` < `1.2.0` < `1.2.0+matrix.1`).
//! - [`addon`]: an `addon.xml` entry: id, version, `<requires>`, what it is (video
//!   plugin, module, repository...), summary, icon, broken marks.
//! - [`repo`]: a repository addon's `<dir>`s: where `addons.xml` and its checksum
//!   live, and where each addon's zip is.
//! - [`catalogue`]: every repository's addons merged, newest compatible version wins,
//!   and an install plan that resolves `<requires>` (including `script.module.*`)
//!   dependencies first, skipping what Qupid itself provides (`xbmc.python` 3).
//! - [`check`]: the checksums repositories publish (MD5, SHA-1, SHA-256, SHA-512) and
//!   [`check::verify`] for a downloaded file.
//! - [`package`]: an addon zip opened, checked to hold only `<id>/...` members and the
//!   promised `addon.xml`, and unpacked into memory.
//!
//! No networking or files here: Qupid fetches with sex_fetch and stores in the
//! addon folder it was granted. Only running a plugin needs the Python helper.
#![no_std]
extern crate alloc;

pub mod addon;
pub mod catalogue;
pub mod check;
pub mod package;
pub mod repo;
pub mod version;
pub mod xml;

pub use addon::{Addon, Kind, Require};
pub use catalogue::{Catalogue, PlanError, Provided};
pub use repo::{Dir, Repository};
