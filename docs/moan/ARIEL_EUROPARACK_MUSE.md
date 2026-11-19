# Moan Studio 2.0: Ariel Formant Synthesizer, EuropaRack Modular Patchbay & Opt-In Muse Engine

Moan Studio 2.0 upgrades SexOS's digital audio workstation into a hybrid studio combining deterministic `#![no_std]` DSP instruments, a modular Eurorack-style patchbay, and opt-in generative audio synthesis.

This document is the architectural specification and user guide covering:
1. **Dual-Mode Design**: Classic DAW zero-overhead baseline vs. Opt-In Muse workflows.
2. **Ariel Formant Synthesizer (`crates/moan-core/src/vocal.rs`, `phonemes.rs`, `pronounce.rs`)**: Parallel-formant speech and singing synthesis, full directive DSL, phoneme engines, karaoke synchronization, and Piano Roll MIDI singing.
3. **EuropaRack Modular Patchbay (`crates/moan-modular`)**: 64-frame block scheduler, 16-channel polyphonic 1V/oct CV/Gate/Audio/MIDI routing, cycle feedback stabilization, 19 built-in modules, Dark Room mode, and sub-patch selection interchange.
4. **Studio 2.0 Core DSP & Composition Tools (`crates/moan-core`)**: Prism Wavetable, multi-mode Distortion, visual Parametric EQ curves, WSOLA Warping (Transpose, Speed, On Beat), offline Extract MIDI, Musical Typing + Arpeggiator, and Track Automation.
5. **Muse Generative Engine (`crates/moan-gen`, `apps/moangen`, `scripts/start_moan_ace_bridge.sh`)**: 4-tier hardware budget, ephemeral worker execution, Muse workflows, Natural-Language Studio Chat Bar, and host bridge CLI.
6. **`MOANPRJ` v4 Compatibility & Serialization**: Preserving byte-identical v1/v2/v3 projects and security rules for persistent metadata.

---

## Status (2026-11-16)

What runs in Moan on SexOS today: Ariel, EuropaRack, the Studio 2.0 DSP tools and
**Tier 0** Muse generation (pure Rust, no model). Tiers 1 to 3 are groundwork only:
`crates/moan-gen` has the request/response codecs and tier selection, and
`scripts/start_moan_ace_bridge.sh` can fetch the open ACE-Step 1.5 weights and serve them on
a host, but Moan does not yet talk to a Muse server or run a local model, so its setup card
offers only the built-in synth. The dialog below is the target design. Moan's UI shows only
our own names ("Muse"), never a third-party product or model name.

## 1. Dual-Mode Design: Classic DAW vs. Opt-In Muse

Moan Studio 2.0 operates under a strict **Zero-Cost Baseline** policy:

```
+--------------------------------------------------------------------------+
|                            Moan Studio 2.0                               |
|                                                                          |
|  [ Play ] [ Stop ] [ Rec ] [ 120 BPM ] [ 4/4 ]           [ Muse ] (Off)  |
+--------------------------------------------------------------------------+
|  Timeline: Tracks 1..8 | Clips | Waveforms | Piano Roll | Automation      |
|  DSP: EQ, Compressor, Reverb, Distortion, Prism Wavetable, Ariel Voice   |
|  Modular: EuropaRack 64-frame block patchbay (19 modules)                |
+--------------------------------------------------------------------------+
                                     |
                          User clicks [ Muse ]
                                     |
                                     v
+--------------------------------------------------------------------------+
|                         Muse Opt-In Dialog                               |
|                                                                          |
|  Select Generative Hardware Tier:                                        |
|  (*) Tier 0: Featherweight (0 MB Model RAM, Pure Rust DSP, Always Ready) |
|  ( ) Tier 1: Remote Bridge (Connect to ace-server:8080 / acestep-api)    |
|  ( ) Tier 2: Local Turbo Lite (Download ~2.4 GB GGUF; needs 6-20 GB RAM) |
|  ( ) Tier 3: Local Studio XL (Download ~7.7 GB GGUF; needs >= 20 GB RAM) |
|                                                                          |
|  [ Download / Connect ]                            [ Cancel / Stay Off ] |
+--------------------------------------------------------------------------+
```

### 1.1 Default Startup: Pure Classic DAW (`muse_enabled = false`)
- **Zero Background RAM**: No weights, no model runtimes, no background Python or C++ servers, and zero daemon threads are spawned. The application footprint is identical to Classic Moan.
- **Zero AI Clutter**: The timeline, mixer, piano roll, and waveform editors remain focused on deterministic recording, editing, sequencing, and mixing.
- **Zero Network Egress**: Moan makes no network calls or telemetry transmissions at startup.

### 1.2 Activating Muse
Clicking the `[ Muse ]` button on the top toolbar opens the configuration modal. The user can choose:
1. **Tier 0 Featherweight Mode**: Operates immediately with `0 MB` model RAM using built-in algorithmic composition, genre blueprints, and Ariel formant voices.
2. **Tier 1 Remote Bridge**: Connects to a local or LAN `ace-server` (:8080) or `acestep-api` (:8001) endpoint. Audio is streamed via lightweight HTTP/JSON requests with zero local model RAM.
3. **Tier 2 / 3 Local Model Download**: Initiates an on-demand, resumable download of the ACE-Step 1.5 GGUF weights (`~2.4 GB` Turbo Lite or `~7.7 GB` Studio XL) to `@/data/moan/muse`. SHA-256 chunk verification ensures file integrity.

### 1.3 Returning to Classic DAW
Clicking `[ Muse ]` back to **OFF** unmaps the generative UI and shuts down any active sidecar connection:
- All generated audio clips, takes, stems, and MIDI notes **remain on the timeline**.
- All DSP mixing channels, automation curves, and plugin routings are preserved.
- Generation memory is completely freed back to SexOS.

---

## 2. Ariel Formant Synthesizer

Ariel (`crates/moan-core/src/vocal.rs`, `phonemes.rs`, `pronounce.rs`) is a native `#![no_std]` 48 kHz parallel-formant speech and vocal synthesizer inspired by Dennis Klatt's 1980 cascade/parallel formant model and `klattsch`.

### 2.1 DSP Architecture

```
                    +---------------------------+
                    | Rosenberg Glottal Pulse   |
                    | f0 + Vibrato LFO          |
                    +---------------------------+
                                  |
                                  v
+------------------+     [ Voicing Mix (V) ]
| 32-bit xorshift  |              |
| LFSR White Noise | ---> [ Aspiration (h) ]
+------------------+              |
         |                        +-----------------------+
         |                                                |
         v                                                v
[ Tremolo LFO (m, n) ]                         [ Spectral Tilt (t) ]
         |                                                |
         +------------------------+-----------------------+
                                  |
               +------------------+------------------+
               |                  |                  |
               v                  v                  v
        +--------------+   +--------------+   +--------------+
        | Formant BP 1 |   | Formant BP 2 |   | Formant BP 3 |
        | F1 / BW1 / A1|   | F2 / BW2 / A2|   | F3 / BW3 / A3|
        +--------------+   +--------------+   +--------------+
               |                  |                  |
               +------------------+------------------+
                                  |
                                  v
                       [ Soft-Clipped Output ]
                       (Linear to ±0.85, smooth knee)
```

1. **Excitation Sources**:
   - **Voiced Excitation**: Derivative of the Rosenberg glottal pulse (`glottal_pulse(phase, effort)`). The `effort` parameter (0.0..=1.0) controls the open/closed quotient of vocal fold closure.
   - **Unvoiced / Noise Excitation**: 32-bit `xorshift` LFSR producing white noise for fricatives, plosives, and whisper aspiration.
   - **Spectral Tilt (`tilt`)**: First-order one-pole filter shaping high-frequency roll-off (-0.9..=0.9).
   - **Vibrato & Tremolo**: Sine LFOs modulating pitch (0..=15 Hz deviation) and amplitude (0..=1.0 depth).
2. **Formant Resonators**:
   - Three constant-skirt-gain bandpass biquads (`BandpassBiquad`) running in parallel at 48 kHz.
   - Resonator coefficients are derived from Robert Bristow-Johnson's Audio EQ Cookbook:
     $$\alpha = \frac{\sin(\omega_0)}{2 Q}, \quad Q = \frac{f_0}{BW}, \quad b_0 = \frac{\alpha}{1+\alpha}, \quad b_1 = 0, \quad b_2 = -b_0$$
   - Each formant has an independent center frequency ($F_n$), 3 dB bandwidth ($BW_n$), and linear amplitude gain ($A_n$).
3. **Formant Glides & Stops**:
   - Diphthongs glide formants smoothly from onset to target over the phoneme duration.
   - Plosive stops (`B`, `D`, `G`, `P`, `T`, `K`) insert a brief silent occlusion phase followed by a burst transient.

### 2.2 Directive Reference Table

Ariel scripts support an expressive token-based DSL. Directives can be embedded inline with phonemes or words:

| Directive | Syntax | Parameter Range | Description | Example |
|---|---|---|---|---|
| **Base Pitch** | `b<pitch>` | Note name or Hz | Sets the baseline vocal pitch $F_0$. Bare `b` resets to default (120 Hz). | `bC#4`, `b140`, `b+12`, `b-5`, `b` |
| **Phoneme Rate** | `r<ms>` | 20..=500 ms | Sets the nominal duration per phoneme slot. Bare `r` resets to default (110 ms). | `r80`, `r140`, `r` |
| **Formant Scale** | `s<scale>` | 0.70..=1.50 | Scales all formant frequencies ($F_1, F_2, F_3$) to model vocal tract length (gender/character). | `s1.18` (feminine/child), `s0.85` (deep masculine) |
| **Vibrato Depth** | `v<hz>` | 0.0..=15.0 Hz | Sets peak pitch deviation of vibrato. Bare `v` disables vibrato. | `v4.5`, `v` |
| **Vibrato Rate** | `w<hz>` | 1.0..=12.0 Hz | Sets frequency of vibrato LFO. Bare `w` resets to 5.0 Hz. | `w5.5`, `w` |
| **Tremolo Depth** | `m<depth>` | 0.0..=1.0 | Sets amplitude modulation depth. Bare `m` disables tremolo. | `m0.25`, `m` |
| **Tremolo Rate** | `n<hz>` | 1.0..=15.0 Hz | Sets frequency of tremolo LFO. Bare `n` resets to 5.0 Hz. | `n6.0`, `n` |
| **Aspiration** | `h<amt>` | 0.0..=1.0 | Injects breathy aspiration noise into voiced vowels. | `h0.3` (whisper / breathy vocal) |
| **Spectral Tilt** | `t<tilt>` | -0.9..=0.9 | Brightens (>0) or dampens (<0) high harmonic energy. | `t-0.3` (warm vintage), `t0.4` (piercing) |
| **Effort** | `g<effort>` | 0.0..=1.0 | Controls glottal pulse tension and vocal effort. | `g0.8` (belted / shouted), `g0.3` (soft) |
| **Stress Marker** | `!` or `'` | Suffix | Accents the phoneme: +25% duration, +15 Hz pitch rise, +30% amplitude. | `AA!`, `IY'` |
| **Sticky Pitch Delta** | `+N` or `-N` | Suffix (Hz) | Shifts pitch up or down for this and subsequent phonemes until reset. | `AY+15`, `OW-10` |
| **Transient Ornament** | `(+N)` or `(-N)` | Suffix (Hz) | Applies a transient pitch bend during this phoneme, returning to base pitch. | `AE(+40)`, `UW(-30)` |
| **Syllable Group** | `( ... )` | Delimiter | Groups phonemes into a single syllable; automatically shares duration. | `( B EY B IY )` |
| **Phoneme Bank** | `[bank=NAME]` | `en`, `ja` | Switches active phoneme pronunciation bank. | `[bank=ja]`, `[bank=en]` |
| **Voice Switch** | `[voice=N]` | `0..=7` | Directs following directives/phonemes to voice lane $N$ for polyphonic singing. | `[voice=0]`, `[voice=1]` |
| **Pauses** | `,` / `;` / `.` | Inline tokens | Inserts pauses: `,` = 100 ms, `;` = 200 ms, `.` = 300 ms, `p<ms>` = custom. | `p150` |
| **Comments** | `#` or `/* */` | Inline | Comment syntax ignored by compiler. | `# verse 1`, `/* bridge */` |

### 2.3 Pronunciation Engines: English & Japanese

1. **English Text-to-ARPABET (`pronounce.rs`)**:
   - Converts natural English words into standard Hillenbrand/Klatt ARPABET phonemes.
   - Employs a binary-searched music lyric table (`COMMON_MUSIC_EXCEPTIONS`) covering 400+ words frequently found in song lyrics (`alive`, `breathe`, `cyber`, `rhythm`, `tonight`, etc.).
   - Fallback grapheme-to-phoneme rule engine handles arbitrary English text.
2. **Japanese Romaji & Kana (`pronounce.rs`)**:
   - Activated via `[bank=ja]`.
   - Parses Romaji (Hepburn) syllables and UTF-8 Hiragana/Katakana directly into Mokhtari 2000 Japanese formant targets (`a`, `i`, `u`, `e`, `o`, `ka`, `shi`, `tsu`, etc.).
   - Sokuon small *tsu* (`っ` / double consonants like `tte`) inserts silent gemination stops.
3. **Canned Expressive Presets (`phonemes.rs`)**:
   - Built-in presets ready for instant playback: `choir_chords`, `robot_funk`, `acid_vox`, `countdown`, `tokyo_neon`, `sci_fi_alert`, `cyber_diva`, `retro_speech`, `lunar_echo`, `daft_anthem`.

### 2.4 Karaoke Sync & Piano Roll MIDI Singing

- **Karaoke Synchronization**:
  `vocal::synthesize(input, params)` returns `(Arc<Vec<Frame>>, Vec<PhraseSpan>)`. Each `PhraseSpan` provides:
  ```rust
  pub struct PhraseSpan {
      pub byte_start: usize,  // Offset in source lyrics text
      pub byte_end: usize,
      pub start_frame: usize, // Timeline frame at 48 kHz
      pub end_frame: usize,
  }
  ```
  During playback, Moan uses `PhraseSpan` to highlight words and syllables in the lyrics bar synchronously with the timeline playhead.
- **Piano Roll MIDI Singing**:
  Calling `vocal::sing_midi_pattern(pattern, lyrics, params)` distributes syllables from the lyrics text across note events in a Moan MIDI pattern:
  - Syllable pitch is driven by `note.key` ($F_0 = 440 \cdot 2^{(\text{key}-69)/12}$).
  - Syllable duration is matched to `note.frames`.
  - Velocity scales vocal intensity and effort.
  - Also selectable in Moan tracks as `Instrument::ArielFormant` (`Instrument = 3`).

---

## 3. EuropaRack Modular Patchbay

EuropaRack (`crates/moan-modular`) brings a `#![no_std]` 48 kHz Eurorack modular synthesis and multigraph processing environment directly into Moan.

### 3.1 64-Frame Block Scheduler & Cycle Stabilization

```
Patch Graph (Nodes & Cables)
         |
         v
3-Color Depth-First Search (DFS)
  - Color 0 (White): Unvisited
  - Color 1 (Gray):  Currently visiting in stack (Active path)
  - Color 2 (Black): Visited & verified
         |
Back-edges detected (Gray -> Gray or self-patch)
         |
         +----------------------------------+
         |                                  |
         v (DAG Forward Edges)              v (Feedback Back-Edges)
Kahn's Topological Sort             Insert 1-Block Delayed Buffer (FeedbackDelay)
Produces strict causal execution    64-sample history buffer from previous block
order across all modules            guarantees numerical stability & zero latency stalls
```

- **64-Frame Block Processing (`BLOCK_FRAMES = 64`)**: The audio engine processes audio and CV in compact 64-frame blocks (1.33 ms at 48 kHz), achieving low CPU cache pressure and bounded buffer overhead.
- **Cycle Detection**: Topological sorting handles directed acyclic connections. Any cyclic cable (e.g. VCO self-FM, filter resonance feedback loops) is detected by 3-color DFS and designated as a `FeedbackDelay`.
- **Feedback Stability**: Feedback buffers retain the previous block's output, soft-clipping any explosive feedback ($|V| \le 12.0\,\text{V}$) and substituting `0.0` on NaNs to guarantee audio engine stability.

### 3.2 Signal Types & Color-Coded Patch Cables

EuropaRack provides 16-channel polyphony on every port. Signals automatically normalize monophonic cables across polyphonic voice channels:

| Signal Type | PortKind | Standard Voltage / Scale | Default Cable Color | Wire Representation |
|---|---|---|---|---|
| **Audio** | `PortKind::Audio` | $\pm 5.0\,\text{V}$ (Eurorack) / $\pm 1.0$ (DAW) | **CrimsonAudio** | Stereo `[[f32; 2]; 64]` |
| **Pitch CV** | `PortKind::PitchCv` | $1\,\text{V}/\text{oct}$ ($0.0\,\text{V} = \text{C4} = 261.63\,\text{Hz}$) | **GoldPitchCv** | Polyphonic `[[f32; 64]; 16]` |
| **Modulation CV** | `PortKind::ModCv` | $\pm 10.0\,\text{V}$ bipolar modulation | **CyanModCv** | Polyphonic `[[f32; 64]; 16]` |
| **Gate / Trigger** | `PortKind::Gate` | $0.0\,\text{V}$ (Low) / $10.0\,\text{V}$ (High) | **EmeraldGate** | Polyphonic `[[f32; 64]; 16]` |
| **MIDI** | `PortKind::Midi` | Bounded MIDI note on/off events | **VioletMidi** | Buffer (up to 32 events/block) |

### 3.3 Built-in 19-Module Catalog

EuropaRack includes 19 built-in modules in its catalog (`crates/moan-modular/src/catalog.rs`):

| # | Module Name | Kind | Category | HP | In/Out | Description & Primary Parameters |
|---|---|---|---|---|---|---|
| 1 | **Track In** | `TrackIn` | AudioIo | 4 | 0 / 1 | Injects stereo PCM audio from Moan timeline tracks into EuropaRack. |
| 2 | **Master Out** | `MasterOut` | AudioIo | 6 | 1 / 0 | Stereo master output to Moan mix bus with soft-clip limiter and peak meter. |
| 3 | **Poly MIDI-to-CV** | `PolyMidiToCv` | Midi | 8 | 1 / 4 | 1..16 voice polyphonic allocator: outputs Pitch CV, Gate, Velocity, Retrigger. |
| 4 | **Analog VCO** | `AnalogVco` | Oscillator | 12 | 4 / 4 | PolyBLEP anti-aliased Saw, Pulse with PWM, Triangle, Sine, Lin FM, Hard Sync. |
| 5 | **4-Op FM Synth** | `Fm4OpMacro` | Oscillator | 14 | 4 / 1 | 4-operator phase-modulation synth voice with ratio, modulation index, feedback. |
| 6 | **Ariel Formant Vocal** | `ArielFormantCv` | Oscillator | 16 | 6 / 2 | Klattsch formant vocal module: Vowel X/Y morph, Effort, Breath, ARPABET CV stepping. |
| 7 | **Moog Ladder VCF** | `LadderVcf` | Filter | 10 | 3 / 1 | 4-pole 24 dB/oct resonant lowpass ladder filter with drive and attenuverters. |
| 8 | **State Variable VCF** | `StateVariableVcf` | Filter | 10 | 3 / 4 | 12 dB/oct multi-mode filter with simultaneous LP, BP, HP, and Notch outputs. |
| 9 | **Poly VCA** | `PolyVca` | Amplifier | 6 | 2 / 1 | 16-channel linear/exponential voltage controlled amplifier with bias control. |
| 10 | **Buchla Wavefolder** | `Wavefolder` | Amplifier | 8 | 3 / 1 | West-coast multi-stage diode-clipping wavefolder with symmetry bias CV. |
| 11 | **Poly ADSR** | `PolyAdsr` | Modulator | 8 | 2 / 1 | 16-channel Attack/Decay/Sustain/Release envelope generator with retrigger. |
| 12 | **Tempo LFO** | `TempoLfo` | Modulator | 8 | 2 / 5 | BPM-synced or free-running Sine, Triangle, Saw, Square, and S&H LFO outputs. |
| 13 | **Clock Divider / Mult**| `ClockDivider` | Sequencer | 8 | 2 / 6 | Master BPM clock generator with /2, /4, /8, *2, *4 gate outputs and swing. |
| 14 | **Euclidean Sequencer** | `EuclideanSeq` | Sequencer | 12 | 4 / 2 | Bjorklund Euclidean rhythm generator with 16-step pitch CV sequencer. |
| 15 | **Turing Quantizer** | `TuringQuantizer` | Sequencer | 10 | 2 / 3 | Generative shift-register melody generator with Lock/Prob and scale quantizer. |
| 16 | **S&H + Slew Limiter** | `SampleAndHoldSlew`| Utility | 8 | 4 / 3 | White/pink noise source, triggered Sample & Hold, and dual rise/fall slew. |
| 17 | **4x4 Matrix Mixer** | `MatrixMixer` | Utility | 12 | 4 / 4 | 4-input, 4-output attenuverter CV and audio matrix mixer & router. |
| 18 | **Stereo Ping-Pong Delay**| `StereoDelay` | Effect | 10 | 3 / 1 | Tempo-synced stereo ping-pong delay with feedback CV and soft-limiting. |
| 19 | **Bidule Plugin Host** | `BidulePluginHost` | Host | 14 | 9 / 1 | Plogue Bidule / VCV Host slot storing plugin path, format (CLAP/VST3), 8 CV params. |

### 3.4 Dark Room Mode & Patch Interchange

- **Dark Room Mode**: `RackTheme` controls interface luminance (`dark_room_brightness: 0..=100`). At `0`, the interface simulates a darkened studio booth with illuminated glowing LED ports, module backlight meters, and neon cables.
- **Binary Patch Interchange (`serialize.rs`)**:
  - Full patch save/load using magic `EURORACK` (version 1) encodes all nodes, coordinates, parameters, cables, and themes.
  - Sub-patch selection export/import using magic `EUROSELC` allows selecting a group of modules and cables, copying to the clipboard, and sharing modular channel strips between projects.

---

## 4. Studio 2.0 Core DSP & Composition Tools

### 4.1 Prism Wavetable Synthesizer (`wavetable.rs`)
- **Dual Morphing Wavetables**: Two oscillators (Osc A and Osc B) with morph positions `osc_a_pos` (0..255) and `osc_b_pos` (0..255) blending across 3 wavetable banks:
  1. *Basic Analog*: Sine $\rightarrow$ Triangle $\rightarrow$ Saw $\rightarrow$ Square.
  2. *Harmonic / Formant*: Rich vocal formant tables and acoustic spectra.
  3. *Digital / FM*: High-order metallic and phase-modulated waveforms.
- **Analytical Keyframe Synthesis**: Keyframe interpolation computes wave shapes analytically, eliminating large wave-buffer RAM footprints.
- **Phase Warp**: Applies asymmetric phase-bend distortion to add aggressive harmonics.
- **Visual Waveform Preview**: `preview_curve(params, points)` generates a 256-point preview buffer for real-time rendering in Weft UI.

### 4.2 Multi-Mode Studio Distortion (`effects.rs`)
Supports four distinct saturation algorithms:
1. **Symmetric Soft-Clip**: Classic tape/tube saturation ($y = \frac{x}{1 + |x|}$).
2. **Asymmetric Soft-Clip**: Triode-style tube saturation with even-order harmonics:
   $$y = \begin{cases} \frac{x}{1 + x/2} & x \ge 0 \\ \frac{x}{1 + 2|x|} & x < 0 \end{cases}$$
3. **Hard Clip**: Transistor overload clamped at $\pm 24000$.
4. **Foldback**: Multi-stage diode wavefolding creating complex overtones for basslines and industrial leads.
- **Controls**: Pre-drive (0..400%), Tilt (-100..100), Bias (-100..100), Post-Cutoff Filter (200..20,000 Hz), and Dry/Wet Mix (0..100%).

### 4.3 Visual Parametric EQ Curves (`effects.rs`)
- `Equalizer::response_curve(out_db_q8)` evaluates complex transfer functions across log-spaced frequencies from 20 Hz to 20 kHz.
- Output is rendered as fixed-point Q8 dB samples ($256 = 1.0\,\text{dB}$), enabling interactive graphical EQ curves in the channel strip with zero floating-point overhead during audio playback.

### 4.4 Clip Settings & WSOLA Warping (`warp.rs`)
- **Non-Destructive Warping**: Clips store a `WarpSpec`:
  - `transpose_semitones`: -24..=24 semitones pitch shift.
  - `speed_percent`: 50..=200% playback speed.
  - `on_beat_bpm`: Optional `(source_bpm, target_bpm)` automatic tempo match.
- **WSOLA Engine (`wsola_stretch`)**: Windowed Overlap-Add pitch-shifting and time-stretching with cross-correlation lag alignment prevents phase cancellation and comb filtering.

### 4.5 Offline Extract MIDI (`audio_to_midi.rs`)
- **0 MB AI Footprint**: Pure DSP pitch and onset detector running entirely in `#![no_std]`.
- **Normalized Autocorrelation / YIN F0**: Analyzes 512-frame blocks with 256-frame hop sizes across a 50 Hz to 1000 Hz search window. Subharmonic peak suppression avoids octave errors.
- **Energy Onset Segmentation**: RMS energy slope detection places discrete MIDI notes with accurate velocities into a new MIDI clip.

### 4.6 Musical Typing & Arpeggiator (`arp.rs`)
- **Musical Typing**: Maps standard QWERTY keys (`A`, `W`, `S`, `E`, `D`, `F`, `T`, `G`, `Y`, `H`, `U`, `J`, `K`, `O`, `L`, `P`, `;`) across chromatic keys, with `Z` / `X` for octave shifting.
- **Scale Quantization**: Quantizes notes to 7 diatonic scales: *Chromatic*, *Major*, *Natural Minor*, *Dorian*, *Mixolydian*, *Pentatonic Minor*, and *Blues*.
- **48 kHz Arpeggiator Engine**: 6 modes (*Up*, *Down*, *UpDown*, *Random*, *Chord*, *AsPlayed*) synced to project BPM across 1..=4 octaves with adjustable gate lengths.

### 4.7 Track Automation (`automation.rs`)
- Bounded piecewise-linear automation lanes (`AutomationLane`, max 256 points) for Track Volume, Pan, and Filter Cutoff.
- Zero-allocation binary search and linear interpolation during 48 kHz mixing.

---

## 5. Muse Generative Engine (ACE-Step 1.5)

Moan Studio 2.0 integrates ACE-Step 1.5 as its opt-in generative music engine (`crates/moan-gen`, `apps/moangen`).

### 5.1 4-Tier Hardware RAM Budget Table

| Tier | Tier Name | RAM Requirement | Models / Architecture | Peak RAM Footprint | Performance & Behavior |
|---|---|---|---|---|---|
| **Tier 0** | **Featherweight** | $< 4096\,\text{MB}$ or offline | Pure Rust deterministic blueprints & Ariel formant voices | **$< 2\,\text{MiB}$** | Instant rendering; zero model files; 100% offline; runs on low-spec hardware. |
| **Tier 1** | **Remote Bridge** | Any RAM (with network) | Remote `ace-server` (:8080) or `acestep-api` (:8001) | **$\sim 8\,\text{MB}$** (audio buffer) | Offloads computation to external workstation / GPU cluster; 0 MB local model RAM. |
| **Tier 2** | **Local Turbo Lite** | $6144..20480\,\text{MB}$ | `TurboLiteGguf` (~2.4 GB GGUF: Qwen3-0.6B + acestep-v15-turbo-Q4_K_M + VAE) | **$\sim 3.5\,\text{GB}$** (ephemeral) | One-shot local synthesis; fast generation on laptops and mid-tier systems. |
| **Tier 3** | **Local Studio XL** | $\ge 20480\,\text{MB}$ | `StudioXlGguf` (~7.7 GB GGUF: adds acestep-5Hz-lm-4B-Q8_0 + XL turbo) | **$\sim 12\,\text{GB}$** (ephemeral) | Maximum audio fidelity; full multi-track conditioning on high-end workstations. |

### 5.2 Ephemeral Worker Architecture

To ensure the DAW never competes with generative models for RAM during playback and mixing:

```
[ Moan DAW Main Process ]
         |
         | 1. Dispatches Job Request with 128-bit Cryptographic Nonce
         v
[ Spawn: apps/moangen (Ephemeral Worker Process) ]
         |
         | 2. Maps model weights or connects to bridge
         | 3. Synthesizes audio buffer (Create, AddLayer, Stems, etc.)
         | 4. Writes audio PCM and stem metadata
         | 5. Calls `task_finish(0)` and exits immediately
         v
[ Process Terminated: 100% Generation RAM Reclaimed by SexOS ]
         |
         v
[ Moan DAW Auditions Audio & Seamlessly Places Takes on Timeline ]
```

### 5.3 Muse Generative Workflows

Moan Studio implements the full suite of Muse generative paradigms (`crates/moan-gen/src/job.rs`):

1. **Create Song (`MuseMode::CreateSong`)**:
   Generates a full composition from a text prompt and optional lyrics, specifying BPM, key/scale, time signature, duration, batch count, and optional Chain-of-Thought reasoning (`thinking_cot`).
2. **Add Layer (`MuseMode::AddLayer`)**:
   Context-aware overdubbing conditioned on Moan's rendered timeline mix window. Generates a new companion layer for a specific stem role: `Vocals`, `Drums`, `Bass`, `Chords`, `Fx`, or `Other`.
3. **Replace Section (`MuseMode::ReplaceSection`)**:
   Timeline inpainting between `from_frame` and `to_frame`. Generates replacement audio and applies smooth equal-power cosine crossfades at the boundaries to eliminate clicks.
4. **Extend (`MuseMode::Extend`)**:
   Extends a project timeline seamlessly past `from_frame` for `extra_frames`, maintaining musical key, tempo, and lyrical continuity.
5. **Cover / "Mouth Beat" (`MuseMode::Cover`)**:
   Takes a guide audio track (e.g. hummed melody, mouth beatbox, or acoustic demo) and transforms it into a fully produced instrumental or stylized arrangement governed by `style_strength_pct`.
6. **Vocal2BGM (`MuseMode::Vocal2Bgm`)**:
   Generates a full backing arrangement built around an existing dry vocal track or Ariel formant vocal performance.
7. **Extract Stems (`MuseMode::ExtractStems`)**:
   Splits a stereo mixdown into discrete, time-aligned stem tracks: *Vocals*, *Drums*, *Bass*, *Chords*, *FX*, and *Other*.
8. **MIDI-as-Prompt (`MuseMode::MidiToAudio`)**:
   Takes a Moan MIDI clip as melodic and harmonic conditioning, synthesizing studio-quality acoustic or electronic audio following the exact note events.
9. **Understand Audio (`MuseMode::UnderstandAudio`)**:
   Analyzes an imported audio clip to detect BPM, key signature, descriptive caption, and timed LRC lyrics.

### 5.4 Natural-Language Studio Chat Bar (`chat.rs`)

The Studio Chat Bar parses natural language instructions and slash commands into DAW actions:

```
[ Studio Chat: "generate 4 bars tech house bassline at 128 bpm"                 ] [ Send ]
```

- **Tempo Changes**: `/tempo 124`, `set tempo to 128 bpm`, `140 bpm` $\rightarrow$ sets project tempo.
- **Stem Separation**: `/stems`, `extract stems`, `split into stems` $\rightarrow$ triggers `ExtractStemsFromSelected`.
- **Audio to MIDI**: `/midi`, `audio to midi`, `transcribe melody` $\rightarrow$ triggers offline `ExtractMidiFromSelected`.
- **MIDI Clip Generation**: `generate midi 4 bars funky bass`, `create midi chords` $\rightarrow$ generates patterned MIDI clips using blueprint catalog.
- **Device Preset Design**: `preset eq warm tube`, `design preset punchy compressor` $\rightarrow$ parses natural language into DSP parameters via `studio_presets.rs`.
- **Context-Sensitive Edits**: With an audio region selected, typing `replace with guitar solo` or `inpaint section` automatically routes to `ReplaceSection`.

### 5.5 Host Bridge CLI Reference (`scripts/start_moan_ace_bridge.sh`)

The host bridge script manages system detection, model downloads, and bridge daemon execution:

```bash
# 1. Inspect hardware specs, GPU backend, model directory, and recommended tier
./scripts/start_moan_ace_bridge.sh --status
./scripts/start_moan_ace_bridge.sh --status --json

# 2. Download model packs on-demand with resume support and SHA-256 verification
./scripts/start_moan_ace_bridge.sh --download lite --dir /path/to/models
./scripts/start_moan_ace_bridge.sh --download studio

# 3. Launch the bridge server (auto-detects C++ ace-server, Python acestep-api, or mock)
./scripts/start_moan_ace_bridge.sh --serve --port 8080 --mode auto
./scripts/start_moan_ace_bridge.sh --serve --port 8001 --mode python
```

---

## 6. `MOANPRJ` v4 Compatibility & Serialization

Moan Studio 2.0 maintains 100% backward compatibility with all historical Moan project files while introducing the version 4 format for new features.

### 6.1 Format Version Matrix

| Version | Magic | Key Additions | Compatibility Guarantee |
|---|---|---|---|
| **v1** | `MOANPRJ\0` | Stereo PCM audio sources, tracks, clips, basic EQ/Comp/Reverb | **Read & Write**: Byte-identical backwards compatibility. |
| **v2** | `MOANPRJ\0` | MIDI patterns, instruments (Silk Keys, Satin Bass, Velvet Kit) | **Read & Write**: Compatible with Moan 1.1+. |
| **v3** | `MOANPRJ\0` | Master volume and mix bus settings | **Read & Write**: Compatible with Moan 1.2+. |
| **v4** | `MOANPRJ\0` | Ariel Vocal tracks, Distortion params, Automation lanes, EuropaRack patches, Muse metadata | **Read**: Always loads v1..v4. **Write**: Saves v4 only when Studio 2.0 features are present. |

### 6.2 Serialization Layout & Boundaries

- **Header (64 bytes, little-endian)**:
  - Magic `MOANPRJ\0` (8 bytes).
  - Format Version `u16` (`1`, `2`, `3`, or `4`).
  - Flags `u16` (reserved 0).
  - Sample Rate `u32` (always `48000`).
  - Total File Length `u64`.
  - FNV-1a Corruption Checksum `u64` (computed across all bytes following the checksum field).
  - Sources count `u16`, Tracks count `u16`, Selected track `u16`.
  - Playhead frame `u64`, Selection start frame `u64`, Selection end frame `u64`.
- **v4 Sections**:
  - **Vocal Track Data**: Text lyrics, voice parameters, and timing spans.
  - **Distortion Parameters**: Drive, tilt, bias, mix, cutoff, and distortion mode.
  - **Automation Lanes**: Frame and value pairs for track volume, pan, and cutoff.
  - **EuropaRack Patch Blob**: Embedded `EURORACK` binary payload.
  - **Muse Metadata**: Prompt text, generated stem roles, seed, and model tier info.
- **Strict Security Rules**:
  - **NO SECRETS ON DISK**: Remote bridge tokens, API keys, passwords, and host credentials are **never serialized** into `.moan` files. Secrets are held strictly in memory or environment variables.
  - **Atomic Save**: Projects save using exclusive file creation and sync before replacing previous baselines; partial writes never invalidate existing sessions.
