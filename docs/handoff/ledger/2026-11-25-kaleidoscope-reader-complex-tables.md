# Kaleidoscope reader tables: row spans and nested data

The Firefox/Safari comparison in `docs/handoff/codex-week/kaleidoscope.md` still
identified row spans and nested tables as gaps in article reading. Kaleidoscope's
reader now retains bounded `rowspan` values and serializes nested data tables
inside their parent cells. The article's visible word count includes nested
content once, with cell boundaries kept so separate words do not run together.
Hidden content, styling, and event handlers remain excluded.

The `sex-web` reader host test covers a two-row span, a nested table, an
out-of-range span, stripped event handlers, and the nested table's exact word
count against equivalent plain text. The existing GitHub CI reader
QEMU gate uses an updated article fixture with a row span and nested table and
uploads its screenshot. Full image build and QEMU execution belong to CI.
