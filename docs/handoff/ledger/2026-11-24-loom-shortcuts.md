# Loom shortcut reference

- Added `docs/apps/loom-shortcuts.md` from the controls in the native Loom window. It covers navigation, tabs, split view, file actions, archives, trash, search, metadata and encrypted home.
- Clarified that prompt keys depend on context, destructive operations ask twice, and redo remains unavailable. No runtime control changed.
- Validation: checked the reference against `apps/files/src/main.rs` and `git diff --check`. The branch's GitHub CI full run and its existing Loom QEMU screenshot gate provide integration evidence; no local full build or QEMU run.
