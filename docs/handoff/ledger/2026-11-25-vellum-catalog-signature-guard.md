# Vellum catalog certification guard

- Gap: the signed annotation guard inspected AcroForm signature fields only.
  A catalog `/Perms /DocMDP` reference without a discoverable signature field
  could still reach annotation mutation.
- Change: refuse annotation edits when the catalog carries `/DocMDP`; reject a
  malformed `/Perms` dictionary rather than treating it as absent.
- Proof: focused PDF fixture has a catalog DocMDP reference and no signature
  form field. Add, text edit, and remove all refuse without a pending change.
  The QEMU edit gate opens a corresponding one-page fixture, attempts a
  highlight, and captures `pdf-catalog-docmdp-refused.ppm`.
- Checks: focused `vellum-edit` test, Python fixture generation and syntax,
  x86_64 viewer check, and diff check locally. Full build and QEMU run on
  GitHub CI.
