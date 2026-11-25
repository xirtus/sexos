# Kaleidoscope separate site data controls

- Gap: The site settings view counted cookies and local storage but only offered
  one all-data clear. A person could not sign out of a site by clearing its
  cookies while keeping local drafts, or clear local storage without losing a
  session.
- Change: In native site settings, Ctrl+Alt+C clears only cookies and
  Ctrl+Alt+S clears only local storage for the shown site in the active tab's
  container. The existing full clear still removes both and saved code. Web
  content cannot invoke these browser keyboard actions. The chosen data file
  is staged and synced immediately; its pre-clear recovery copy is retired so
  damage to the live file cannot restore data the person explicitly cleared.
- Proof: The shared `site_data::clear` host test covers parent-domain cookies,
  an unrelated site, another container, and a different port's storage. The
  browser privacy QEMU gate checks the intermediate counts and captures
  `site-cookies-cleared.ppm` before clearing storage.
- Full image build and guest QEMU proof run only on GitHub CI.
