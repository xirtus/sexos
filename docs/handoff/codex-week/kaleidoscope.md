# Kaleidoscope daily-driver gaps

Compared with Firefox and Safari, Kaleidoscope already has persistent tabs, history,
bookmarks, cookies, downloads with a panel, reader view, private and container tabs,
and Shields. The remaining gaps are:

- The Ctrl+J downloads panel now shows failed transfers with an exact URL retry
  link. A failed stream clears the progress bar and removes its hidden partial
  file, so retry can take the original name. Completed public downloads now have
  a bounded, staged history with source URL and size that survives restart;
  private-tab addresses stay out of the stored history. Escape now cancels a
  synchronous transfer, removes its hidden partial file, and offers a retry.
  Ctrl+Shift+J now pauses the live request and file writes at the committed
  offset, then resumes that same connection. A server that closes during a
  long pause still requires a fresh retry; durable Range/ETag resume remains.
  The hidden part is now synced before it gets a visible name, and Files is
  synced again after rename. A failed sync is reported as a failed download;
  it cannot enter completed history. If the second sync fails, the visible
  file may remain but is explicitly marked unconfirmed for retry.
  A network-interrupted or window-close partial with a strong ETag now stays hidden and is
  synced alongside bounded resume metadata. Retrying its exact URL, including
  after a browser restart, sends Range/If-Range and appends only after a matching
  `206` with exact start, total, ETag, and identity body length. A changed reply
  discards the partial and restarts from byte zero. Last-Modified alone and weak
  ETags still require a fresh transfer; a process killed mid-transfer can leave
  an orphan hidden part because no confirmed metadata was written yet.
  When the live resume metadata is readable but malformed, Kaleidoscope now
  checks its previous durable copy before abandoning a synced hidden part.
  The downloads panel now offers Ctrl+Alt+J to clear saved history and current
  failures after a durable state write. Downloaded files and resumable transfers
  remain available; orphan hidden-part cleanup is still open.

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
  first view and controls. The view now counts cookies matching this host and
  local storage items and bytes in the current container. Clearing site data
  updates those counts to zero. The person can now clear only cookies or only
  local storage from the native site settings page, scoped to the active box.
  Permission prompts and a full settings catalog remain.
- Stronger tab isolation, developer tools, extensions, and browser compatibility.
- Private URLs and page content previously reached serial diagnostics even though
  private history, session, cookies and storage stayed out of Files. Kaleidoscope
  now stops content-bearing diagnostics for the rest of a process once a private
  tab opens, including delayed callbacks after switching back to a public tab.
  Numeric lifecycle receipts remain. The private guest gate checks URL, title,
  console and delayed WebSocket protocol sentinels against the serial stream.
- Inline PDF, print to PDF, service workers, and web app installation.
- Profile and session writes previously replaced live files in place. This slice
  stages and syncs each save, retains a previous copy, and recovers it after an
  interrupted promotion. Per-site Shields choices now use the same staged save
  and previous-copy recovery; the QEMU gate cuts power after a toggle and checks
  the choice on reboot. Extension enablement and saved dynamic rules now use
  staged replacement and previous-copy recovery; the extension QEMU gate
  verifies both after a hard reboot. Disposable caches still need review.

Keep browser state and controls within its own capabilities. Do not add a POSIX
filesystem or network path.

The existing browser controls now have an offline in-app shortcut page and a
matching keyboard reference in `docs/apps/kaleidoscope-shortcuts.md`.

Reader view now extracts the article from the currently shown DOM. It keeps
script-added text and avoids a second request for a page that may have changed
or required a sign-in. Direct `about:reader?url=` addresses still fetch once.
Article data tables now retain captions, rows, header/data cells including empty
cells, links, and bounded column spans in reader view. Hidden rows, page styles,
and event handlers stay
out. The reader QEMU fixture shows the table in its screenshot; complex nested
tables and row spans remain gaps.

The browser's own controls now follow SexOS light, dark and high-contrast
appearance and accent changes. Site content keeps its own CSS colors.
