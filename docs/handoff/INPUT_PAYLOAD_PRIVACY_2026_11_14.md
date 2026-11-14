# Keyboard diagnostic privacy boundary

Normal userspace images log keyboard admission/routing metadata without raw
scancodes, normalized characters, modifiers, or HID register payloads. Input,
Shell and USB each emit an explicit `input.payload.mode` startup receipt. The
first-eight generic `pdx_listen_raw` wrapper never prints message arguments.
Input delivery and routing are unchanged. Spindle intentionally echoes commands
only on its focused CLI path; the vault typing audit rejects any such echo in a
protected interval. Other applications and disclosure actions remain responsible
for their own logging.

`sexos_input_payload_proof` explicitly enables unsafe payload diagnostics in the kernel,
Input and Shell. Never enter passwords or private content into that image.
`scripts/build_input_payload_proof.sh` builds/boots through the canonical sealed
entrypoint, freezes the exact diagnostic source/image/runner binding, preserves a
clearly labeled unsafe ISO, restores the normal spec and rebuilds normal artifacts.
Failure restores the original spec and previously staged artifacts. The dedicated
CI diagnostic job runs the unchanged strict physical-key oracles for HTTPS assets,
connected desktop and Browser reserve using only that frozen diagnostic image.
Every other CI job keeps normal build flags. Normal images cannot satisfy these
payload proof gates. Ordinary app Ctrl-chord fences use both key-up admissions
and the actual application ACK without disclosing which keys were released.

`program_chastity_gate.py --input-privacy` requires three explicit normal userspace
receipts and audits focused vault typing intervals for literal fixture passwords,
encoded keyboard/HID payload records, generic argument dumps, and CLI echoes.
It writes `input-privacy.json` while retaining original serial evidence. Raw kernel
PS2 payloads anywhere in that boot, including before protected input, cause failure.
The gate also requires observed redacted kernel records. The diagnostic mode and negative audit
have host regressions; production KeyboardOutbox delivery is exercised for every
raw key 1..127 in both modes, with invariant normal diagnostics.

The initial v3 userspace change left three first-sixteen kernel PS2 raw-scancode
sites unchanged; its clean protected interval could occur after that budget was
consumed. That evidence is preserved below and in its original manifest. The user
subsequently approved the exact logging-only exception: three sites, +9 nonblank
lines, normal metadata-only records and explicit diagnostic payloads. Queue, wake,
interrupt, and capability behavior are unchanged. Full exact-head CI and reconciled
candidate build coverage remain required; this document makes no blanket claim
about every application's logging or all keyboard/privacy boundaries.

Local validation before runtime graft: 1,202 Python regressions passed; 23 normal
and 23 diagnostic production keyboard-outbox tests plus three HID tests passed.
Normal Input/Shell/USB and Chastity guest builds passed forbidden-instruction
scans. Actual local v3 two-boot Chastity gate subsequently passed: probe 6/6,
create, generated entry, TOTP, Lock, wrong-password refusal, persisted reboot
unlock, search, clipboard refusal and exact run retirement. All 12 focused typing
intervals had no userspace keyboard payload/CLI echo. Normal Input/Shell/USB mode
receipts were verified. Zero observed kernel payload lines in those intervals
**does not establish kernel privacy**, because its first-sixteen budget may already
have been consumed. Exact image/kernel/serial/audit digests and scope are recorded
in `INPUT_PAYLOAD_PRIVACY_RUNTIME_2026_11_14.json`.

A separately labeled local UNSAFE diagnostic graft emitted both proof startup
receipts and the original code=35 physical H-key admission, followed by verified
DNS/TCP exchange. The full legacy HTTP gate failed its unchanged body pixel oracle
(rows 11/12/14/15); this is partial diagnostic receipt evidence, not a passing
full gate or sealed source/image proof. Original logs, screenshots and PCAP remain
under `target/input-privacy-runtime/UNSAFE-diagnostic/http-proof`. The dedicated
canonical diagnostic CI build/freeze/gates remain pending.

Normal ELF SHA-256 (source checkpoint accompanying this document):

- Chastity: `e2875a22602f2dac011e2c920561c6f25fa010ecd096c014ee4466dbd1be57bf`
- Input: `8228dc8f39dc720fed5430889f7450443eb61f9ae6df95b9750a5877280e087c`
- Shell: `7d0c9b3602ab40dd61b30945e60e86a08ce29ccd75846a57edb47ebef2407767`
- USB: `7c295169873546c50b4f9a35beac0eaebd5d0017447d6032cc6dfab581abd743`


## Authorized kernel follow-up: actual v4 evidence

Both kernel modes were built with the canonical kernel target and linker flags,
then frozen separately. Normal v4 replaces only the kernel in the already verified
v3 userspace image. Its two-boot Chastity gate passed probe 6/6, all vault operations
and 12 protected typing intervals. Each boot actually emitted all 16 redacted port
records and all 16 redacted enqueue records, with zero raw kernel payload records
anywhere in the boot. This checks the startup budget directly instead of inferring
privacy from its exhaustion. The normal audit also rejects a proof kernel even if
its raw records preceded the protected interval. Ring-full drop was not induced
at runtime; the actual drop-site statements are exercised in both modes by the
separate source regression. All 1,208 Python regressions passed before this
runtime evidence update.

A separate UNSAFE v4 diagnostic boot emitted raw first-key PS2 data and both
userspace proof receipts. Its recorded public `a` sequence joins make/enqueue,
focused CLI ACK, break/enqueue and admitted key-up. The initial small driver timed
out because it incorrectly expected `a` at counter 1; the standard focus helper had
already sent ScrollLock (counters 1/2), so `a` occupied counters 3/4. The original
failure log is retained, and the exact recorded sequence was independently checked
without weakening a production gate. This is bounded public-key diagnostic
qualification, not a passing broad legacy HTTP or sealed-build gate.

Exact source, flags, kernel/image/serial/audit hashes are recorded in
`INPUT_PAYLOAD_PRIVACY_RUNTIME_V4_2026_11_14.json`; the original v3 manifest is
unchanged. Runtime artifacts are under
`target/input-privacy-kernel-artifacts/vault-proof` and
`target/input-privacy-kernel-artifacts/UNSAFE-public-key-proof`. These are local
boot-preserving grafts, not canonical sealed source/image provenance. The final
canonical diagnostic CI build/freeze and full exact-head CI remain pending.
