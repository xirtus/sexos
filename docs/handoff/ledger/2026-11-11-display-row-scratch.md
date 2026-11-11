# Display row workspace: stack reduction (2026-11-11)

- `SceneSnapshot` owns bounded row edges/runs in a `RefCell<RowScratch>`. Preparation fills the existing storage and returns a small borrowed view; no large `RowWinners` aggregate is returned through the render stack. Scratch holds indices and geometry only, and the view resolves them exclusively against its borrowed immutable scene surfaces.
- `raster_row` borrows scratch for one row. Reentry uses the existing scalar compositor; teardown clears scratch through the exclusive scene borrow. Scene capture remains in-place, neither Copy nor Clone; no producer pointer or lifecycle authority enters scratch.
- Host gates adapted: independent rounded-shape owner checks, current-slice pointer identity after scratch reuse/reordering, shorter/empty rows, and exact fast/scalar pixels plus contributor bits when scratch is already borrowed.
- Validation pending compiler slot: source reviewed only at this commit. The previous ELF stack analysis remains incomplete; this source change is not a 64 KiB proof. Rebuild and inspect generated frames plus full call paths before qualifying the stack. No kernel or ABI growth.
