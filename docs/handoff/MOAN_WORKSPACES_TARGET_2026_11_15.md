# Moan workspace target

User direction, 15 November 2026: the four supplied Logic screenshots are the
visual target. Moan should be GarageBand-simple with Logic depth, using Weft.
The source images are retained locally in target/moan-user-ui-references-2026-11-15
with hashes; they are internal references, not shipped artwork.

One project has several quick-switch workspaces: Arrange, Mix, Piano Roll,
Step Sequencer and Instruments. They share tracks, stable selection, transport,
project persistence and undo. A view change does not edit the musical document.
Panels can dock or float while the compact transport stays accessible.

Arrange leads with aligned track headers and colored waveform/MIDI lanes,
beat/time ruler, playhead and region handles. Mix docks real channel strips and
master below it: meters, faders, balance, mute/solo and supported effects.
Library and Smart Controls occupy optional side panels. Dense controls must fit
1280x800 and retain a usable smaller-window layout without clipping track data.
Piano Roll and Step Sequencer use the lower editor area. Instrument, sampler,
drum-pad and plugin controls open as focused editors within the same workspace.
Only implemented, data-backed controls and actual tracks are shown.

Step Sequencer edits the same persisted MIDI notes: an initial 16 step grid,
note/drum rows, step toggles, playback position and real velocity/gate controls.
Frame-to-beat/step conversion uses actual BPM without accumulated rounding drift.
Tempo changes must affect rendered timing before being exposed as functional.
Note limits stay bounded; future repeat/skip/direction/randomization controls
require their own implementation and tests. Live Loops-style clip/scene launch,
shown in reference 4, is a separate future performance workspace.

Each step needs a real QEMU gate and source/binary-bound screenshots. Verify
captured audio continuity, pitch/velocity/onset timing, actual consumed playhead,
prompt Stop, edit/undo, save/reboot/reopen and responsive workspace switching.
Earlier partial captures or failed WAVs remain unchanged and keep their original
source cohort. Offline plugin rendering requires actual Moan controls and result
import; a qualification helper alone does not complete that workflow.

Primary references:

- https://www.apple.com/hr/newsroom/2020/05/apple-unveils-biggest-update-to-logic-since-the-launch-of-logic-pro-x/
- https://support.apple.com/en-us/109503
- https://www.audacityteam.org/audacity-4/
- https://getartcraft.com/apps/soundcraft
- https://www.ableton.com/en/live-manual/12/live-concepts/

The user also supplied https://horiamc.com/blog/logic-pro-13-new-features-2026;
feature claims are checked against Apple's official release notes. The matrix
separates released reference features, implemented Moan behavior and remaining
work. No full product parity or hostile-plugin isolation is implied.
