# Vellum gate clicks the region highlight where it was dragged

The pdf-edit gate picked the new region highlight by pointer at a point
worked out from fixed window chrome and a page that fills the window. Vellum
centres a page that fits, so that point missed the highlight and
`[pdf.annot.pick]` never came. The gate now clicks the middle of the area it
dragged, with the same window-to-page mapping it used for the drag. Gate-only.
