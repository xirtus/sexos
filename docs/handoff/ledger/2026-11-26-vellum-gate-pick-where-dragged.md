# Vellum gate clicks the region highlight where it was dragged

The pdf-edit gate picked the new region highlight by pointer at a point
worked out from fixed window chrome and a page that fills the window. Vellum
centres a page that fits, so that point missed the highlight and
`[pdf.annot.pick]` never came. The gate now clicks the middle of the area it
dragged, with the same window-to-page mapping it used for the drag. Gate-only.

Follow-up: on CI the Vellum window is only 144 pixels tall, so the middle of
the dragged area lay below the window and the click became a pointer Leave.
The gate now clicks just inside the drag's top edge.

Second follow-up: a click just inside the drag's top edge still missed (the
page can move a few pixels when the status line changes). The gate now tries
points down the dragged area, 6 px apart, until Vellum reports a pick.
