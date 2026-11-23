# Splicy: Scarlet updates of 2026-11-20..22, 2026-11-23

Deterministic rules from Scarlet (MIT, Forward-Future/scarlet) brought into Splicy and
Splicy Studio. Only Splicy branding shows in either app.

- **Restarts** (`crates/sex-splice/src/restarts.rs`): a word cut off and said again in full
  ("I was gonna sa- say that") joins the restart; new `Transcript::doubled_words()` finds a word
  said twice in a row ("the the") and takes out the first, leaving words people do double
  ("no no", "very very", "bye bye"). Splicy's clean-up shows a "Remove N doubled words" card.
- **Cut edges** (`edges.rs`): across a pause the cut now sits 50 ms from the kept words (room
  before a breath stays), never inside a kept word that is still sounding (15 dB above the room
  for 60 ms), and noisy rooms raise the room-tone, trough and low-point bars from the clip's own
  levels (`Levels::bars()`; under 1 s of levels keeps the studio bars).
- **J/L cuts** (`split_edit.rs`, `Splicy::split_edit`): at the picture join nearest the caret,
  `j` brings the next voice in 250 ms early and `l` lets the last voice run 250 ms on. A clip
  whose sound is in its picture gets a copy on a free sound track first.
- **OTIO export** (`otio.rs`): a muted sound track and a turned-off clip go into the file with
  OTIO's `enabled: false` instead of as gaps; a sped-up clip's range is its timeline length with
  a `LinearTimeWarp` effect carrying the speed.
- **Splicy Studio**: `transcript.fixWord {find, replace, item?}` corrects a word or phrase in
  every transcript, keeping word times, case and punctuation (`core/crates/edit/src/transcript/fix.rs`);
  CLI `--fix-word=FIND:REPLACE`.
- Skipped: music ducking and beat-matched cuts, which Splicy Studio already has (Essential Sound
  ducking, Remix beats); agent prompts, pricing and Scarlet's server plumbing.

Tests: `cargo test -p sex-splice` (84), `cargo test -p splicy --no-default-features` (15),
Studio word fix tests in `core/crates/edit/src/transcript/tests.rs`.
