# CLAP qualification source

Source: https://github.com/free-audio/clap
Commit: a47f6badb49d948fd009998f28309cdab78979c9
MIT license retained in LICENSE. include/ and src/plugin-template.c are unchanged
upstream files. The build produces the unchanged stereo-swap processor and two
explicit negative variants: UD2 or an endless pause loop inserted at process entry.
Those fault variants are qualification fixtures, not upstream behavior.
