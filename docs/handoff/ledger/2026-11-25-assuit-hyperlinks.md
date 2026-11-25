# Assuit hyperlinks

- **Gap:** WordCraft's Ctrl+K sent an unhandled link UI request in Assuit, so writers could not link selected text or insert a labeled link.
- **Change:** Assuit now presents its own Ctrl+K prompt for bounded web and mail addresses. WordCraft applies the styled link to selected text or inserts an optional display label.
- **Evidence:** A focused host test checks input rejection, selection preservation, undo, and DOCX/ODT link round trips. The guest gate captures the native prompt and inserted link; full build and QEMU run in GitHub CI.
- **Limit:** Editing and removing an existing link still need native controls.
