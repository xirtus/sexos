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
