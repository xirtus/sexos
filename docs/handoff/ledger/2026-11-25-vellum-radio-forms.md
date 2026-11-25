# Vellum radio form controls

- Gap: the PDF editor could write radio fields, but Vellum skipped them in form focus and pointer hit testing. A person could not complete common forms with radio groups.
- Change: expose each widget's own PDF on-state, include radio groups in fillable order, choose by Up/Down plus Space or by clicking a widget, and preserve the existing incremental save and undo path. Widgets with no known on-state are refused.
- Proof: host form round-trip checks distinct Red/Blue widget states, target check includes pointer hit tests, and the QEMU gate selects Blue by keyboard, reopens the saved file, then selects Red by pointer. The gate captures `pdf-radio-blue.ppm` and `pdf-radio-red-pointer.ppm` and verifies saved widget states from the guest disk.
- Checks: focused `vellum-edit` host tests, `cargo check --locked -p pdf --bin pdf --tests --target x86_64-unknown-linux-musl`, fixture determinism, Python syntax, and `git diff --check`. Full image build and QEMU execution run on GitHub CI.
