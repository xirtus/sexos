# Vellum signed form guard

- Gap: annotation changes refused signed PDFs, but form filling and another
  signature field could still change a signed revision. A catalog `/Perms
  /DocMDP` reference without a discoverable signature form field was missed by
  signature-field placement.
- Change: share the fail-closed signed-revision check across annotation edits,
  form filling, signature-field placement, and another CMS signature. Vellum
  refuses form entry before editing when that check reports a signature or
  unreadable permissions.
- Proof: focused editor tests reject field edits and field placement for a
  signed field and a catalog-only certification reference without changing
  the editor; the PDF guest gate attempts form entry after signing and on the
  catalog-only fixture, checks the refusal, and captures
  `pdf-signed-form-refused.ppm`.
- Checks: focused `vellum-edit` tests, PDF viewer target check, Python gate
  syntax, and diff check passed locally. Full build and QEMU proof run only on
  GitHub CI.
