# Program windows show only finished frames (two pixel pages per window)

- `crates/sex-pdx/src/pixel_region.rs`: each program row's pixel region holds two
  pages (`PAGES`, `page_bytes`, `region_for_screen`). `OP` (0x2D2) names the page in
  arg2; Display answers `shown(page)` once it reads that page.
- `kernel/src/boot_memory.rs`: regions are two screen-sized pages; one buddy block per
  four rows, so each block is the same size eight single-page rows asked for. ABI
  change: OP arg2 (was unused), region length doubled.
- `servers/sexdisplay`: reads only the page named last (`PIXEL_PAGE`); the `shown`
  answer is retried each loop while the program's reply slot is full.
- `crates/sex-app`: `Window::pixels` is the page off screen, holding the last frame
  (copied after the switch); `frame` skips that copy for programs that paint every
  pixel; `show_pixels` flips. Before handing out a page, the window waits (up to
  100 ms) for Display's answer, keeping keys and answers that arrive in the mailbox.
  `Window::room` gives the page size without waiting.
- SDL keeps its own framebuffer and copies it in at present (the page is no fixed
  address any more); Quil takes the page at each draw.
- Fixes Display compositing (cursor, clock, neighbour damage) catching any program
  half drawn: SDL games, Kaleidoscope, the console, Weft, every sex-app program.
- Display's `shown` answer counts as an answer (`is_answer`), so a task waiting for
  Files that takes it keeps it in the mailbox instead of dropping it. Dropped, every
  frame after a Files call waited the full 100 ms; Spindle then took a native
  program's output too slowly and the relay lost a line (Terminal gate "attached
  output has gaps").
- A shown answer overdue by 4 ms is asked for again (`OP` arg1 = `ASK`); waits of
  4 ms or more log `[app.pages.late]` (first 16). `Window::ready()` says, without
  waiting, whether the back page is free: Qupid drops a picture instead of holding
  up decoding and sound while Display finishes a frame.
- Memory: BOOT gives whole-screen pages only with 1.5 GB free (`ROOMY_BYTES`);
  otherwise the region stays one screen (as before) split in half pages, and a
  window bigger than a half shows `WHOLE` (page 2), painted in place. 512 MB gates
  ran out of program memory with doubled regions (Qupid `no_memory`).

## Later review (Andreas, 2026-11-25)

- Qupid's Kodi add-on path is marked for later review, not part of this change. This
  change only made the Kodi gate fit in 512 MB again (half pages); the add-on itself
  (sources, playback through Kodi plug-ins) gets its own review later.
