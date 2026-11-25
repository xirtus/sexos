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
| Search | Ctrl+F, then Enter / Ctrl+G next, Shift previous; Ctrl+Shift+M toggles Match Case while Find is open; Escape closes |
| Choose a highlight region | Ctrl+R, then drag across the page; arrows move and Shift+arrows resize; Enter highlights |
| Highlight, add note, list annotations | Ctrl+H; Ctrl+N; Ctrl+L |
| Choose highlight color | Ctrl+Shift+H cycles yellow, green, pink, blue; the choice applies to found text and selected regions |
| Choose highlight opacity | Ctrl+Shift+O cycles 100%, 75%, 50%, 25% before placing a highlight |
| Select markup on the page | Click its visible area; the annotation list opens at that item |
| Remove selected annotation | Delete in the annotation list |
| Edit a highlight comment | Ctrl+L, select a highlight, Enter; type up to 400 characters, Enter saves or Escape cancels |
| Edit a sticky note | Ctrl+L, select a note, Enter; edit its text, then Enter saves or Escape cancels |
| Fill a form | Ctrl+E; Ctrl+Down / Ctrl+Up changes fields; type text, Space checks a box, Up / Down chooses combo, list or radio options, Enter commits and advances. Space sets a radio choice. Editable combos accept custom text (Ctrl+A clears); multi-select lists use Space to toggle the focused option |
| Edit form text at the caret | Left / Right moves one character; Home / End jumps to an edge; Backspace / Delete removes the character before / after the caret |
| Select and replace form text | Shift+Left / Shift+Right or Shift+Home / Shift+End extends a selection; Ctrl+A selects the whole text field; typing, Backspace, or Delete replaces or removes it |
| Place the form text caret | Click the focused text field's value in Vellum's form bar |
| Select form text with the pointer | Drag across the focused text field's value in Vellum's form bar; typing replaces the selected range |
| Undo / redo committed form-field edits and markup in edit order | Ctrl+Z / Ctrl+Shift+Z, before saving |
| Place a typed visual signature | Ctrl+Shift+G |
| Place an empty digital signature field at the chosen region | Ctrl+Shift+D |
| Sign a saved empty digital signature field | Ctrl+Shift+B, then Enter to request Shibari approval; Escape cancels |
| Inspect digital signature fields and supported content integrity | Ctrl+Shift+V |
| Save edits | Ctrl+S |

In Find, a single space can match words separated by a line break in the PDF.
The bar shows whether Match Case is on. Toggling it reruns the current query.
Find accepts the active keyboard layout and committed input-method text;
Backspace removes the previous displayed character, including its accents.

Editable combo boxes retain their existing Ctrl+A clear command. For a focused
combo or list field, click its value in the form bar to open
Vellum's option menu. Click a row to choose it; multi-select lists remain open
for more toggles, and the pointer wheel scrolls long lists.
Click a form widget on the page to focus it directly; a choice widget opens its
menu, a checkbox toggles, and a radio widget sets its own option. Read-only
fields are skipped.

Typed visual signatures are markup. Signature inspection verifies exact covered
bytes for the supported P-256/SHA-256 detached CMS shape. It does not verify
signer identity, certificate trust, or other CMS algorithms.
Vellum refuses adding or deleting annotations in a signed PDF until it can
evaluate the document's DocMDP and FieldMDP permissions.
To sign, run `shibari-sign` with a P-256 DER certificate in Shibari's own
window, enter the matching private scalar there, and approve each request with
Y. Vellum refuses signing when edits are unsaved or the PDF already has a
signature. It checks signed content after writing. Certificate-chain trust and
signer identity are still unverified.
