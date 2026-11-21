# 2026-11-21 Kaleidoscope privacy defaults (workstream N13a)

- Third-party cookies blocked while shields are up: requests, set-cookie, media and
  WebSocket only carry cookies when the target is same-site (registrable domain via
  psl) as the top page; cross-site frames see no `document.cookie`. Shields down
  restores them.
- HTTPS-first: top-level `http://` loads to public hostnames try HTTPS first
  (`[web.https.upgraded]`); if that fails, an interstitial offers
  `about:http?url=` to continue for that host (`[web.https.allowed]`). IPs,
  localhost, single-label and .local/.internal/.lan/.home.arpa hosts are not upgraded.
- Global Privacy Control: every request sends `Sec-GPC: 1`;
  `navigator.globalPrivacyControl` is true.
- Gate: `scripts/program_web_privacy_gate.py` (CI).
