# Assuit: retry failed recovery writes

- **Gap:** A failed idle recovery write cleared Assuit's pending timer before Files had promoted a recovery copy. With no later edit, it never retried, leaving DOCX or ODT changes unprotected across a crash.
- **Change:** Keep the pending edit until `write_recovery` succeeds; schedule a bounded five-second retry after an error. If a close-time keep fails, decline the close, show the failure and retry while the window stays open. Ctrl+Shift+Q explicitly discards the unsaved changes. A successful ordinary save still clears the timer.
- **Evidence:** The focused host test makes the real staged recovery promotion fail, confirms the edit stays dirty and no early retry occurs, then retries the same edit without a keystroke. It reopens the recovery copy and saves/reopens DOCX and ODT with text and a linked footnote. The existing two-boot `program_assuit_recovery_gate.py` captures the restored page after a guest power cut.
- **CI:** Run the Assuit host test and the existing QEMU recovery screenshot gate on the exact PR head. Full builds and QEMU run only on GitHub CI.
- **Limits:** The test blocks the recovery destination on the host; it does not fault the Files server itself or prove the power cut lands inside Files' rename. A forced task termination can still bypass cooperative close and retry.
