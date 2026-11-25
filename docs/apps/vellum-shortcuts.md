# Vellum keyboard shortcuts

Press **Ctrl+Shift+/** for the in-app reference; Escape closes it. It follows
Vellum's light, dark, and high-contrast colors.

| Action | Keys |
| --- | --- |
| Next or previous page | Right / Space / N; Left / P |
| First or last page | Home; End |
| Scroll | Up; Down |
| Zoom | +, -, 0 / W fit width, F fit page, 1 actual size |
| Thumbnails or outline | T; O, then Up / Down / Enter |
| Search | Ctrl+F, then Enter / Ctrl+G next, Shift previous, Escape closes |
| Choose a highlight region | Ctrl+R, arrows move, Shift+arrows resize |
| Highlight, add note, list annotations | Ctrl+H; Ctrl+N; Ctrl+L |
| Remove selected annotation | Delete in the annotation list |
| Fill a form | Ctrl+E; Ctrl+Down / Ctrl+Up changes fields; Space checks a box |
| Undo / redo committed form-field edits or added/deleted markup | Ctrl+Z / Ctrl+Shift+Z, before saving |
| Place a typed visual signature | Ctrl+Shift+G |
| Place an empty digital signature field at the chosen region | Ctrl+Shift+D |
| Sign a saved empty digital signature field | Ctrl+Shift+B, then Enter to request Shibari approval; Escape cancels |
| Inspect digital signature fields and supported content integrity | Ctrl+Shift+V |
| Save edits | Ctrl+S |

Typed visual signatures are markup. Signature inspection verifies exact covered
bytes for the supported P-256/SHA-256 detached CMS shape. It does not verify
signer identity, certificate trust, or other CMS algorithms.
To sign, run `shibari-sign` with a P-256 DER certificate in Shibari's own
window, enter the matching private scalar there, and approve each request with
Y. Vellum refuses signing when edits are unsaved or the PDF already has a
signature. It checks signed content after writing. Certificate-chain trust and
signer identity are still unverified.
