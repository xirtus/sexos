# Preserved Claude browser-site evidence

This entry indexes historical evidence; it changes no browser behavior and makes
no current compatibility or playback claim. The original worktree was clean at
inspection. Its two commits are absent from master and remain preserved on the
remote branch `claude/site-evidence-2026-11-10`:

- [`a9758ac52e3a983c8e39b813c39762d2e8d4eda3`](https://github.com/xirtus/sexos/commit/a9758ac52e3a983c8e39b813c39762d2e8d4eda3): QEMU site results, compressed serial logs, screenshot and fetched pages.
- [`faf15924a677b9f0265ccd518417f98e73a2a535`](https://github.com/xirtus/sexos/commit/faf15924a677b9f0265ccd518417f98e73a2a535): additional stylesheet, module and film-asset snapshots.

No OS source, build script or gate fix exists in those commits. The combined
addition is 162 files / 22,159,563 bytes: 120 downloaded page/asset files account
for 20,295,180 bytes; `tls1` and `tls2` account for 1,747,997 bytes; the remainder
is the README and screenshot. The largest file is a 9,726,297-byte YouTube script.
The assets remain on their original branch; this entry does not import them.

## Evidence limits

The original README explicitly says the standalone page snapshots were fetched
with curl using a desktop Chrome User-Agent. They may differ from responses to
SexOS, and must not be presented as SexOS response or compatibility proof. No such
fetch was repeated during this assessment. Future live tests must use the actual
browser request identity and retain server responses without spoofing headers.

The 19 `result.json` files contain site, elapsed time, selected serial lines,
TLS/HTTP observations, and loaded/error flags. They do not bind those observations
to an ISO hash, Web hash, exact source revision, complete runner hash, or a sealed
build manifest. Their `loaded` flag can establish only what that historical
runner checked; it does not establish rendering fidelity, interaction, JS
correctness, video playback or current-master behavior. The screenshot has the
same missing build provenance.

`evidence/tls1/sexos_dev/serial.log.gz` is zero bytes and provides no serial proof.
Every nonempty compressed serial file decompresses successfully. Google's two
historical runs end with `[serial.record.overflow] panic diagnostic-too-long`;
that is a retained failure signal, not a diagnosis of its underlying cause or a
current-master reproduction. Keep failed and timed-out records alongside successes.

## Retrieval and follow-up

The original clean worktree remains at
`/private/tmp/claude-501/-Users-xmacbookm5-Projects/344cb972-00c7-415c-a10c-87274a55d7af/scratchpad/wt`.
A separately generated path/size/SHA256 inventory is retained at
`/Users/xmacbookm5/Projects/sexos-evidence/claude-site-evidence-faf15924/inventory.json`
(SHA256 `78af64c8af1aced4cf9f5b2e70cdf3c01f1c26c0113205e8da61819e101a8c03`).
The remote commits are the durable retrieval source; the inventory contains no
copied third-party asset bodies.

Use these logs to select small real regression cases. Any claimed fix needs a
fresh coherent ISO/Web pair with hashes, the actual unmodified response, raw
serial evidence, and a gate that checks the affected behavior. Do not merge the
bulk snapshots merely to preserve an already committed diagnostic corpus.
