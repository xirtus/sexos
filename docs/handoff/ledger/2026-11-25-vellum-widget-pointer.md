# Vellum PDF widget pointer focus

- A pointer click on a visible, editable PDF widget now maps through the current client viewport, zoom, scroll, crop and page rotation to its field. Text widgets focus for typing, choice widgets focus and open Vellum's own option menu, and checkbox widgets toggle directly. Read-only and password widgets remain excluded by the existing fillable-field filter.
- Host tests cover viewport conversion with centering and scroll, and hit testing real choice widgets under all four rotations while rejecting another page and read-only fields. The Documents guest gate clicks a checkbox and choice widget, captures `pdf-widget-checkbox.ppm` and the choice-menu screenshots, and checks the saved PDF's export values.
- Full image build and QEMU pointer proof run on GitHub CI. Drag selection and richer text controls remain open.
