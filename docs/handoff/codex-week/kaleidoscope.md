# Kaleidoscope daily-driver gaps

Compared with Firefox and Safari, Kaleidoscope already has persistent tabs, history,
bookmarks, cookies, downloads with a panel, reader view, private and container tabs,
and Shields. The remaining gaps are:

- A password manager with safe autofill and passkeys, integrated with Chastity.
  The first autofill slice defines and tests exact top-level HTTPS scope and an
  unambiguous same-origin POST login pair. This slice wires explicit Ctrl+Shift+K
  requests to Chastity, rechecks the
  tab and document after approval, and cancels pending requests on navigation
  or tab changes. A browser capture policy and Chastity's scoped add/update
  decision are host-tested; the Shell write transfer, visible Chastity approval,
  browser command, and passkeys remain.
  The Shell broker now has an attested, bounded reverse proposal route from
  Kaleidoscope to Chastity. Ctrl+Shift+U now captures the current eligible
  login, transfers it through that route, and cancels when the page or values
  change. Chastity now exposes a fully received proposal only after bounded
  transfer and can acknowledge it only after the caller's durable commit.
  Chastity now shows a default-deny save dialog with Add or the one exact
  matching account as Replace, stores the encrypted vault, and only then
  acknowledges the browser. The GitHub CI two-app QEMU gate is the runtime
  evidence for this first end-to-end password save. Passkeys still remain.
- A visible per-site settings view for Shields and stored data. This change adds a
  first view and controls; permission prompts and a full settings catalog remain.
- Stronger tab isolation, developer tools, extensions, and browser compatibility.
- Inline PDF, print to PDF, service workers, and web app installation.
- Profile and session writes previously replaced live files in place. This slice
  stages and syncs each save, retains a previous copy, and recovers it after an
  interrupted promotion. Other browser state files still need the same treatment.

Keep browser state and controls within its own capabilities. Do not add a POSIX
filesystem or network path.

The existing browser controls now have an offline in-app shortcut page and a
matching keyboard reference in `docs/apps/kaleidoscope-shortcuts.md`.

Reader view now extracts the article from the currently shown DOM. It keeps
script-added text and avoids a second request for a page that may have changed
or required a sign-in. Direct `about:reader?url=` addresses still fetch once.

The browser's own controls now follow SexOS light, dark and high-contrast
appearance and accent changes. Site content keeps its own CSS colors.
