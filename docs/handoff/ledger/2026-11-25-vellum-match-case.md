# Vellum Match Case search

Vellum's Find always ignored letter case, so a person could not narrow a search
to an exact capitalization. Ctrl+Shift+M now toggles Match Case while Find is
open. The current query reruns immediately, the bar shows the mode, and the
same source character spans drive page highlights. Default search still ignores
case and still normalizes layout whitespace.

Host coverage: the `match_case_filters_hits_without_changing_source_spans`
search test checks mixed-case hits, page order, and a match across a line break.
CI guest coverage: `program_pdf_find_gate.py` searches for the lowercase form
of the fixture's uppercase-only word, toggles Match Case, checks zero hits,
captures `pdf-find-match-case.ppm`, then toggles back and checks the original hit.

Limits: this adds exact case matching for extracted text. It does not change
PDF text extraction or multi-column reading order.
