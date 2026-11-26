# Loom local Places navigation

- Branch: `codex/sexfiles-places-navigation`, stacked on `codex/sexfiles-graphical-split` (#674).
- Gap: reaching a common folder or the granted start folder required navigating back through each directory, especially when two folder tabs were open.
- Change: Ctrl+Shift+B opens an app-owned local Places list. It offers the granted start, previous folder, and available SexOS Home, Downloads, Documents and Disks locations. Candidates are confined to the current Files grant; Loom checks each folder through Files before showing it. Up/Down and Enter navigate; Escape closes. It works from a single folder or split text/grid view and changes only the active tab.
- Focused host test: `cargo test -p loom-core quick_places::tests` checks grant boundaries and duplicate removal. Targeted `cargo check -p files --target x86_64-unknown-none` checks the native UI.
- QEMU gate: `scripts/files_loom_gate.py` opens Places inside a USB folder, captures `folder-places.ppm`, returns to the granted root, reenters the folder, and opens/closes Places while the graphical split is active. Full build and QEMU run on GitHub CI.
- Remaining: persistent custom favorites and a graphical sidebar; this slice is a compact keyboard list. SexFiles grants, not ambient paths, determine what appears.
