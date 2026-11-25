# Vellum: find a word split at a line-end hyphen

- The PDF fixture now visibly prints `inter-` on one line and `national` on the next. Hayro's text collector extracts `inter-\nnational`, with one box per source character.
- Search accepts `international` and `inter-national` for that precise alphabetic line-end pattern and retains the full source span for the highlight and annotation placement. Same-line hyphens such as `co-operate`, a hyphen before a space, and a query with a space do not silently join.
- Pure search tests cover the two forms, exact source offsets, and nonmatches. The extracted-PDF test checks the fixture itself; the Programs QEMU find gate searches both forms and captures `pdf-find-hyphen-joined.ppm` and `pdf-find-hyphen-literal.ppm`.
- Full build and QEMU execution run on GitHub CI. Reading order across columns remains open.
