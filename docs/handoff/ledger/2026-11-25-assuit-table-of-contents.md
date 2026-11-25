# Assuit table of contents

- **Gap:** Assuit could apply heading styles, but could not create or refresh a table of contents from them.
- **Change:** Ctrl+Alt+T presents a bounded native action prompt to insert heading levels 1–9, update an existing table, or remove it. WordCraft owns fields, pagination, serialization and generated entries.
- **Evidence:** A focused host test covers invalid commands, visible rendering, DOCX/ODT save/reopen, refreshing a new heading and removal. The guest gate captures the prompt and generated table; full build and QEMU run in GitHub CI.
- **Limit:** Advanced contents formatting, custom entry styles and multiple tables remain outside this slice.
