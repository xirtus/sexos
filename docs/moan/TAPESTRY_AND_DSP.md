# Moan Tapestry & Pro Studio DSP Architecture

This document specifies Moan's **Tapestry** multi-channel pattern workstation, exact rational musical time engine, expressive and physical-modeled `#![no_std]` instruments, Piano Roll compositional transforms, Standard MIDI File (`.mid`) interchange, and Pro Console mixing and mastering DSP chain.

All core DSP, timing, composition, and SMF engines live in `crates/moan-core/src/` with zero external crate dependencies, deterministic fixed-order sample processing at `RATE = 48_000` Hz, bounded `try_reserve` / `try_reserve_exact` allocations at setup/edit time, and zero heap allocations inside per-sample audio render loops.

---

## 1. Tapestry Pattern Workstation

**Modules**: `crates/moan-core/src/pattern.rs`, `crates/moan-core/src/kits.rs`, `apps/moan/src/tapestry_ui.rs` (UI integration)

Tapestry unifies beat-oriented step sequencing with expressive Piano Roll editing and clip-based song arrangement over a single canonical data model.

```
+-----------------------------------------------------------------------------+
|                              TapestryBank                                   |
|                                                                             |
|  Shared Channel Rack (<= 32 RackChannels)                                   |
|  [ Ch 0: Punch Kick ] [ Ch 1: Snare ] [ Ch 3: Closed Hat (cut 1) ] ...      |
|           |                     |                 |                         |
|           +---------------------+-----------------+                         |
|                                 v                                           |
|  Arc<TapestryPattern> (<= 256 patterns, 1..=16 bars, 1/16..1/64 & triplets) |
|  -> Canonical (ChannelId, Vec<ExpressiveNote>)                              |
|     * Step Sequencer Grid (StepCell) <-> Piano Roll (ExpressiveNote)        |
|     * ChannelPreviewMode: StepGrid (one-shots) vs MiniRoll (melodic/chords) |
|                                 |                                           |
|          +----------------------+----------------------+                    |
|          v                                             v                    |
|  PatternInstance Clips (<= 1024)              SceneMatrix (8 lanes x 8)     |
|  * Linked via PatternId                       * Bar-quantized queue_scene() |
|  * Make Unique -> clone_pattern() + rebind    * advance_to() emits clips    |
+-----------------------------------------------------------------------------+
                                  |
                                  v
            realize_events(PatternMode | SongMode)
            -> TempoMap + Swing + Mute/Solo + Cut/Choke Groups
            -> Vec<RealizedChannelNote> (exact beats + 48 kHz frames)
```

### 1.1 Shared Channel Rack (`RackChannel`)
- `TapestryBank` owns up to `MAX_RACK_CHANNELS = 32` shared [`RackChannel`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/pattern.rs#L30-L43) strips across all patterns in a project.
- Each channel carries `id: ChannelId`, `name`, `instrument: u8` (`0..=11`), `root_key` (`0..=127`, default `60`), `volume_pct` (`0..=400`, default `100`), `pan` (`-100..=100`), `muted`, `solo`, `cut_group: u8`, and `target_track` (`0..MAX_TRACKS`, `MAX_TRACKS = 8`).
- **Cut / Choke Groups (`cut_group`)**: Any note emitted on a channel with nonzero `cut_group` truncates all earlier sounding notes in the same `cut_group` at its exact `start_frame` (and `start_beat`) during `TapestryBank::realize_events`. Standard kits assign `CUT_GROUP_HIHAT = 1` to Closed Hat / Open Hat pairs so closed hats choke ringing open hats, and `CUT_GROUP_MONO = 2` to monophonic bass or vocal chop channels.

### 1.2 Unified Step Sequencer & Piano Roll (`TapestryPattern`)
- A [`TapestryPattern`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/pattern.rs#L109-L117) stores `bars` (`1..=16`, `MAX_BARS`), `steps_per_beat` (`1..=16`), and per-channel chronologically sorted `Vec<ExpressiveNote>` vectors.
- Supported step subdivisions:
  - `steps_per_beat = 4`: `1/16` (16 steps per `4/4` bar; 16 / 32 / 64 steps for 1 / 2 / 4 bars)
  - `steps_per_beat = 3`: `1/12T` (eighth-note triplets)
  - `steps_per_beat = 6`: `1/16T` (sixteenth-note triplets)
  - `steps_per_beat = 8`: `1/32` (32nd-note trap hi-hat grid)
  - `steps_per_beat = 16`: `1/64` (64th-note micro-rolls)
- **Single Source of Truth**: Step-grid reads (`step_grid`) and edits (`toggle_step`, `set_step`) operate directly on the underlying `ExpressiveNote` vectors—no dual synchronization state can drift.
  - `StepCell` projects velocity, pan, `fine_cents`, `slide` (`slide_to_key`), and ratchet rolls (`repeat_subdiv` in `1..=4`, subdividing a single step into 2–4 rapid sub-notes).
  - `TapestryPattern::preview_mode` automatically classifies each channel as `ChannelPreviewMode::StepGrid` (one-shot root hits) or `ChannelPreviewMode::MiniRoll` (varied pitches, sustained notes longer than one step, overlapping chords, or pitch slides).
- **Beat-Group Step Colors**: `STEP_BEAT_COLORS` and `step_group_color(step_idx, steps_per_beat)` in `kits.rs` alternate 4 distinct beat colors (`0x2d3e54`, `0x3f3247`, `0x28414f`, `0x3b3040`) across every beat group so 16/32/64-step bars read at a glance.

### 1.3 Linked `PatternInstance` Clips, CoW Forking & Transport Modes
- Patterns are stored inside `TapestryBank` as `Vec<Arc<TapestryPattern>>` (`MAX_PATTERNS = 256`).
  - `TapestryBank::pattern_mut` uses `Arc::make_mut` for copy-on-write mutation when an edit targets a shared pattern.
  - Up to `MAX_INSTANCES = 1024` [`PatternInstance`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/pattern.rs#L380-L386) clips can be arranged across `SCENE_LANES = 8` playlist lanes with `start_beat`, `length_beats`, and `offset_beats`. Sibling instances pointing to the same `PatternId` stay linked—editing the pattern updates every linked clip.
  - **Make Unique (`make_instance_unique`)**: Clones the pattern referenced by `instances[instance_idx]` into a fresh `PatternId` (`"<Name> (Copy)"`) via `clone_pattern` and rebinds only that instance, leaving all sibling instances linked to the original pattern.
  - **Split by Channel (`split_pattern_by_channel`)**: Explodes a multi-channel pattern into separate single-channel patterns (one per non-empty channel) for independent song arrangement.
- **Transport Modes (`TransportContext`)**:
  - `TransportContext::PatternMode`: Realizes only `bank.active_pattern` starting at beat `0` for loop auditioning and step programming.
  - `TransportContext::SongMode`: Realizes every placed `PatternInstance` sliced by `[offset_beats, offset_beats + length_beats)` at `start_beat`, with Mute/Solo, `TempoMap`, global `swing_pct`, and `cut_group` choking applied across all active clips.

### 1.4 Bar-Quantized `SceneMatrix` & `RetrospectiveNoteBuffer`
- **`SceneMatrix`**: An `8`-lane (`SCENE_LANES`) × `8`-scene (`SCENE_COUNT`) live performance matrix of `SceneRow`s.
  - `queue_scene` / `queue_scene_for_bank` computes the exact next bar boundary `(launch_beat, launch_frame)` from `TempoMap::next_bar_at_frame`.
  - `advance_to(current_frame, &mut bank)` triggers when `current_frame >= launch_frame`, activating the queued scene and appending its non-empty lane slots as `PatternInstance` clips at `launch_beat`.
- **`RetrospectiveNoteBuffer`**: A bounded ring buffer (`MAX_RETROSPECTIVE_NOTES = 512`) continuously recording live unrecorded `note_on` / `note_off` events at 48 kHz frames.
  - `recover_to_pattern` converts captured performances back into a bar-aligned `TapestryPattern` using `TempoMap::bar_bounds_at_beat`, with optional onset quantization (e.g., `Some(Time::SIXTEENTH)`).

### 1.5 Kit Starters & Song Starter Templates (`kits.rs`)
`crates/moan-core/src/kits.rs` ships 6 multi-channel **Kit Starters** (`ALL_KITS`, lookup via `find_kit`) and 6 full **Song Starter** templates (`ALL_SONG_STARTERS`, lookup via `find_song_starter`):

| Kit Starter (`ALL_KITS`) | Genre | Default BPM | Highlights |
| :--- | :--- | :---: | :--- |
| `808 Glide Kit` | Trap | 140 | Punch Kick, Snare, Clap, Closed/Open Hat (`cut_group = 1`), legato Sub 808 (`cut_group = 2`) |
| `Acoustic Studio Kit` | Rock / Indie | 112 | Modal Acoustic kick/snare/hats/crash, Picked Bass, Electric Guitar |
| `Neon Synthwave Kit` | Synthwave | 108 | Gated kick/snare/hat, Pulse Bass, Poly Pad, Cross Lead |
| `Dusty Boom-Bap Kit` | Boom-Bap | 90 | Dusty kick/snare/hat, Tine Keys, Upright Sub, Ariel Chop (`cut_group = 2`) |
| `Double-Kick Metal Kit` | Metal | 150 | Metal Kick, Crack Snare, Ride Bell, Floor Tom, Chug Guitar |
| `Club House Kit` | House | 124 | 909 Kick, Shuffle & Open Hats (`cut_group = 1`), Clap, Organ, Sub |

| Song Starter (`ALL_SONG_STARTERS`) | Genre | Tempo & Swing | Grid |
| :--- | :--- | :--- | :--- |
| `Night Slide (Trap 140)` | Trap | 140 BPM | 2 bars, `1/32` (64 steps), 1/32 hat rolls + overlapping octave 808 slides |
| `Afterglow (Synthwave 108)` | Synthwave | 108 BPM | 2 bars, `1/16` (32 steps) |
| `Dusty Crate (Boom-Bap 90, 22% Swing)` | Boom-Bap | 90 BPM, 22% swing | 2 bars, `1/16` (32 steps) |
| `Iron Pulse (Double-Kick 150, 1/16T)` | Metal | 150 BPM | 1 bar, `1/16T` (24 steps) |
| `Velvet Club (House 124)` | House | 124 BPM | 2 bars, `1/16` (32 steps) |
| `Glass Choir (Ariel & Mallet 116)` | Ambient Choral | 116 BPM | 2 bars, `1/16` (32 steps), Ariel Formant + Mallet Bells + Grand Piano |

---

## 2. Exact Musical Time & Tempo Maps

**Module**: `crates/moan-core/src/time.rs`

To eliminate floating-point tick accumulation drift across long arrangements, triplet grids, and tempo ramps, Moan models musical positions using exact reduced rationals paired with closed-form calculus over piecewise tempo segments.

### 2.1 Rational `Time`
- [`Time`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/time.rs#L76-L79) stores `{ num: i64, den: u32 }` in quarter-note beats, always reduced by GCD (`gcd_u64` / `gcd_u128`).
- `MAX_DENOMINATOR = 15_360` ($16 \times 960 = 2^{10} \times 3 \times 5$), cleanly representing `DISPLAY_PPQ = 960` ticks, binary subdivisions down to `MIN_BEAT_STEP` (`1/64` beat = 256th note), and tuplets (`1/3`, `1/6`, `1/12`, `1/5`) with zero rounding error.
- Built-in constants: `ZERO`, `ONE` / `QUARTER`, `EIGHTH`, `SIXTEENTH`, `THIRTY_SECOND`, `SIXTY_FOURTH`, `MIN_BEAT_STEP`, `TRIPLET_QUARTER` (`2/3`), `TRIPLET_EIGHTH` (`1/3`), `TRIPLET_SIXTEENTH` (`1/6`), `TRIPLET_THIRTY_SECOND` (`1/12`).
- Exact interop: `from_ticks`, `from_ticks_nearest`, `from_display_ticks`, `to_ticks_rounded`, and `to_display_ticks`.

### 2.2 Compound Time Signatures, Tempo Ramps & Global Swing
- **`TimeSignature`**: Supports arbitrary `numerator` and power-of-two `denominator` (`<= 64`), with built-in constants `FOUR_FOUR` (`4/4`), `THREE_FOUR` (`3/4`), `FIVE_FOUR` (`5/4`), `SIX_EIGHT` (`6/8`), and `SEVEN_EIGHT` (`7/8`). Exact bar duration in quarter notes is `beats_per_bar() = (numerator * 4) / denominator`.
- **`TempoMap` & Analytical Linear BPM Ramps**:
  - Stores up to `4096` sorted `TempoSegment`s (`bpm_x100` in `10.00..=999.00 BPM`, optional `ramp_to_bpm_x100`) and up to `1024` `(Time, TimeSignature)` markers.
  - For a linear tempo ramp from $B_0$ to $B_0 + k\,\Delta q$ over $\Delta q$ beats, elapsed time is integrated analytically—without discrete step approximation—via:
    $$\Delta t = \frac{60}{k}\,\ln\!\left(\frac{B_0 + k\,\Delta q}{B_0}\right)$$
    with exact analytical inverse $\Delta q = \frac{B_0}{k}\left(e^{k\,\Delta t / 60} - 1\right)$ in `segment_beats_for_seconds`, keeping `beat_to_frame` and `frame_to_beat` consistent at `RATE = 48_000`.
- **Global Swing (`0..=75%`)**: `TempoMap::swing_beat_f64` and its exact inverse `unswing_beat_f64` apply a piecewise-linear warp inside every half-beat (eighth-note pair), delaying odd 16th subdivisions ($0.25 \to 0.25 + 0.25 \cdot \text{pct}/100$) while preserving all even eighth-note boundaries and strict monotonicity.

---

## 3. Expressive & Physical-Modeled Instruments

**Modules**: `crates/moan-core/src/instruments.rs`, `crates/moan-core/src/device_presets.rs`, `crates/moan-core/src/midi.rs`

Moan extends its classic engines (`0..=4`: `SilkTriangle`, `SatinPulse`, `VelvetSampler`, `ArielFormant`, `PrismWavetable`) with 7 expressive, `#![no_std]` synthesized and physical-modeled instruments (`5..=11`) unified under `NoteExpr`, `instruments::ExpressiveNote`, and `InstrumentParams`.

### 3.1 Per-Note Expression (`NoteExpr`)
Every instrument (`5..=11`) accepts [`NoteExpr`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L27-L38) via `render_expressive` and `render_notes_with_params`:
- `pitch_bend_semitones`: `-48.0..=48.0` semitones
- `pressure`: polyphonic aftertouch (`0.0..=1.0`)
- `timbre`: per-note brightness/harmonic character (`0.0..=1.0`, `0.5` neutral)
- `slide_from_key` & `glide_frames`: explicit note-entry pitch slides triggered from Piano Roll slide notes

### 3.2 Instrument Roster (`5..=11`)

1. **`Sub 808` (`Instrument::Sub808 = 5`, [`Sub808`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L680-L709))**:
   - **Oscillator & Punch**: Sine-to-triangle morph (`shape`), phase-locked 2nd harmonic (`octave`), `0.33 ms` (16-frame) zero-crossing attack guard, and exponential downward pitch-drop punch (`punch_semitones`, `punch_ms`).
   - **Legato Pitch Glides**: Overlapping notes smoothly glide pitch when `GlideMode::Legato` or `GlideMode::Always` is active (`glide_ms`), or when `NoteExpr::slide_from_key` is set.
   - **4x Oversampled Waveshaper & Growl**: `4x` oversampled saturation with selectable `DriveCurve` (`WarmSym`, `TubeAsym`, `HardClip`, `Foldback`), 12 dB/oct SVF lowpass (`tone_hz`), transient `click`, and dual-bandpass vocal formant `growl`.
   - **Factory Presets**: `14` presets in `SUB808_PRESETS` (including *Classic Sub*, *Punchy Knock*, *Legato Slide*, *Wavefold Growl*, *Formant Growl*).

2. **`Studio Drums` (`Instrument::DrumKit = 6`, [`DrumKit`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L1205-L1215))**:
   - **3 Synthesis Models (`DrumModel`)**: `Electronic` (clean modern punch), `Circuit` (analog-inspired 808/909 topologies including `SquareBank6` metallic oscillator bank), and `ModalAcoustic` (modal shell resonances + room ambience).
   - **GM Drum Mapping (`DrumPiece`)**: `Kick` (35/36), `Rim` (37), `Snare` (38/40), `Clap` (39), `ClosedHat` (42/44), `OpenHat` (46), `Tom(0..=5)` (41/43/45/47/48/50), `Crash` (49/57), `Ride` (51), with built-in Closed-Hat -> Open-Hat choke and per-piece stereo panning.
   - **Factory Presets**: `10` presets in `DRUMKIT_PRESETS` (*Electronic Core*, *Circuit 808*, *Circuit 909*, *Acoustic Studio*, *Heavy Metal Kit*, *Dusty Vinyl Kit*, *Gated 80s Kit*, *Tight Trap Kit*, *Crisp Brush Kit*, *Industrial Crush Kit*).

3. **`Tine Keys` (`Instrument::TineKeys = 7`, [`TineKeys`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L1837-L1852))**:
   - Physical/FM electric tine piano with tine `bell` FM harmonics, hammer strike `bark`, electromagnetic `pickup_asym`, stereo autopan/tremolo (`tremolo_rate_hz`, `tremolo_depth`), and felt damper `release_ms`.

4. **`Grand Piano` (`Instrument::ModalGrand = 8`, [`ModalGrand`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L1988-L2001))**:
   - Velocity-hardened felt hammer filter (`brightness`), detuned 3-string unison (`unison_detune`), stiff-string `inharmonicity`, wooden soundboard `body_mix`, and damper release.

5. **`Electric Guitar` (`Instrument::PluckGuitar = 9`, [`PluckGuitar`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L2222-L2231))**:
   - **String Waveguide**: Delay-line plucked string with 1st-order Thiran allpass fractional-delay tuning, `PluckMode::Sustained` vs `PalmMuted`, `pick_pos` comb coloration, `brightness`, and release `fret_noise`.
   - **2x-Oversampled `GuitarAmp`**: Feeds a 2x-oversampled asymmetric tube preamp/power-amp stage, 3-band tone stack (`bass`, `mid`, `treble`), and 4x12 speaker `cabinet` resonance & rolloff.

6. **`Mallet Bells` (`Instrument::ModalMallet = 10`, [`ModalMallet`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L2460-L2470))**:
   - Modal bar/bell resonator supporting `MalletKind::Marimba` (`1 : 3.99 : 9.2`), `TubularBell` (inharmonic chime partials), and `Vibraphone` (metallic bar with motor tremolo `shimmer`), plus velocity/timbre-scaled `hardness` and `stereo_width`.

7. **`Cross Synth` (`Instrument::CrossSynth = 11`, [`CrossSynth`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/instruments.rs#L2617-L2646))**:
   - Dual-oscillator hybrid synth (`osc_b_ratio`, `osc_b_detune_cents`, `mix_ab`) combining bidirectional `cross_fm`, oscillator `hard_sync`, and `ring_mod`, feeding a resonant 12 dB/oct TPT SVF (`cutoff_hz`, `resonance`, `env_to_cutoff`), ADSR envelopes, and `stereo_spread`.

---

## 4. Piano Roll Compositional Note Tools & SMF Interchange

**Modules**: `crates/moan-core/src/compose.rs`, `crates/moan-core/src/smf.rs`

### 4.1 Expressive Notes & Visual Pitch-Slide Connectors (`compose.rs`)
- `compose::ExpressiveNote` stores rational `start` and `length` (`Time`), `key`, `velocity`, `pan`, `fine_cents`, `slide_to_key: Option<u8>`, `timbre`, and `pressure`.
- When `slide_to_key` is set on a Piano Roll note, the UI renders a visual pitch-slide connector from `key` to `slide_to_key`, and synthesis maps it into `NoteExpr::slide_from_key` / legato pitch glide.
- `notes_to_midi_pattern` and `midi_pattern_to_notes` convert losslessly between rational `ExpressiveNote`s and 48 kHz frame-baked `midi::Pattern`s via `TempoMap` and global `swing_pct`.

### 4.2 Ten Deterministic Piano Roll Transforms (`ComposeTool`)
[`apply_tool`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/compose.rs#L298-L345) (and `apply_tool_to_midi`) executes up to `MAX_COMPOSE_NOTES = 16_384` notes deterministically:

| Tool Variant | Parameters | Behavior |
| :--- | :--- | :--- |
| `Legato` | `gap: Time` | Extends/trims each note to reach the next onset minus `gap` |
| `Articulate` | `scale_pct: u16` | Scales note durations (staccato `< 100%` or tenuto `> 100%`) |
| `Chop` | `division: Time`, `gate_pct: u8` | Slices sustained notes into rhythmic subdivisions |
| `Arpeggiate` | `step`, `order` (`Up`/`Down`/`UpDown`/`Random`), `octaves`, `gate_pct`, `seed` | Expands simultaneous chords into sequenced arpeggio steps |
| `Strum` | `spread: Time`, `direction` (`Up`/`Down`/`Alternate`), `vel_falloff_pct` | Staggers chord note onsets with progressive velocity taper |
| `Flam` | `count: u8`, `spacing: Time`, `vel_pct: u8` | Prepends/inserts tight grace-note flams and rolls |
| `Humanize` | `max_timing_ticks`, `max_vel_delta`, `max_len_pct`, `seed` | Applies deterministic `splitmix64` micro-timing, velocity, and length jitter |
| `Levels` | `multiply_pct: u16`, `offset: i8` | Scales and offsets note velocities (`1..=127`) |
| `Flip` | `reverse_time: bool`, `mirror_pitch: bool` | Retrograde time reversal and/or pitch inversion around median key |
| `Tidy` | — | Clamps bounds, removes duplicates, sorts chronologically |

### 4.3 Standard MIDI File (`.mid`) Import & Export (`smf.rs`)
Zero-dependency `#![no_std]` parser and serializer with strict resource limits (`MAX_SMF_BYTES = 4 MiB`, `MAX_SMF_TRACKS = 32`, `MAX_SMF_NOTES = 4096`) that never panics on truncated or malformed files:
- **Import (`import_smf`)**: Parses `MThd` Format 0 and Format 1 files, extracting `TempoMap` (`FF 51` set-tempo meta events), `TimeSignature` (`FF 58`), track names (`FF 03`), program changes, channel 10 percussion flags (`is_percussion`), signed 14-bit pitch bends, and paired note-on/note-off spans into `SmfImport`. `SmfImport::to_tapestry_bank` materializes the tracks directly into a `TapestryBank`.
- **Export**: `export_smf_pattern`, `export_smf_midi_pattern`, and `export_smf_bank` write standards-compliant Format 1 (`DISPLAY_PPQ = 960`) `.mid` byte vectors with a dedicated conductor track (`FF 51` / `FF 58`) and per-channel note tracks honoring `TransportContext`, Mute/Solo, and `cut_group` truncated note durations.

---

## 5. Pro Console Mixing & Mastering

**Modules**: `crates/moan-core/src/parametric_eq.rs`, `crates/moan-core/src/tape_fx.rs`, `crates/moan-core/src/mastering.rs`, `crates/moan-core/src/lib.rs`, `crates/moan-core/src/device_presets.rs`

### 5.1 6-Band RBJ Parametric EQ, FFT Spectrum Analyzer & Resonance Detection (`parametric_eq.rs`)
- **`ParametricEq`**: Six cascaded RBJ biquad sections with Direct Form II Transposed `f64` state at 48 kHz:
  - Band 0: `HighPass` (`20..=1000 Hz`, Q `0.5..=4.0`, toggleable)
  - Band 1: `LowShelf` (`30..=500 Hz`, `-18..=+18 dB`, Q `0.5..=2.0`)
  - Band 2: `Bell` 1 (`40..=16000 Hz`, `-18..=+18 dB`, Q `0.3..=12.0`)
  - Band 3: `Bell` 2 (`100..=18000 Hz`, `-18..=+18 dB`, Q `0.3..=12.0`)
  - Band 4: `HighShelf` (`2000..=18000 Hz`, `-18..=+18 dB`, Q `0.5..=2.0`)
  - Band 5: `LowPass` (`1000..=20000 Hz`, Q `0.5..=4.0`, toggleable)
  - `response_curve_db` computes the exact cascaded complex magnitude response $|H(e^{j\omega})|^2$ across arbitrary frequency grids for live EQ curve rendering.
- **`SpectrumAnalyzer` & `detect_resonances`**:
  - Hann-windowed radix-2 in-place stereo-power (`L^2 + R^2`) FFT (`256` or `512` points, exponential smoothing `0.0..=0.98`, normalized so full-scale sine = `0.0 dBFS`).
  - `detect_resonances` suppresses silence (`< -55.0 dBFS`) and broadband tilt across `60..=18000 Hz` and proposes up to 3 `ResonanceCandidate` bell cuts (capped at `-6.0 dB`, Q in `4.0..=8.0`), applicable via `ParametricEq::apply_resonance_cut`.

### 5.2 Dynamics, Tape & Color Inserts (`tape_fx.rs`)
- **`SidechainCompressor`**: Soft-knee (`6 dB` knee) stereo-linked compressor (`-60..=0 dB` threshold, `1.0..=20.0` ratio, `0.1..=500 ms` attack, `5..=2000 ms` release, `-12..=+24 dB` makeup), 80 Hz 1-pole sidechain high-pass filter, optional external key input, and live positive `gain_reduction_db()`.
- **`SidechainGate`**: Noise and rhythmic ducking gate (`-80..=0 dB` threshold, `0..=24 dB` hysteresis, `0..=2000 ms` hold, `-80..-3 dB` attenuation range, 80 Hz sidechain HPF, optional external key, `is_open()`, live `gain_reduction_db()`). `5` presets in `GATE_PRESETS` (*Clean Threshold*, *Tight Drums*, *Gated Snare Reverb*, *Vocal Noise Floor*, *Rhythmic Chop*).
- **`TapeMachine`**: Pre-allocated 6 ms (`288`-frame) stereo modulated delay line with `144`-frame (`3.0 ms`) latency-aligned dry/wet compensation, asymmetric soft oxide saturation, `85 Hz` head bump (`0..=5 dB`), tone high-cut (`1.5..=20 kHz`), dual wow/flutter LFOs, and signal-envelope-gated tape hiss. `6` presets in `TAPE_PRESETS` (*Warm Reel*, *Master Glue*, *Lo-Fi Cassette*, *Worn Flutter*, *Pitch Wobble*, *Hot Oxide*).
- **`TransientShaper`**: Dual fast/slow envelope differential attack (`-100..+100%`) and sustain (`-100..+100%`) shaper.
- **`BitCrusher`**: Sample-and-hold downsampler (`1..=64`) and bit-depth quantizer (`2..=16` bits) with dry/wet mix.
- **`StereoChorus`**: Quadrature LFO-modulated cubic-interpolated delay chorus (`0.05..=10.0 Hz`, `0..=15 ms` depth, `-0.9..=0.9` feedback, dry/wet mix).

### 5.3 Console Routing, Sends, Buses & Signal Order (`lib.rs`)
[`render_console_mix`](file:///Users/xmacbookm5/Projects/Sex/.worktrees/moan-tapestry/crates/moan-core/src/lib.rs#L744-L940) processes up to `MAX_CONSOLE_TRACKS = 64` tracks in topological dependency order (source tracks -> sidechain targets / `SendRoute` post-fader aux returns / `output_bus` group buses -> `MasterBus`) with zero heap allocations:
1. Per-track insert chain (`Track::process_console_frame`):
   - Self-keyed `SidechainGate` (noise floor cleanup before saturation) -> `TransientShaper` -> `Distortion` -> `BitCrusher` -> `TapeMachine` -> `ParametricEq` -> `SidechainCompressor` (self or external `SidechainSource` key) -> `StereoChorus` -> `effects::Rack` (3-band EQ / Compressor / Reverb) -> Externally keyed `SidechainGate` (post-reverb gated reverb / rhythmic ducking).
2. Post-fader gain/balance -> `sidechain_tap` capture -> `post_fader_send` (`SendRoute`) -> `output_bus` summing -> `MasterBus` fader -> optional `MasteringProcessor` -> per-track meter tap `(track_idx, post_fader_frame, gr_db_q8)`.

### 5.4 Mastering Chain & True-Peak Lookahead Limiter (`mastering.rs`)
- **`MasteringProcessor`**:
  1. **Input Trim**: `-12.0..=+18.0 dB`
  2. **3-Band Mastering EQ**: `120 Hz` Low Shelf / `500 Hz` Body Bell / `8 kHz` Air Shelf (`-12..=+12 dB`)
  3. **Glue Bus Compressor**: Toggleable soft-knee `SidechainCompressor` (default `-12 dB`, `2.0:1`, `10 ms` attack, `120 ms` release)
  4. **Mid/Side Stereo Width**: `0.0` (mono) to `2.0` (wide)
  5. **`LookaheadLimiter`**: `4.0 ms` (`LOOKAHEAD_FRAMES = 192` at 48 kHz) circular lookahead delay buffer with 4-tap cubic 2x midpoint inter-sample peak estimation, smooth 192-frame attack ramp scheduling, configurable ceiling (`-24.0..=0.0 dB`, default `-1.0 dB`) and release (`10..=1000 ms`, default `100 ms`), and combined live `gain_reduction_db()`.
- **Factory Presets**: `8` presets in `MASTERING_PRESETS` (*Balanced Clear*, *Warm Glue*, *Punchy Bus*, *Loud Ceiling*, *Sub Heavy*, *Neon Wide*, *Dusty Vintage*, *Gentle Broadcast*).

---

## 6. Provenance & Licensing

Moan's `#![no_std]` Rust DSP, exact rational musical time architecture, and multi-channel Tapestry pattern-workstation design were informed by open-source architectural research of [`Vheissu/peel-studio`](https://github.com/Vheissu/peel-studio) (`GPL-3.0-or-later`, license-compatible with SexOS's `GPL-3.0-only`).

All implementation code in `crates/moan-core` and `apps/moan` consists of original `#![no_std]` Rust modules with zero external dependencies, strict `try_reserve` / `try_reserve_exact` OOM bounds, and Moan's own naming, presets, kits, and song starters.
