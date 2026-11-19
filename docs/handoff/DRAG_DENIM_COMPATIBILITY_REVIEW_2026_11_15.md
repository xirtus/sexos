# Drag / Denim compatibility review

Read-only review of actual master 34b1cb3310dd54dded61d0cf0a4a2c9fb8eb97be,
after the previously reviewed raw 23090514 cohort. No kernel source was edited.
This new 141 file kernel tree remains unapproved by the timer source checker.

## Compressed markers accepted as physical frames

`kernel/src/app_resources.rs:517-536`, heap_frames, admits a loan when the live
PTE equals its saved expected value and, for writable loans, has the write bit.
A Denim-compressed page has the same nonpresent out_marker in both locations.
A read-only loan therefore passes and pushes marker & PHYSICAL_MASK into EPT.
For target 42, that produces 0x2a000, rather than a verified owned physical frame.

The agent compiled the exact extracted production heap_frames function with a
mocked authorized live-row/custody fixture. The current function returns that
value for a compressed page. Evidence is retained in the prerequisite worktree:
`target/actual-master-vmx-audit/heap_frames_counterexample.rs`, its log and JSON.
This proves the admission-predicate failure; no VM execution or exploit is claimed.

## EPT loan lifetime survives compression/free

`kernel/src/vmx.rs:205-239` resolves caller heap frames, writes EPT leaves and
retains each (gpa, frame) in Vm.loans. The loan remains until unmap/destroy/revoke;
VM entry does not revalidate the current host heap PTE. In contrast,
`app_resources.rs:830-844`, denim_out, replaces that heap leaf with a compressed
marker and returns the frame to the program allocator without checking VM loans.
The VM can retain access after the frame has been made available for reuse.

The current vmx::lent query protects code sealing, but not denim_out. Main-task
VCPU execution is also distinct from program_workers::all_dead. This is a concrete
source lifetime mismatch, not a reproduced cross-domain exploit.

## Required narrow compatibility repair

1. heap_frames must reject compressed/nonpresent/non-user markers before deriving
   a physical frame, while preserving valid owned read and write permissions.
2. A frame must remain unreclaimable for the entire EPT loan lifetime. Compression
   must defer or refuse loans under an explicitly checked lock/ownership protocol.
3. Review VMS/APPS locking and concurrent lend/compress/unmap/VM entry. A bare
   additional blocking lookup is not an adequate concurrency design.
4. Add actual-production predicate tests and a lifetime/concurrency regression;
   where VMX is available, run a real guest loan/compression/reuse test. Preserve
   the feature-unavailable path as a separate outcome.
5. Only then review the new complete kernel/SDK/USB causal cohort and any actual
   auxiliary wait roles. Never broaden the old raw 140 file cohort or rewrite its
   runtime evidence to claim coverage of this new kernel.

The handoff says **Do not touch the kernel**; the user's existing exception covers
only nine logging lines. These compatibility changes need author ownership or a
new explicit exception. Current Moan, plugin and autofill work can continue on
its separately sealed ba8-based c327 image. PR296's published e7 cohort and CI are
separate from this newer master audit; merge/final-master qualification stays held.
