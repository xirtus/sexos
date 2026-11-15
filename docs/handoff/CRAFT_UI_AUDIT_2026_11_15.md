# Craft UI audit (V16), 2026-11-15

Slice V16 of [APP_PARITY_PLAN_2026_11_14.md](APP_PARITY_PLAN_2026_11_14.md): set each SexOS
creative and office app beside the storytold Craft app it starts from and the apps that lead its
field. For each one: what to take from the Craft UI, what in that UI is not good enough, what the
leaders add, and the gaps. Each gap is either fixed here or handed to the app's owner (or to Craft
upstream).

## Sources

- **Craft apps.** Read at these revisions: PhotoCraft c162cb6, VectorCraft 0995c13, FilmCraft
  e6df318 and SoundCraft 2688cca (2026-11-14/15), plus GridCraft c0ee120 and WordCraft a4b898f (the
  revisions Spreadeagle and Assuit build on). What was read:
  - the README;
  - `ROADMAP.md`, for each app's own readiness numbers and ranked gaps;
  - `docs/ui-parity.md`, where the app has one;
  - for GridCraft and WordCraft, the egui UI's keymap and clipboard code.
- **Leaders.** Their public documentation and default behaviour: Excel and LibreOffice Calc, Word
  and Writer, Photoshop, GIMP and Krita, Illustrator and Inkscape, Premiere, Kdenlive and Shotcut,
  Pro Tools, REAPER, Ardour, Logic, Audacity and LMMS.
- **SexOS.** The code at master 990a654c.

The Craft apps measure themselves against the commercial leader. Their roadmaps say so plainly
(numbers as of each roadmap's last headline update):

| Craft app | Target | Feature breadth | Ready for real work | Their first-ranked gap |
|---|---|---|---|---|
| PhotoCraft | Photoshop | ~76% | ~45% | PSDs Photoshop trusts; 22 of 25 performance budgets missed |
| VectorCraft | Illustrator | ~88% | ~62% | interaction-fidelity pass, tool by tool |
| FilmCraft | Premiere | ~86% | ~56% | launch crashes, no CI on pull requests, `.prproj` import |
| SoundCraft | Pro Tools | ~70% | ~40% (pre-alpha) | audio interfaces and buffer size, MIDI input, AAF/OMF |
| GridCraft | Excel | ~75% | ~50% | XLSX fidelity on a real corpus, large workbooks |
| WordCraft | Word | ~80% | ~60% | DOCX corpus round trip, pagination fidelity |

Lesson for SexOS: the Craft **engines** are the valuable part: formats, maths, layout, DSP, codecs,
each with thousands of tests. Their **interfaces** copy the commercial app's menus item by item
(Premiere's 366 menu items, Pro Tools' 512 menu leaves), and the Craft roadmaps name the cost of
that themselves: discoverability, interaction fidelity and stability on real machines are where
"ready" falls behind "breadth". So SexOS takes the engines whole and builds its own interface in
the SexOS style, borrowing Craft's layouts where they are good. It does not copy menu catalogues.

## Summary

| SexOS app | Craft base | Leaders | How the base is used | State on master | Owner |
|---|---|---|---|---|---|
| Spreadeagle (sheets) | GridCraft | Excel, LibreOffice Calc | engine crates as dependencies; own grid UI | T1 + T2 + this PR | office thread (T) |
| Assuit (words) | WordCraft | Word, Writer | engine crates as dependencies; own page view | T3 + pages + this PR | office thread (T) |
| Batik (images) | PhotoCraft (plan) | Photoshop, GIMP, Krita, Pixelmator | native `batik-core`, PhotoCraft not used | layers, masks, brushes, selections, adjustments, PSD/ORA | Gemini |
| Gestalt (vector) | VectorCraft | Illustrator, Inkscape | engine ported; native canvas, 11 tools, SVG/PDF/.ai/EPS open, SVG save, PNG/PDF export | host tests, `program_gestalt_gate.py` | office thread (T) |
| Splicy (video) | FilmCraft (plan) | Premiere, Kdenlive, Shotcut, Scarlet | native `sex-splice` core and export | core and export; no editor window | Splicy thread |
| Moan (DAW) | SoundCraft (plan) | REAPER, Ardour, Logic, Audacity, LMMS | native `moan-core` and `serenade-core` | engine slices and a probe; no window | ChatGPT |
| Burlesque (slides) | none in V16 | Keynote, PowerPoint, Impress | native `burlesque-core` | decks, PPTX/ODP, presenter, rehearse | Gemini |

## Findings that cut across every app

These belong to shared layers, not to any one app. Fixing them once fixes them everywhere.

1. **No system clipboard for std programs.** Shell keeps clipboard history (Super+V), but programs
   built for `x86_64-sex-std.json` have no copy or paste route. Spreadeagle, Assuit, Jot and Quil
   each keep a clipboard of their own, so copying between programs does not work. Wanted: a small
   `sex_std::clipboard` (text first, then HTML and images) over Shell's existing route.
   **Owner:** desktop shell (workstream C).
2. **Keys have only Shift and Ctrl; pointer events have no modifiers at all.** `app_key::Key`
   carries `shift` and `control`, nothing for Alt or Super, and `app_pointer::Pointer` carries
   none. What this blocks:
   - Alt+= (AutoSum) in Excel, so Spreadeagle uses GridCraft's Ctrl+Shift+T instead.
   - Alt-click, which Photoshop, Krita and Pro Tools use to sample or reset a value.
   - Shift-click to extend a selection, and Ctrl-click to add to one. Assuit drags to select for
     this reason.

   **Owner:** input (sexinput and Shell).
3. **One type face for SexOS-drawn interfaces.** `sex-font` ships Geist Regular and Geist Mono
   Regular only. Spreadeagle's grid, which SexOS draws, can show bold only by drawing the text
   twice, and cannot show italic. Assuit's pages are not affected: WordCraft draws them with its
   own bundled OFL families. Wanted: Geist Bold, Italic and Bold Italic in `sex-font`.
   **Owner:** fonts (Gemini's fonts/RTL task).
4. **No shared menu or command palette.** Each office app lists its keys in its status line, and
   that runs out of room. The Craft apps get discoverability from a ribbon or menus. SexOS should
   have one small `sex-office` command palette (Ctrl+Shift+P, as in Quil) listing each app's
   command ids. **Owner:** office thread, next slice.
5. **Agent and test control.** Every Craft app exposes every command through a control channel
   (MCP or CLI), and it is their best feature. SexOS apps record `[app.command] id` lines and run
   Craft command ids, which serves gates the same way. Keep doing that: new apps should name
   commands by id and record them.
6. **Performance budgets.** PhotoCraft misses 22 of its 25 budgets, and FilmCraft and SoundCraft
   list playback lag. When a Craft engine is used, its hot paths need SexOS budgets of their own
   (time to first page, recalculation of a 10k-cell sheet, scrub latency), not only correctness
   gates.

## Spreadeagle and GridCraft

**Take from GridCraft:** the Excel key map, which GridCraft has written out completely
(`ui-egui/src/keymap.rs`, `ribbon/shortcuts.rs`). Spreadeagle now follows it key for key where
SexOS keys allow (table below). Also take its number formats, the AutoFilter button, and charts
that float over cells.

**Not good enough in GridCraft's UI:**
- The ribbon puts about 300 commands one click away, but a beginner has nowhere to start.
- Copy and paste lean on egui's clipboard (`ctx.copy_text`), so an engine-only host such as
  Spreadeagle gets only the workbook's internal clipboard.
- GridCraft's own roadmap ranks first: XLSX files that Excel opens without repair, large
  workbooks, and chart depth.

**Leaders add:**
- Calc: one menu bar, cell styles, the Navigator.
- Excel: Flash Fill, Ideas and Analyze Data, the dynamic-array spill outline, and paste preview.

| Gap (against GridCraft or Excel) | Status |
|---|---|
| Ctrl+C / Ctrl+X / Ctrl+V on a range, with the copied cells outlined | **fixed here** |
| Delete and Backspace empty the whole selection, not only the cursor cell | **fixed here** |
| Ctrl+D fill down, Ctrl+R fill right | **fixed here** |
| AutoSum (Ctrl+Shift+T, GridCraft's key; Alt+= needs Alt, see cross-cutting 2) | **fixed here** |
| Ctrl+B / I / U on the selection; bold and underline drawn, italic kept in the file | **fixed here** |
| Ctrl+F find across sheets, Enter for the next match | **fixed here** |
| F2 edits the cell, as in Excel | **fixed here** |
| Copying between programs | cross-cutting 1 (shell) |
| Find and replace (Ctrl+H) | **fixed** (office slice 2) |
| Go To (Ctrl+G / F5) | next Spreadeagle slice |
| Column resize by dragging | **fixed** (office slice 2) |
| Row and column insert or delete, hide and unhide | next slice |
| Number format keys (Ctrl+Shift+~ and 1..6) | **fixed** (office slice 2) |
| Cell borders | next slice |
| Multi-value filter picker, more chart kinds, moving and resizing charts | next slice |
| ODS writing (saves as `.xlsx` today) | next slice |
| PivotTables, conditional formatting UI, Get & Transform | later (GridCraft roadmap) |

## Assuit and WordCraft

**Take from WordCraft:**
- Its shortcut registry. Assuit asks it for every chord (`registry.by_shortcut`), so Ctrl+C,
  Ctrl+X, Ctrl+V, Ctrl+A, Ctrl+B, Ctrl+I, Ctrl+U, Ctrl+Z and the caret keys already behave as in
  Word.
- Continuous pages and selection rectangles from the layout.

**Not good enough in WordCraft's UI:**
- Ctrl+F only opens an egui dialog: the engine's `edit.find` with no text does nothing for a host
  without that dialog.
- Its own roadmap ranks pagination fidelity and the DOCX corpus first.

**Leaders add:**
- Writer: the Navigator, styles at the sidebar, and a word count in the status bar.
- Word: Focus mode, Read Aloud, and live collaboration.

| Gap | Status |
|---|---|
| Find (Ctrl+F prompt, Enter selects the next match, "match i of n") | **fixed here** |
| Word count in the status bar | **fixed here** |
| Copy, cut and paste inside the document (WordCraft's clipboard) | works; now gated |
| Copying between programs | cross-cutting 1 (shell) |
| Find and replace (Ctrl+H) | **fixed** (office slice 2) |
| Go To page | next Assuit slice |
| Styles picker (Ctrl+Shift+S) and the paragraph's style in the status line | **fixed** (office slice 2) |
| Navigation pane of headings | next slice |
| Shift-click to extend a selection | cross-cutting 2 (input) |
| Loom file picker for Open and Save As | waits on Loom (Q) |
| Tables, pictures, track changes UI, comments | later (WordCraft has the engine) |

## Batik and PhotoCraft (Photoshop, GIMP, Krita)

Batik is native (`batik-core`, about 6.5k lines), not a port of PhotoCraft.

**Take from PhotoCraft:**
- The tool groups with flyouts and Shift-cycling, and the options bar that changes with the tool.
- Live filter previews on the canvas, through the selection.
- History with a history brush.
- The behaviour notes in `docs/ui-parity.md` and `docs/brush-parity.md` (cursor, nudge, modifier
  and transform-handle rules), which are written to be reused.

**Not good enough in PhotoCraft:** brush and transform feel and the missed performance budgets
are its own top-five gaps. Its menus (628 items) copy Photoshop's.

**Leaders add:**
- Krita: canvas rotation, a brush engine built for painting, wrap-around mode, and a pop-up
  palette on right-click.
- GIMP: the image-menu layout.
- Pixelmator: one-window simplicity.

| Gap in Batik | Owner |
|---|---|
| Zoom, pan and rotate the canvas (no zoom anywhere in `apps/batik`) | Gemini |
| Move and Free Transform on the canvas (`batik-core` has transform, the UI does not) | Gemini |
| Crop tool in the UI (core has crop) | Gemini |
| Eyedropper (core has it, no tool), gradient tool, clone stamp and healing | Gemini |
| Shape and pen tools, paths | Gemini, or shared with Gestalt |
| Layer styles (drop shadow, stroke, glow) | Gemini |
| Tablet tilt and the USB digitizer (V3) | input + Gemini |
| Alt-click sampling | cross-cutting 2 |

## Gestalt and VectorCraft (Illustrator, Inkscape)

Gestalt now runs VectorCraft's whole engine (2026-11-15) instead of `apps/shapes`' demo scene:
the canvas, tool strip, overlays and keys are native (`apps/gestalt/src/ui.rs`), and every edit
is an engine command or tool event, so undo, Smart Guides, Pathfinder and the formats are
VectorCraft's. The engine is 59 MB of code even size-optimised, so the program image limit
went from 32 to 64 MiB (`sex_pdx::program_store::IMAGE_BYTES`, `sex_pkg::PROGRAM_MAX`).

**Take:** Smart Guides, Shape Builder, the Appearance panel (live effects as a stack), Pathfinder,
and Image Trace.

**Not good enough:** VectorCraft ranks its interaction fidelity (pen, direct selection,
snapping) and `.ai` files from real users as its first two gaps.

**Leaders add:**
- Inkscape: the XML editor, Live Path Effects, and SVG as the native format.
- Illustrator: artboards in the Properties panel, and Retype.

| Gap | Owner |
|---|---|
| ~~The whole app (V4): SVG open, edit, save; paths and nodes; text; PNG/PDF export~~ | fixed 2026-11-15 (office thread) |
| ~~SVG as the native file format, Inkscape-style, with `.ai` and PDF import~~ | fixed 2026-11-15 |
| Panels: Layers, Appearance, Swatches, Properties with artboards | office thread (T), with egui on sex-app |
| ~~Shift- and Alt-drags (constrain, copy)~~ | fixed 2026-11-15 with PR #299's pointer modifiers |
| Image Trace, Shape Builder polish, `.ai` files from real users | office thread (T) |

## Splicy and FilmCraft (Premiere, Kdenlive, Shotcut, Scarlet)

Splicy has its editing core (`sex-splice`: timeline, ops, transcript edits, suggestions, OTIO) and
WebM, H.264 and AV1 export. It has no editor window yet.

**Take from FilmCraft:** the workspaces (Edit, Color, Audio), which are good. Also the Source and
Program monitors, the trim modes with a Trim Monitor, and scopes as a separate crate.

**Not good enough in FilmCraft** (its own `ui-parity.md`):
- No transform handles in the Program monitor.
- Effect Controls without keyframe zoom.
- No continuous filmstrip, and tracks cannot be renamed.
- Docking reported broken.
- No screen reader.

**Leaders add:**
- Shotcut: one simple timeline and filters as a list.
- Kdenlive: proxy clips by default, and a render queue.
- Scarlet: editing by transcript (already in `sex-splice`).

| Gap | Owner |
|---|---|
| Editor window: monitors, timeline, clip list, using `sex-splice` | Splicy thread |
| Transform handles on the monitor from the start (FilmCraft's #639) | Splicy thread |
| Keyframe lanes with zoom; filmstrip thumbnails; track rename | Splicy thread |
| Scopes (waveform, vectorscope, histogram) | Splicy thread (V11) |

## Moan and SoundCraft (REAPER, Ardour, Logic, Audacity, LMMS)

Moan has `moan-core` (timeline and stereo mixer) and `serenade-core` (recorder, meters, waveform
mipmaps, WAV). The guest probe mixes two tracks into Bell. Moan has no window yet.

**Take from SoundCraft:** the separate Edit and Mix windows, Strip Silence with a live preview of
what it keeps, Clip Looping, and the built-in instruments it synthesises itself.

**Not good enough in SoundCraft:**
- It is pre-alpha by its own gate: there is no choice of audio device or buffer size, no MIDI
  input, and takes are held in RAM.
- It clones Pro Tools' menus (482 of 512 leaves).
- It has no sound browser.

**Leaders add:**
- REAPER: any-to-any routing, actions as a scriptable list, and small size.
- Ardour: its session model.
- Logic: Live Loops and Drummer.
- Audacity: destructive editing and spectral view.
- LMMS: patterns.

| Gap | Owner |
|---|---|
| Edit window with tracks, clips and transport over `moan-core` | ChatGPT |
| Record to disk (not RAM) and Bell buffer-size choice (V13) | ChatGPT + Bell |
| USB MIDI input (V14) | ChatGPT + sexusb |
| Sound browser with previews (SoundCraft's G-16 too) | ChatGPT |

## Fixed in this PR

- **Spreadeagle**
  - Copy, cut and paste of ranges, with an outline on the copied cells.
  - Delete empties the selection.
  - Fill down and fill right.
  - AutoSum.
  - Bold, italic and underline.
  - Find across sheets.
  - F2 to edit a cell.

  Covered by host test `copy_paste_fill_sum_style_and_find` and by the gate: Ctrl+F "rent" lands
  on A4, then Ctrl+C, Ctrl+V into A8, and Ctrl+Z.
- **Assuit**
  - Find prompt.
  - Word count.
  - Copy and paste, now tested.

  Covered by host test `find_selects_matches_and_copy_pastes` and by the gate: Ctrl+F "assuit",
  Enter twice, then copy, and paste on page 2.

## Handed on

- Cross-cutting 1 (clipboard): shell thread C.
- Cross-cutting 2 (modifiers): input.
- Cross-cutting 3 (fonts): Gemini fonts task.
- Batik table: Gemini.
- Splicy table: Splicy thread.
- Moan table: ChatGPT.
- Gestalt: needs an owner for V4. The recommendation above is to port VectorCraft's engine
  crates.
