# Valentine local folders

Ctrl+G cycles the top level and immediate child folders of Valentine's Files
grant. Only `.eml` files in the selected folder appear in the list. New drafts
save in that folder. Path construction rejects separators and special names;
this does not add access outside the existing grant.

The host path test checks valid and escaping names. The productivity QEMU gate
creates a child folder, imports mail into it, switches folders, checks the
opened message path and captures `valentine-folder.ppm`. Full builds and QEMU
run on GitHub CI.
