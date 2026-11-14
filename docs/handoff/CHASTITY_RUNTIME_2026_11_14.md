# Canonical Chastity and live autofill runtime evidence

Both full QEMU gates pass with final guest source b9241079 (canonical37471138 reused,
reviewed dependency patches and Close/scroll/focus-driver fixes included). This is
local native runtime proof; combined remote CI must still be green before merging.

| Artifact | SHA256 |
| --- | --- |
| Chastity std ELF | c00775e49cb73995e25c728b41dfe9d48b9f44dcf20b8bf515d58ced6fd5c19f |
| Separate consumer ELF | e0fc74523fdc6ff8cea8fcfc262691ded144c6e1b24b1df6085263c32e8d1524 |
| Real Shell/Files v2 ISO | 4cb7e33127e88bb44d9b2f13139ddc41f58c4db80a41b56b5f8bd298286547b5 |

The independently verified ISO preserves the original kernel, Limine configuration,
BIOS/UEFI boot entries and GPT while grafting the actual updated userspace Shell and
Files. Each gate copies an all-zero seed disk; external source artifacts are read-only.
The ISO lives at /Users/xmacbookm5/Projects/sexos-plan/target/chastity-runtime/sexos-autofill-v2.iso.

Canonical gate: `scripts/program_chastity_gate.py`; local artifacts under
/Users/xmacbookm5/Projects/sexos-chastity/target/chastity-proof-v2. Probe6/6;
Argon2id32MiB/4pass save1123ms/open1238ms. Create/add/generated password/TOTP,
Lock acknowledgement, wrong-password refusal, reboot with the saved entry, search,
honest clipboard refusal, and exact app.private.retired Close receipt all pass.
Screenshots show the actual maximized shared Weft screen.

Live gate: `scripts/chastity_autofill_gate.py`; local artifacts under
/Users/xmacbookm5/Projects/sexos-chastity/target/chastity-live-proof-v2. A real separate
consumer43/43 is denied provider impersonation and receives nothing before consent.
Token1 defaults to Deny; token2 receives exactly one expected fictional login after
clicking the actual visible Allow control. Restarted consumer43/1073741824 receives
fresh token3 consent and is denied. Provider Lock yields consumer Unavailable.
The complete master password and credential do not appear in serial output.

The screenshot follow-up waits for a fresh `display.focus.drawn` receipt before
capturing consent (Shell focus alone can precede compositor repaint). Its final
reviewable captures live in target/chastity-live-visible-proof-v2. No app authority
or credential-handling code changed for this evidence timing fix.

Host:26 canonical core/UI tests,17 unchanged production broker/SDK/native-wrapper
tests. The native wrapper tests include full intake, transport Full retries,
Close tombstone/retirement, reused row and old-token/epoch replay. Both binaries pass
scripts/key_bytes.py. These tests do not promise complete compiler/allocator memory
erasure; the documented dependency and caller custody limits still apply.
