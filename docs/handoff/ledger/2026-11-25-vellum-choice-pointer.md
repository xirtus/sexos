# Vellum pointer choices

- Added a bounded native option menu for the focused AcroForm combo or list field. Clicking the form bar opens it; clicking an option stores its PDF export value, and multi-select lists stay open for repeated toggles. The pointer wheel scrolls long menus.
- Added a host test over a real advanced-choice PDF to check export values, multi-select toggling, and incremental save/reopen. The Vellum x86 test target checks menu geometry; the Documents QEMU gate clicks the menu, captures open and selected screenshots, saves, and checks extracted guest PDF values on the host.
- Full image build, guest pointer behavior, and screenshot proof run in GitHub CI. PDF widget click-to-focus and broader form pointer controls remain open.
