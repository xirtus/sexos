# Kaleidoscope daily-driver gaps

Compared with Firefox and Safari, Kaleidoscope already has persistent tabs, history,
bookmarks, cookies, downloads with a panel, reader view, private and container tabs,
and Shields. The remaining gaps are:

- A password manager with safe autofill and passkeys, integrated with Chastity.
  The first autofill slice defines and tests exact top-level HTTPS scope and an
  unambiguous same-origin POST login pair. Live consent, fill and cancellation
  are still required before passwords can be used in the browser.
- A visible per-site settings view for Shields and stored data. This change adds a
  first view and controls; permission prompts and a full settings catalog remain.
- Stronger tab isolation, developer tools, extensions, and browser compatibility.
- Inline PDF, print to PDF, service workers, and web app installation.

Keep browser state and controls within its own capabilities. Do not add a POSIX
filesystem or network path.
