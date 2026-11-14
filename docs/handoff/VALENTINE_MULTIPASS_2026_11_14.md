# Multipass, Valentine and Flirt: state and next steps (2026-11-14)

Workstream Y of `docs/handoff/APP_PARITY_PLAN_2026_11_14.md`. Asked for by Andreas: his
Leeloo Multipass ([app](https://github.com/xirtus/leeloo-multipass),
[SDK](https://github.com/xirtus/leeloo-multipass-sdk), both MIT) nativized as the system
accounts service, and Valentine as one inbox for mail, Matrix, IRC, XMPP, the fediverse and
Discord, with Flirt as the voice and video side (Mumble, Matrix calls, Jitsi).

## What landed in this PR

| Piece | Where | Gate |
|---|---|---|
| Shibari, the RAM-only secret keeper (owner-tied knots, use-in-place, wipe on untie) | `crates/shibari` | unit tests |
| Multipass core: address rules, public-URL rule, stamp book, expiring single-use flows | `crates/sex-multipass` | ported leeloo goldens (address namespaces, private destinations, proof expiry) |
| Linking: Mastodon OAuth+PKCE, Matrix SSO (with `.well-known` discovery), Bluesky handle→DID→PDS→PAR with PKCE and DPoP (nonce retry, `iss` check, `sub` check), fediverse bio proof by WebFinger | `sex-multipass/src/flows.rs` | `tests/link_fixtures.rs`: each fixture links an account |
| Act-for-account grants: an app asks Multipass to send a request to the account's own server; the token is added inside Shibari's closure | `sex-multipass/src/service.rs` | a program asking for the raw token is refused; tokens never reach another host; read grants cannot write |
| DAV app-password accounts (CalDAV/CardDAV/WebDAV: generic, Nextcloud, Fastmail, iCloud) for Date and Crush | `Multipass::add_server_account` | password checked with PROPFIND, REPORT with Depth through a grant |
| Mail core for Valentine: IMAP4rev1 + IDLE + SASL-IR, SMTP submission with AUTH and dot-stuffing, RFC 5322/MIME (encoded words, QP, base64, multipart, attachments, remote-content detection), threading | `crates/sex-mail` | `tests/send_then_receive.rs` against `sex_mail::fixture`, an in-memory IMAP+SMTP server |
| PDX protocol for apps | `crates/sex-pdx/src/accounts.rs` (op `0x2B0`) | word round-trip test |

All three crates are `no_std` + `alloc` and build for `x86_64-unknown-none`.

## What changed from Leeloo Multipass to fit SexOS

- Fastify routes → the PDX protocol in `sex_pdx::accounts`. PostgreSQL tables → the stamp
  book as a plain file in Files (it holds no secrets) and RAM-only flow state.
- Sealed tokens in the database → Shibari. Tokens are never written to disk. After a
  reboot verified stamps remain and apps ask the person to sign in again, until Chastity's
  vault (R7) can keep refresh tokens encrypted.
- The callback is not a web route: the authorize URL opens in Kaleidoscope and a navigation
  to `multipass://callback/<provider>` is handed to Multipass. The pending flow is keyed by
  `state` and tied to the person who started it (Leeloo used an httpOnly cookie).
- Leeloo only verifies and signs out. Multipass asks: **Verify only** (signs out / revokes,
  as before) or **Keep signed in for apps** (wider scopes, token kept in Shibari).
- The SDK's "the app receives no credentials" becomes the act-for grant. For mail, which is
  not HTTP, Multipass will run the SASL step and hand the app the signed-in flow
  (`OPEN_MAIL_FLOW`), which is why `sex-mail` keeps authentication as a separate call.
- One deliberate fix: Leeloo's fediverse pattern read `https://host/@x` as the account
  `https://host/` on server `x`. URLs are now always profile URLs.

## Discord: Yapfast

Andreas pointed to [Yapfast](https://github.com/yapfast/yapfast). Checked: it is
GPL-3.0-or-later Rust, so it can be linked into GPL-3 SexOS programs. Its workspace has
`yapfast-core` and `yapfast-discord` (the network layer, each network an optional feature,
Matrix too) and a Slint+Skia UI outside the workspace. Its voice path vendors libwebrtc
(C++), DAVE end-to-end encryption, x264 and ffmpeg; its desktop feature pulls zbus,
Wayland, X11 and tray crates that SexOS does not have.

Plan for Y8: port `yapfast-core` + `yapfast-discord` behind the `sex-chat` trait (Y5) and
draw it with Weft instead of Slint; voice goes to Flirt (libwebrtc confined in its own
organ, or webrtc-rs to compare). The Dorion/GoofCord way, Discord's web client in a
Kaleidoscope view, stays as the fallback. Discord's terms forbid third-party clients, so
both stay off until the person turns them on.

## Next slices

1. **Y1b** `servers/multipass`: boot-catalog row, slots and region for `sex_pdx::accounts`,
   sex-net/TLS transport behind `http::Transport`, entropy and clock from their sources,
   Shibari as its own organ (or a library inside Multipass first), `multipass://` handoff
   from Kaleidoscope, the Tatami page (stamp book: navy cover, pastel stamps, Geist),
   and a QEMU gate: Mastodon fixture links an account, a program asking for the token is
   refused. Bluesky needs SexOS's client metadata published at
   `https://sexos.org/multipass/client-metadata.json` (`flows::BLUESKY_CLIENT_ID`).
2. **Y2** adapters: mail accounts (IMAP/SMTP app passwords, OAuth2 XOAUTH2 for Gmail and
   Outlook) with `OPEN_MAIL_FLOW`; Misskey MiAuth, IndieAuth, Nostr signer, WebDAV/S3,
   Mumble certificates. ActivityPub signed fetch needs an RSA key behind `flows::HttpSigner`.
3. **Y3b** the Valentine program (needs Weft): folders, threads, reader with sex-html and
   remote content blocked (`Message::has_remote_content`), compose, attachments through
   Loom; JMAP; search through the E4 index; IMAP modified UTF-7 mailbox names.
4. Y4 to Y11 as in the plan. Date and Crush (workstream U) use DAV accounts through
   `ACT` grants already.
