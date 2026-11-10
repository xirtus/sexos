// SPDX-License-Identifier: GPL-3.0-only
//! PDF: reads PDF documents. `run pdf NAME.pdf` opens a file in the program's folder;
//! with no name it opens `welcome.pdf`, writing it there first if it is missing.
//!
//! The first ordinary `std` program: hayro parses and renders the pages (pure Rust,
//! vello_cpu underneath), std::fs reads the file, and the window shows the page.
//!
//! Keys: Right, Space, Page Down or N for the next page; Left, Page Up or P for the
//! previous one; Home and End for the first and last; Up and Down scroll a page taller
//! than the window; + and - zoom, 0 fits the page to the window's width.
mod sample;
#[cfg(target_os = "hermit")]
mod viewer;

fn main() {
    #[cfg(target_os = "hermit")]
    viewer::main();
}
