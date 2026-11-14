# Keyboard diagnostic privacy boundary

Normal userspace images log keyboard admission/routing metadata without raw
scancodes, normalized characters, modifiers, or HID register payloads. Input,
Shell and USB each emit an explicit `input.payload.mode` startup receipt. The
first-eight generic `pdx_listen_raw` wrapper never prints message arguments.
Input delivery and routing are unchanged. Spindle intentionally echoes commands
only on its focused CLI path; the vault typing audit rejects any such echo in a
protected interval. Other applications and disclosure actions remain responsible
for their own logging.

`sexos_input_payload_proof` explicitly enables unsafe payload diagnostics in Input
and Shell only. Never enter passwords or private content into that image.
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
It writes `input-privacy.json` while retaining original serial evidence. Kernel
payload records are reported separately. The diagnostic mode and negative audit
have host regressions; production KeyboardOutbox delivery is exercised for every
raw key 1..127 in both modes, with invariant normal diagnostics.

**Full keyboard privacy is not established by this userspace change.** The
unchanged kernel has three first-sixteen PS2 raw-scancode diagnostic sites. A
clean runtime interval may merely occur after that budget was consumed; it cannot
establish a safe kernel policy. Vault/full release remains blocked on separately
authorized kernel source review and evidence. No kernel file was changed here.

Local validation before runtime graft: 1,202 Python regressions passed; 23 normal
and 23 diagnostic production keyboard-outbox tests plus three HID tests passed.
Normal Input/Shell/USB and Chastity guest builds passed forbidden-instruction
scans. Actual v3 vault typing evidence is pending, not claimed by these host tests.

Normal ELF SHA-256 (source checkpoint accompanying this document):

- Chastity: `e2875a22602f2dac011e2c920561c6f25fa010ecd096c014ee4466dbd1be57bf`
- Input: `8228dc8f39dc720fed5430889f7450443eb61f9ae6df95b9750a5877280e087c`
- Shell: `7d0c9b3602ab40dd61b30945e60e86a08ce29ccd75846a57edb47ebef2407767`
- USB: `7c295169873546c50b4f9a35beac0eaebd5d0017447d6032cc6dfab581abd743`
