# Kaleidoscope keyboard shortcuts

Open the in-app reference with **Ctrl+Shift+/** or enter `about:shortcuts` in the
address bar. These controls belong to Kaleidoscope and work without a network grant.

| Action | Shortcut |
| --- | --- |
| Address, reload, find, bookmark | Ctrl+L, Ctrl+R, Ctrl+F, Ctrl+D |
| Downloads, reader view | Ctrl+J, Ctrl+Shift+R |
| Clear history in the downloads panel | Ctrl+Alt+J |
| Stop an active download | Esc |
| Pause or resume an active download | Ctrl+Shift+J |
| Zoom in, out, reset | Ctrl+=, Ctrl+-, Ctrl+0 |
| New, close, next, previous tab | Ctrl+T, Ctrl+W, Ctrl+PageDown, Ctrl+PageUp |
| Select tab 1–8, last tab | Ctrl+1–8, Ctrl+9 |
| Private tab, container tab 1–4 | Ctrl+Shift+P, Ctrl+Shift+1–4 |
| Vertical tabs, split view, group, side panel | Ctrl+Shift+,, Ctrl+Shift+\\, Ctrl+Shift+G, Ctrl+Shift+B |
| Site settings, Shields, clear site data | Ctrl+Shift+E, Ctrl+Shift+S, Ctrl+Shift+Backspace |
| Clear only site cookies / local storage (in site settings) | Ctrl+Alt+C / Ctrl+Alt+S |
| Request Chastity fill for a visible HTTPS login | Ctrl+Shift+K |
| Ask Chastity to review saving a visible HTTPS login | Ctrl+Shift+U |
| Keyboard reference | Ctrl+Shift+/ |

The reader view shortcut returns to the source page when used in reader view.
Site settings shows the current container's cookie and local-storage counts for
the site. Clearing site data refreshes those counts. On the site settings page,
Ctrl+Alt+C clears only that site's cookies and Ctrl+Alt+S clears only its local
storage in the current container; the full clear also removes saved code.
The downloads panel lists completed files and failed transfers. **Try again**
opens the failed address again; completed files open from the same panel.
Ctrl+Alt+J clears the panel's saved history and failed-transfer list. It keeps
downloaded files and resumable transfers.
Ctrl+Shift+J pauses the live transfer at its current staged file offset and
resumes the same connection; if the site closes it while paused, the download
fails with a retry link. Esc stops it, removes the hidden partial file, and
shows a retry link in the downloads panel.
On launch, Kaleidoscope clears empty hidden download parts only when its durable
ticket names the exact part. Unticketed and nonempty parts remain for recovery.
The Chastity request requires a visible, unambiguous same-origin login form.
Saving additionally requires current username and password values. Chastity must
approve and store the proposal before Kaleidoscope reports a saved password.
