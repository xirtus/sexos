# Spreadeagle chart type chooser

Ctrl+Shift+M offers column, line, bar, and pie charts for the selected range.
Ctrl+Shift+C remains the quick column-chart action. The native chooser passes
the selected type to GridCraft and uses only the workbook already granted to
Spreadeagle.

The host test checks all four chart kinds after an XLSX save and reopen. The
CI guest gate selects pie, checks the app record, captures the chooser and chart,
then confirms both charts survive a restart. Full builds and QEMU run on GitHub CI.
