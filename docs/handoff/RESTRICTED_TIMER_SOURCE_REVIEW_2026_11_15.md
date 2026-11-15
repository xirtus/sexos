# Restricted-provider timed-wait source review

Reviewed integration `dd22ccb5c` against `154816f76` before changing the accepted
source profiles. The SDK root adds four module exports only. Its syscall/clock
wrappers are unchanged. Net adds early deny-journal initialization, a bounded
provider poll before ordinary work, quarantine cleanup, restricted intake and
reply filtering. A pending policy transaction participates in the existing
pending predicate, so the same empty try-listen branch polls again after two
ticks. The before/wait/after clock envelope, sampled receipt, 100-tick alternate
wait, BOOT task identity, and USB quiet predicate remain unchanged.

The kernel tree and all named deadline owners remain the already reviewed root
cohort. No newer master kernel is admitted. This is a narrow source-contract
review, not a general unsafe-code verifier or a new runtime qualification.

Accepted new source hashes:

- SDK root: `c2895d4991c2d48d099daee3e6090063b3288f77c0ad5e9a75201c903f95c984`
- Net main: `d7526adf2002e45afd5ad986808e65e7eee3d5f793f718e62f8893fef51ae10d`

The separate passive Shell observer profile remains unchanged until the final
native coordinator hooks have been reviewed. Existing runtime receipts retain
their original source and image bindings; this review reassigns none of them.

Validation: 29 source-contract, log and ACPI tests pass. The first test invocation
omitted the tests directory from Python's import path; its import failure is
preserved separately. The corrected invocation log is
`/tmp/sexos-root-restricted-timer-source-review-v2.log`.
