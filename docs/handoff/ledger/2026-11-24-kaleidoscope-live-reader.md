# Kaleidoscope reader view uses the shown document

Ctrl+Shift+R extracts from the current document DOM, including changes made by
page scripts. The browser consumes this article only for the immediate reader
navigation; it does not save the article to the profile. Direct reader addresses
continue to fetch their target.

The existing reader QEMU gate now rejects a second article request and checks
the current-document navigation record while capturing `reader.ppm`. Full build
and QEMU verification run in GitHub CI.
