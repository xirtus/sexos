// SPDX-License-Identifier: GPL-3.0-only
//! PDF: reads PDF documents. `run pdf NAME.pdf` opens a file in the program's folder;
//! with no name it opens `welcome.pdf`, writing it there first if it is missing.
//!
//! The first ordinary `std` program: hayro parses and renders the pages (pure Rust,
//! vello_cpu underneath), std::fs reads the file, and the window shows the page.
//!
//! Keys: Right, Space, Page Down or N for the next page; Left, Page Up or P for the
//! previous one; Home and End for the first and last; Up and Down scroll a page taller
//! than the window; + and - zoom, 0 or W fits the page to the window's width, F fits
//! the whole page, 1 shows it at 100 %. Ctrl+F finds text (Enter or F3 for the next
//! hit, Shift for the one before, Escape closes the bar); T shows page thumbnails and
//! O the document's outline, where Up, Down and Enter choose.
mod sample;
// The pure parts, and those that need only hayro, build and are tested on the host too.
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod search;
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod zoom;
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod text;
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod outline;
#[cfg(target_os = "hermit")]
mod viewer;

fn main() {
    #[cfg(target_os = "hermit")]
    viewer::main();
}
