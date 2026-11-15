# Moan track colors and responsive workspace proof

The palette app (source `df16fc3d`, staged ELF `a2f6a58a…`) passes two wide and
four narrow actual QEMU screenshots. `MOAN_TRACK_COLORS_RUNTIME_2026_11_15.json`
binds the exact source, ISO/config, ELF, private disks, runners, expected label
patches, captures and preserved failures. This is a development app install over
the preserved v9 normal image, not a new canonical sealing or installer proof.

One ordered eight-track palette now identifies audio and MIDI region fills,
waveform/note ink, native header swatches and mixer names. Selection adds a neutral
brightening rather than discarding the track hue. The wide gate checks exact four
real audio fills, selected state, header swatches and actual waveform ink, then
all eight real mixer names. The latter compares every RGB byte of each 118×19
label rectangle against an independent source-built Weft caption patch: explicit
approved foreground, known 0x35313b background, 14px base/Caption font and expected
text. No guest-reported hash/color is an oracle, and no antialiased/background
pixel is omitted. Source/font/manifests/lock/expected-table hashes are retained.
The maintained generator additionally emits a checked complete 48-file local
Weft/font input receipt; its eight expected patches are byte-identical to those
actually qualified.

The distinct development 640×480 image preserves all 84 extracted production ELFs
from v9. Only Limine resolution and replayed BIOS/UEFI LBA metadata differ; exact
payloads outside the BIOS boot-info table are unchanged. Its embedded installer
image is unchanged and unqualified. The actual framebuffer is 640×480 and the
client 624×410. Real pointer Step editing, the same C4 note in Piano Roll,
master 50→100/mute/unmute controls and exact Instruments status text/RGB all pass.
Native horizontal faders remain the toolkit boundary. These UI checks add no new
audible, restricted-role or physical-device claim; prior live-audio proof stays
attached to its exact 8d46 source.

Initial narrow attempts tried to park the pointer in the top-left corner. Shell
correctly rejected that large jump as `corner_poison_after_ready`, leaving a
cursor over the sampled note pixel. The corrected checker parks at the global
bar midpoint, requires its accepted normalized position and fresh Display
publication, then applies the unchanged exact pixel predicate. The first wide
attempt's fully opaque text assumption was corrected to full source-selected
coverage-aware RGB patches. All original failed captures and source snapshots
remain intact.

101 app/core/plugin host tests and the unchanged default-seed guest forbidden-byte
scan pass. One-off graphic/text contrast calculations remain separate from
runtime pixel qualification. Reproduce with `scripts/moan_palette_oracle.py` to a
fresh directory, pass it as `--label-oracle` to `scripts/moan_track_colors_gate.py`,
and use `scripts/moan_narrow_workspace_gate.py` with the explicitly derived
640×480 image. Both gates require fresh output directories. Final project/export/
microphone/Close regression is a separately recorded check of this new ELF.
