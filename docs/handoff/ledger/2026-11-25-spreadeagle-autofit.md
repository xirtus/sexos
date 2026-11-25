# Spreadeagle: fit selected grid sizes

- Ctrl+Alt+C fits selected columns to displayed cell text, and Ctrl+Alt+R clears custom sizes and fits selected rows to their contents. Both use GridCraft's native AutoFit commands, so each is one undoable workbook change and saves to XLSX/ODS.
- Row fitting refuses an active AutoFilter because GridCraft currently unhides filtered rows when it fits. The in-app and written shortcut references explain the control and this limit.
- Focused host tests cover column and row changes, undo/redo, and XLSX reopen. GitHub CI's Programs guest gate captures the fitted grid as `spreadeagle-autofit.ppm` after a manual resize.
