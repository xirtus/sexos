# Kaleidoscope keyboard shortcuts

Open the in-app reference with **Ctrl+Shift+/** or enter `about:shortcuts` in the
address bar. These controls belong to Kaleidoscope and work without a network grant.

| Action | Shortcut |
| --- | --- |
| Address, reload, find, bookmark | Ctrl+L, Ctrl+R, Ctrl+F, Ctrl+D |
| Downloads, reader view | Ctrl+J, Ctrl+Shift+R |
| Stop an active download | Esc |
| Pause or resume an active download | Ctrl+Shift+J |
| Zoom in, out, reset | Ctrl+=, Ctrl+-, Ctrl+0 |
| New, close, next, previous tab | Ctrl+T, Ctrl+W, Ctrl+PageDown, Ctrl+PageUp |
| Select tab 1–8, last tab | Ctrl+1–8, Ctrl+9 |
| Private tab, container tab 1–4 | Ctrl+Shift+P, Ctrl+Shift+1–4 |
| Vertical tabs, split view, group, side panel | Ctrl+Shift+,, Ctrl+Shift+\\, Ctrl+Shift+G, Ctrl+Shift+B |
| Site settings, Shields, JavaScript, clear site data | Ctrl+Shift+E, Ctrl+Shift+S, Ctrl+Shift+Y, Ctrl+Shift+Backspace |
| Request Chastity fill for a visible HTTPS login | Ctrl+Shift+K |
| Ask Chastity to review saving a visible HTTPS login | Ctrl+Shift+U |
| Keyboard reference | Ctrl+Shift+/ |

The reader view shortcut returns to the source page when used in reader view.
Site settings shows the current container's cookie and local-storage counts for
the site. Ctrl+Shift+Y blocks or allows JavaScript for that exact origin and
reloads the page. The choice survives restart. It covers page scripts, inline
handlers, and content scripts on that origin. Clearing site data refreshes the
stored-data counts.
The downloads panel lists completed files and failed transfers. **Try again**
opens the failed address again; completed files open from the same panel.
Ctrl+Shift+J pauses the live transfer at its current staged file offset and
resumes the same connection; if the site closes it while paused, the download
fails with a retry link. Esc stops it, removes the hidden partial file, and
shows a retry link in the downloads panel.
The Chastity request requires a visible, unambiguous same-origin login form.
Saving additionally requires current username and password values. Chastity must
approve and store the proposal before Kaleidoscope reports a saved password.
