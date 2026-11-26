# Vellum signature field placement

- Branch: `codex/vellum-signature-field`, based on the Vellum content-integrity stack.
- Gap: a person could inspect a signed PDF but could not prepare an unsigned PDF
  with a digital signature field in the Vellum window.
- Change: Ctrl+Shift+D creates a standard visible `/Sig` widget in the chosen
  region, updates the AcroForm and page annotations, and saves it incrementally.
  Duplicate names, malformed placement, and already signed documents are refused.
- Evidence: `vellum-edit` round-trip and refusal tests; the GitHub CI Vellum QEMU
  edit gate saves, reopens, inspects, and screenshots the placed field.
- Remaining boundary: `crates/shibari` is a library, not a booted PDX signer.
  There is no certificate import and approval flow, Shell mediated signing route,
  certificate chain validation, or signature permission policy. Vellum still never
  receives private key bytes.
