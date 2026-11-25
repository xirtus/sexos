# Date timer undo and redo — 2026-11-25

Date's Clock now records timer starts and cancellations alongside alarm and zone
edits. Ctrl+Z cancels a newly started timer or restores a cancelled one;
Ctrl+Y reverses that change. Restoring keeps the original deadline and is refused
once that deadline has passed. A timer that Bell has settled cannot be restored.
Each accepted edit and undo/redo is immediately staged and synced through Date's
existing `clock.txt` save path. The history stack itself lasts only for the open
Date session; snoozes and Bell delivery are outside it.

Focused host tests cover start/cancel, redo, saved clock round trips and elapsed
or delivered timer refusal. The calendar guest gate checks the timer's undo and
redo logs and saved `clock.txt`, then captures `date-timer-redone.ppm` with the
restored timer visible. Full image build and QEMU run belong to GitHub CI.
