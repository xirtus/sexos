# Vellum guest digital signing flow

- Vellum selects a saved, empty signature field with Ctrl+Shift+B. Enter confirms the request; Escape cancels. Unsaved edits and already signed PDFs are refused.
- The Shell broker routes a bounded CMS attribute request to an independently launched Shibari signer. Its owner loads a matching P-256 certificate and private scalar in Shibari and approves each request with Y. The scalar remains in Shibari's protection view.
- Vellum embeds the returned DER signature in detached CMS, checks covered-content integrity before writing, reopens the saved PDF, and checks again. It reports content integrity separately from unverified signer trust.
- The PDF QEMU gate installs and launches both programs, signs a real guest PDF, captures `pdf-digital-signature.ppm`, extracts the saved file, and checks it with `vellum-edit` plus independent OpenSSL CMS verification. Full build and QEMU execution run on GitHub CI.
- Certificate-chain trust, signer identity, other CMS algorithms, PDF permission transforms, and multiple signatures remain open.
