# Moan workstation: workflow parity and first slice

Reviewed 2026-11-15. ChatGPT round 192. Baseline: integration `c84dc5f8`; this candidate remains
unmerged until its exact-head checks pass. A reference product having a feature
is not evidence that Moan has it. This is a workflow matrix, not a claim of full
Logic, Ableton, Audacity or SoundCraft parity.

## Primary references and source boundary

- [Audacity 4](https://www.audacityteam.org/audacity-4/): region-edge editing,
  track metering, timeline annotations/export, repeat-range controls, fine waveform
  correction and customizable workspaces are useful editing targets.
- [Apple Logic Pro guide](https://support.apple.com/guide/logicpro/welcome/mac):
  the current guide selects Logic 13.0. Its arrangement/inspector/mixer, Smart
  Controls, instrument/loop browser, MIDI editing, take selection and automation
  inform the workspace.
- [Released Live 12 concepts](https://www.ableton.com/en/live-manual/12/live-concepts/):
  Session/Arrangement, audio and MIDI tracks, devices and automation are the
  released comparison baseline. The generic compare-editions URL now labels its
  table Live 13, so it is not used as a Live 12 version pin.
- [Live 13 announcement/FAQ](https://www.ableton.com/en/live/): early-2027 release
  and public-beta access are explicitly stated. Its new sequencing, captured
  automation and contextual browsing are announced targets, not shipped-baseline
  evidence and not Moan implementations.
- [ArtCraft SoundCraft](https://getartcraft.com/apps/soundcraft), not Soundcraft
  consoles or Sound Forge. [Official Rust source](https://github.com/storytold/soundcraft)
  inspected at `2688ccac8859c45cc0830fb193c5979952507b16`: workspace version
  0.20.1, `MIT OR Apache-2.0`, with both license files present. Relevant actual
  source includes `crates/mix/src/midi.rs`, `crates/ui-egui/src/panels/midi_editor.rs`
  and `crates/{clap-host,vst3-host}`. The plugin crates explicitly load native
  plugins in-process and document that a plugin crash can kill that process.
  That adapter cannot provide SexOS domain isolation. No code, branding, audio or
  artwork is copied into this slice.

## Existing Moan versus the candidate

`Implemented` below means the identified baseline has working code and existing
runtime evidence; `partial` states the actual limit. Candidate additions require
this slice's own audible-signal, edit/persistence and screenshot gate before they
are recorded as runtime-qualified.

| Workflow | Existing Moan at c84dc5f8 | First workstation candidate / remaining gap |
|---|---|---|
| Arrange several audio sources | Implemented: eight tracks, shared immutable clips, timeline selection/move, split/cut/fades, undo | Adds colored MIDI regions, fixed transport, aligned real track headers, optional library/inspector and docked Mix/Piano Roll workspaces; musical data/selection/undo are shared |
| Record a microphone and keep editable audio | Implemented: bounded Bell capture ring, explicit consent, lost-frame silence, Stop/Close lifecycle | Serenade meters now consume actual capture samples; multi-take comping, punch and monitoring remain missing |
| Mix track gain, stereo balance, mute/solo and basic effects | Implemented: per-track controls, EQ/compressor/reverb | Candidate adds separate horizontal strip level/balance controls and real post-effects/post-fader Peak/RMS via serenade-core; buses, aux/sends, VCAs, sidechains and delay compensation missing |
| Compose MIDI notes | Missing in baseline | Candidate: bounded piano grid, note create/remove, duration/velocity, quantize/transpose, source-preserving split/trim and undo; external MIDI input/output, SMF import/export, MPE and notation remain missing |
| Play instruments and browse musical material | Only a diagnostic tone in baseline | Candidate: original Silk Keys and Satin Bass synth patches plus Velvet Kit PCM sampler and editable two-bar patterns; user sample mapping, large searchable library and complex patch design remain missing |
| Keep editable projects across restart | Implemented audio project v1, exclusive create/sync, dirty-close guard | Candidate v2 persists exact note/instrument metadata and rejects false PCM caches; audio-only encoding remains exact v1 and v1 decoding stays supported |
| Deliver an audio mix | Implemented real FFmpeg WAV/FLAC/Shine MP3, partial-file cancellation and sync | Existing pipeline renders the same MIDI cache as live playback; label/range/stem/batch exports remain missing |
| Repeat/annotate an arrangement | Time selection exists, not a loop transport or markers | Missing: dedicated repeat range, markers/regions, named memory locations and label exports |
| Stretch or repair audio directly | Partial: non-destructive cut/split/fade/drag | Missing: trim handles, tempo-aware stretch/warp, direct sample drawing, spectral repair and comping playlists |
| Automate mix/device changes | Missing | Missing: automation lanes and Read/Write/Touch/Latch/Trim, parameter envelopes, capture gestures and routing-aware modulation |
| Perform independent clips | Missing | Live 12-style Session launch/scene workflow is separate from this arrangement slice; Live 13 step/return-envelope additions remain announced references |
| Host third-party instruments/effects safely | Missing | Parallel owned work: offline external-plugin jobs in separate domains with authenticated bounded results; no live insert, arbitrary desktop-binary compatibility or crash-containment claim from a fixture alone |

## Chosen first-slice engine guarantees

- MIDI notes use 48 kHz frame offsets, keys 0–127 and velocity 1–127. A source is
  at most 60 seconds, 128 notes and 32 simultaneous voices; the session remains
  eight tracks and bounded regions/project bytes. Invalid ranges/overlaps fail
  before cache allocation.
- Synth waveforms, sampler PCM and patterns are authored from source here.
  The sampler reads its original PCM at pitched rates; it is not relabelled
  synthesis. External sample loading is a future workflow, not implied support.
- PCM cache creation is an edit/load operation. The audio block renderer performs
  no file I/O, allocation or cache rebuild. It taps actual processed track samples
  once into preallocated history; Serenade meters consume only frames Bell reports played, rather than queued future audio or a second effects pass.
- MIDI undo archives compact note/patch and clip-edge metadata, never rendered
  MIDI PCM. Shared source identity is restored once outside transport. If cache
  restoration returns a fallible error, the current project and history entry
  remain intact. Small standard-library allocations are infallible and can still
  abort the app on exhaustion; no universal OOM recovery is claimed.
- The cache owns `Arc<Vec<Frame>>`, avoiding a full extra slice-Arc copy. For a
  60-second edit, temporary mix/PCM storage is about 23.04/11.52 MB; eight live
  sources plus a full undo materialization and one bake are conservatively about
  219 MB before UI/effects overhead. Moan sets an explicit 256 MiB heap ceiling
  before its first allocation; physical memory/fragmentation may still reject a
  fallible operation. The bound is not a promise every maximum-size file fits.
- Host regression measures live heap after twelve edits of a 16-second source:
  history growth must stay below 128 KiB, rather than retain twelve audio buffers.
  Audio-only codec compatibility and MIDI cache/metadata corruption are tested.
- The existing file picker, exclusive creation, synchronization, partial-failure
  handling and dirty Close/Discard paths remain the application boundary.

## Acceptance and next ordered workflows

The first slice is not delivered by this document alone. Its own QEMU gate must
edit notes through the real Weft window, capture and independently measure an
actual audible note, save/reboot/reopen MIDI metadata, and preserve workspace,
piano-roll and meter screenshots. Exact ELF/ISO/runner hashes and failure attempts
belong with that result. Guest scanner and host checks remain unchanged.

Next workflows remain authorized work: external MIDI/velocity-lane editing and
user sample mapping; loop/marker/range export plus precise region handles;
comping/punch and routing/automation; and the separately owned CLAP/VST3 domain
boundary with real ABI/format/crash proof. Each step gets its own actual gate and
screenshots. No full-parity or live-plugin claim is made by this slice.


## First actual workstation qualification (2026-11-15)

`target/workstation-proof-v3/result.json` passes both boots and six exact pixel
checks. The tracked `MOAN_WORKSTATION_RUNTIME_2026_11_15.json` records the complete
source/assets closure and artifact digests. This is a development app install over
normal ISO `e23d9d5d…`, not a newly sealed full-image build. Actual app ELF:
`8d22fb8257ef8608efe79e1414bb92172e2dcf862a97d20311b1129e1d05bf34`.

- 68 Moan/core host tests and six independent audio-oracle regressions pass;
  the guest default-seed build passes the unchanged forbidden-byte scanner.
- Real Weft pointer input creates A3, edits duration, transposes to A4 and halves
  velocity. Captured stereo PCM has continuous 4.00/4.01-second notes at
  220.064/440.066 Hz, with RMS ratio 0.4995. Every interior 10ms tone window must
  remain above the original continuity floor; short dropouts still fail.
- The actual original sampler plays at two octave rates. Total-energy ratio is
  0.4955 and 50/90/99% cumulative-energy decay times scale approximately 2:1.
  Its low-note tail contains a 10ms threshold dip followed by a 20ms island.
  The oracle measures onsets, finite support, energy and decay scaling rather
  than miscount that quiet tail as a fifth note. Extra onsets and early dropouts
  are explicit negatives. The original WAV remains unchanged; only the existing
  strict QEMU placeholder-header reader is used.
- A 2,304,490-byte v2 project preserves three real tracks, exact notes and PCM
  caches. Independent disk extraction/decoding passes, followed by cold reopen,
  a real edit/undo returning to clean state, actual run retirement and a later
  durable empty-session save. Six source/state-bound screenshots include the
  three-track Arrange workspace, Piano Roll, real meter fill and reopened notes.
- The initial short-write feeder failure and the next checker-header failure
  remain in v1/v2 directories with source snapshots. None is relabelled green.

### Toolkit boundary and next workspace

Weft currently exposes only horizontal `slider` controls
(`crates/weft/src/build.rs:155`). The intermediate mixer therefore labels real
level and pan controls; it does not draw imitation vertical faders. Vertical
fader/meter widgets and expanded Mix composition remain an owner-coordinated
next step.

At `crates/weft/src/ui.rs:281`, `set_visible(false)` sets node visibility but
passes the original layout to `style_for`; it does not set `Display::None`.
Minimal reproducer: a 150px sidebar and a fill panel in a row; hide the sidebar,
layout again, and the fill panel retains the 150px offset. Moan's local
`visible_layout` sets both visibility and `Display::None`, without changing the
separately owned toolkit. Host tests check the removed offset, aligned headers,
reachable transport at 1264×730/640×400 and existing 550×144 canvas behavior.

The user-requested next workspace is a real 16-step editor over the same compact
MIDI notes, using one BPM-derived rational frame mapping, velocity and gate.
It is not Live Loops/scene launch. It needs independent captured onset timing,
save/reboot proof and new source-bound screenshots. Existing v3 screenshots must
not be relabelled to later Step Sequencer, mixer, Stop or plugin sources.


## Second actual slice: Step Sequencer and prompt Stop

The Step Sequencer edits the same source notes as Piano Roll; project selection,
transport and compact undo remain shared. Sixteen eighth-note steps cover two 4/4
bars. One direct rational helper computes each position as
`floor(48000 × 30 × step / BPM)`; no accumulated per-step truncation. The host 137 BPM
case explicitly catches accumulated rounding. The displayed BPM is the persisted
pattern tempo, with no unimplemented tempo knob. Cells beyond a shorter region
are disabled; the existing 128-note/32-voice limits remain enforced before baking.
Velocity controls only newly inserted notes; Set gate is an explicit operation on
existing notes. Ctrl+Shift+V still intentionally halves all existing note velocities.

`target/step-proof-v2/result.json` passes the real 1280×800 editor, captured onsets
at 0.25/0.75/1.75/3.25 seconds, actual 0.188/0.188/0.188/0.125-second gates and
C4 pitch/half-velocity energy. A 768,186-byte project independently preserves exact
start/length/key/velocity values, then reopens cold. With 64,512 frames prefilled,
Stop is acknowledged at recorded frame 26,613; the last energetic sample is 44,994
(0.383s later), with 47,104 total captured frames. The checker rejects an expired
probe or playback of the whole queued prebuffer, and allows only the existing
bounded hardware tail. This is QEMU evidence, not a physical latency claim.

The identical app ELF also passes actual 640×480 /624×410-client pointer editing,
controls and recorded onset/gate/velocity checks in `step-proof-narrow-v1`. That is
one narrow boot, not a second cold-reopen or Stop qualification. Its separately
named development ISO grafts only the Limine resolution; all 83 extracted production
ELFs are byte-identical to the source c84 ISO. Replayed boot metadata changes only
LBA references, individually checked against their actual BIOS/UEFI payloads.
The embedded installer boot image is unchanged and explicitly not qualified.
`MOAN_STEP_RUNTIME_2026_11_15.json` records all exact digests and both runner revisions.

71 Moan/core tests and five independent onset/Stop-oracle regressions pass. The
original Step v1 failure is preserved: the gate wrongly invoked the all-notes
velocity command instead of the next-note slider, and the unchanged energy oracle
correctly rejected it. The corrected action has a real native Weft regression.

Instrument recipe algorithms are fixed by project version 2. A future DSP change
must preserve the v2 renderer or add an explicit versioned recipe migration; it
must not reclassify valid older projects as corruption because a new bake differs.

## Current qualified workflow checkpoint: workspaces and offline effects

This dated checkpoint supersedes the candidate-status column above for the
specific tested workflows. It does not upgrade unimplemented reference features.
The app source is8d46f756; its later palette and restricted-role revisions require
separate evidence. `MOAN_WORKSPACE_LIVE_RUNTIME_2026_11_15.json` binds the actual
four workspace screenshots and independently captured live mix. The parent-owned
`MOAN_OFFLINE_UI_RUNTIME_2026_11_15.json` binds six exact model/RGB GUI captures and
sample-by-sample project extraction over the preserved v9 normal image.

| Workflow reference | Current Moan status | Concrete remaining boundary |
|---|---|---|
| Logic/Audacity workspace composition | Implemented shared Arrange, Mix, Piano Roll, Step Sequencer and focused Instruments panel, fixed transport, optional library/region controls | Horizontal native faders; richer vertical strips, custom workspaces/themes and per-track color polish remain separate |
| Logic/SoundCraft musical editing | Implemented bounded notes, velocity/duration, transpose/quantize, step input and actual original synth/sampler playback; saved notes reopen cold | External MIDI devices/files, velocity lanes, expressive controllers, notation and user sample mapping missing |
| Logic/Live step workflows | Implemented persisted16-step eighth-note editor with one rational BPM clock; captured onset/gate/velocity proof | Repeat/skip/direction/randomization, probability, scene launch and Live Loops missing |
| Logic/SoundCraft listening while mixing | Implemented safe live master/track level, pan, M/S with captured level/pitch/continuity checks; one undo per live pass | Changes affect newly rendered blocks after queued audio; routing, buses/sends/VCA, automation and delay compensation missing |
| SoundCraft/Logic third-party effects | Implemented user-picked offline CLAP/VST3 effect job and new editable rendered track; actual swap, crash recovery, cancellation, reuse and saved original/result PCM verified | Ordinary helper crash containment only; hostile restricted provider role pending. No live insert, external instrument/MIDI ports, LV2/AU, descriptor-driven named controls or arbitrary desktop binary support |
| Audacity4 trim/stretch/repair | Partial existing selection/cut/split/fade/drag plus original waveform/MIDI regions | Edge handles, warp/stretch, direct sample drawing and spectral repair missing |
| Logic/SoundCraft takes and automation | Missing beyond one bounded capture per recording operation | Comping/playlists, punch, monitoring, automation lanes and Write/Touch/Latch/Trim remain missing |
| Audacity4 repeat/annotation/export | Real WAV/FLAC/MP3 mix export implemented in existing pipeline | Repeat-range transport, markers/memory locations, label export, stems and batch export missing |

The wide accumulator now applies master gain before final i16 saturation. Host
regression verifies two overloaded positive/negative tracks retain headroom when
master is lowered, unity stays byte-identical, and project-loaded export uses the
same path. Nonunity master metadata uses bounded project v3, while unity audio-only
v1 and MIDI v2 stay compatible. A nonunity-v3 cold-reopen audio gate is not claimed
by the current unity-project GUI or live-control proof.

The later colored-build checkpoint now additionally qualifies nonunity-v3 cold
reopen and heard/exported headroom: three real 200% tracks/master 50% preserve exact
original sources, actual unclipped output and every independently expected WAV
sample. `MOAN_FINAL_REGRESSION_2026_11_15.json` distinguishes that new source-bound
case from the earlier unity-only GUI checkpoint and records the final legacy
project/export/capture/Close results. This adds no automation/routing/live-insert
or restricted-plugin-role claim.
