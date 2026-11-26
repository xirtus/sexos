# Spreadeagle gate closes the filter fixture's import warning

`filter-overflow.xlsx` has no styles part, so Spreadeagle opens it with one
import warning ("no styles part") and shows the warnings dialog, which takes
every key. The gate sent Ctrl+Shift+L into that dialog and never saw
`data.filter`. The gate now waits for the dialog and closes it with Esc
first, as it already does for `formula-compat.ods`. Gate-only change.
