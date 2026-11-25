# Vellum signing key operation

- Shibari's RAM-only keeper can now sign bounded CMS signed-attribute bytes with ECDSA P-256 and SHA-256 for the owning domain. It returns a DER signature and an optional public point, never the private scalar. Invalid keys, oversized requests and other owners are refused.
- A focused host test verifies the signature with OpenSSL's independent implementation and checks that tampered input fails. The existing Vellum QEMU gate captures `pdf-signature-inspection.ppm` as viewer evidence; it does not exercise this backend operation.
- A Shibari PDX service, X.509 certificate import/matching, CMS construction, PDF-level cryptographic verification and signing UI remain before Vellum can offer digital signing. Full build and QEMU execution run only on GitHub CI.
