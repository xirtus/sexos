# Kaleidoscope tab groups (N13d), 2026-11-21

Tab groups as in Chrome, Edge and Brave:

- **Ctrl+Shift+G** puts the shown tab in a new group, or takes it out of its group.
  - A new group is named for its site (`example` for news.example.com, `bbc` for bbc.co.uk, an IP address stays whole).
  - It takes the next of Chrome's eight group colours that no group has yet.
  - A tab taken out moves to just after its group, so a group's tabs always sit together.
- **New tabs** opened from a grouped tab join its group, beside it.
- **The group's label** comes before its tabs: a coloured chip in the bar, or a row with a coloured dot in the vertical strip.
  - Grouped tabs are underlined in the group's colour in the bar, and set in beside a colour line in the strip.
- **Clicking a label folds the group.** Only the label shows, with the tab count.
  - Folding the shown tab's group first shows the nearest tab outside it.
  - Showing a tab of a folded group (Ctrl+1..9, Ctrl+PageDown) unfolds it.
- **Empty groups are forgotten** when their last tab closes or leaves. At most 8 groups per window.

Code:
- `sex_web::{TabGroup, Slot, strip_slots, tab_chips, side_row_at, group_name, GROUP_COLOURS}`.
- `Chrome.group_of` and `Chrome.groups`.
- `Browser.groups`; `Tab.group` stays with the tab slot and is not swapped.
- In apps/web/src/tabs.rs: `toggle_group`, `fold_group`, `move_tab`, `prune_groups`.

Serial records:
- `[web.group] group= tab= name=`
- `[web.group.joined]`
- `[web.group.left] group= from= to=`
- `[web.group.folded] group= on=`
- `[web.group.gone]`

Gate: `scripts/program_web_tab_groups_gate.py` (CI). It covers grouping, joining, leaving, folding from the bar and from the vertical strip, unfolding by showing a tab, and forgetting an empty group.

Not yet: renaming a group or choosing its colour, dragging tabs between groups, and keeping groups across restarts (Kaleidoscope has no session restore yet).
