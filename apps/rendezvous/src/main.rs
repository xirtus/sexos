// SPDX-License-Identifier: GPL-3.0-only
//! Rendezvous: where you meet. OpenStreetMap maps from vector tiles (raster tiles
//! where a server has no vector ones), place search, routes, and regions kept for
//! use offline. The tiles, drawing, search and route formats are crates/sex-map's,
//! tested on the host; this is the window and the network around them.
//!
//! `run +net rendezvous [http://SERVER] [LAT,LON[,ZOOM]]`: a server URL is kept in
//! `servers.txt` in the program's folder (lines `vector=`, `raster=`, `search=`,
//! `route=` with `{z}/{x}/{y}` in tile URLs, `max_zoom=`; `base=` sets them all to
//! one server's /tiles, /raster, /search and /route/v1).
//!
//! Keys: arrows move the map, `+` and `-` (or Page Up and Page Down) zoom, Home goes
//! back to where it started; `/` searches, Up and Down pick a result and Enter goes
//! there; `r` draws a route from the place chosen before to the one chosen now (or
//! from the map's centre); `o` keeps the area shown for use offline; Esc closes the
//! search or clears the route. The mouse drags the map and its wheel zooms.
#[cfg(target_os = "hermit")]
mod app;

fn main() {
    #[cfg(target_os = "hermit")]
    app::main();
}
