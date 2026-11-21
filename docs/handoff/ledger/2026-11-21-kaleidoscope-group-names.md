# Kaleidoscope tab group names and colours (N13h), 2026-11-21

You can rename a tab group and change its colour, as Chrome's group editor does: right-click the group's label, in the bar or in the vertical strip.
- **Typing a name:** the bar becomes "Group name: …" and the label shows the name as you type (at most 24 characters).
- **Changing the colour:** Up and Down step through the eight group colours.
- **Finishing:** Enter keeps the name and colour, and the session saves them. Escape puts the old name back.

Code: `start_naming` and `name_key` in apps/web/src/tabs.rs; `Browser.naming` holds the group being named and its old name.

Serial records: `[web.group.naming] group=`, `[web.group.colour] group= colour=`, `[web.group.named] group= name= colour=`.

Gate: `program_web_tab_groups_gate.py` renames the group to "work" and steps it from blue to red.
