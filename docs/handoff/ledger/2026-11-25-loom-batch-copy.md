# Loom batch copy of a selected range

- Ctrl+C takes a contiguous text-list range of 2–64 files or folders. Ctrl+V copies it into the current folder or another folder through one background job. All source trees are planned before writing, and destination names are reserved with `(copy)` suffixes when needed.
- The job stages each file under a hidden partial name. Escape or a failed step removes every completed destination and partial file made by the group. A successful group enters undo as one step; Ctrl+Z removes its copies together. Ratings and tags, including records under copied folders, follow the copies after the job finishes.
- The host selection test covers duplicate and occupied names and the group bound. The Loom QEMU gate copies two rated files in place, checks the collision names and copied rating, captures `loom-batch-copy.ppm`, then undoes both. Full build and QEMU run on GitHub CI.
- Batch cut remains limited to one disk. The batch copy uses the source and destination capabilities already held by Loom; grid and noncontiguous selection remain open.
