#![no_std]
// Vendored pp-rs 0.2.1 (MIT/Apache-2.0), made no_std + alloc for SexOS programs.
extern crate alloc;
#[cfg(test)]
extern crate std;
extern crate unicode_xid;

#[allow(clippy::match_like_matches_macro)]
mod lexer;
pub mod pp;
pub mod token;

#[cfg(test)]
mod lexer_tests;
#[cfg(test)]
mod pp_tests;
