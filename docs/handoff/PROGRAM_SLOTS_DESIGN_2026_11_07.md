# Running programs outside the boot catalog: design (2026-11-07)

Goal: put a new program on a SexOS machine and run it, without rebuilding the
image or adding code to Shell, Display and the kernel for that program.

## What stops this today

1. **Every program is a catalog row.** `app_catalog::INCARNATIONS` holds 28 fixed
   rows of three apps (Quil, Spindle, Browser). Shell alone has 31 matches on the
   app kind. Each row's image is mapped at BOOT from a Limine module.
2. **A program can only run once per row.** An incarnation retires permanently;
   a "reopen" activates the next reserved row. Seven reopens means seven rows.
3. **The hardware keys are spent.** x86 PKU has 16 keys. Key 0 is the kernel and
   domains 1 to 15 take the rest, so there is no key for a new program. Successor
   incarnations reuse their app's key, which works only because at most one runs.

The third is the deepest. It limits any single-address-space design on PKU, so
solving it is core work, not a workaround.

## Design

### Program slots

A fixed BOOT pool of N generic domains. Each holds, from BOOT and for its whole
life: a Display route, a Shell route, a text region, a window frame, and a
reserved block of physical frames. Rights differ by slot profile, for example
with or without network. The capability graph stays immutable after BOOT, which
keeps ARCHITECTURE.md's rule that BOOT_DAG is the only capability genesis: a
running program gets exactly what its slot was granted, and nothing is minted.

Unlike an incarnation, a slot is reusable:

    Free -> Loading -> Running -> Exited -> Free

- **Launch.** Shell holds a BOOT capability per slot. It names an image and the
  kernel validates the ELF, maps segments from the slot's reserved frames with
  W^X permissions, resets the main task and starts it.
- **Reclaim.** After exit, the kernel unmaps and zeroes the frames, drains rings,
  and returns the slot to Free. Authority never changes, so nothing is revoked.
- **Images.** First from Limine modules, listed with `programs`, run with
  `run NAME`. Then from storage, so installing means copying a file.

### Shell and Display know programs by slot, not by kind

Add one catalog kind, `App::Program`. Shell's matches then cover what a program
is, not which program it is. No per-program code remains in the servers.

### Virtual protection keys

Give domains virtual keys and map them onto hardware keys on demand:

- Reserve one hardware key as the parking key, denied in every PKRU.
- A domain holding no hardware key has its pages tagged with the parking key.
- At dispatch, a domain with no hardware key takes the least recently used one:
  retag the evicted domain's pages to the parking key, retag the incoming
  domain's pages to the freed key, invalidate those TLB entries.
- Services that talk constantly stay pinned so they never pay for this.

Authority is unaffected: sex-pdx still decides, and PKU still only enforces, as
ARCHITECTURE.md requires. Cost is per page on eviction, so measure it, and keep
pinned keys for the desktop's hot path.

## Order of work

1. Program ABI and a small SDK crate, plus `hello` as a Limine module.
2. Kernel slot lifecycle and `App::Program`, with one slot and a free key by
   sharing a key among services that are trusted together, as an interim step.
3. Shell launch and reclaim, `run` and `programs` in Spindle.
4. Virtual keys, then more slots.
5. Images from storage.

Acceptance: `run hello` shows a window, exit frees the slot, and running it 100
times leaks nothing. A hostile test program cannot read another domain's memory.

## Open risk

Step 2's interim key sharing weakens isolation between the two sharing services
until step 4 lands, so it must be named in the ledger and removed by step 4.

## Update, 2026-11-07: what building it showed

**Agreed direction.** Andreas proposed organs: a hardware key belongs to an organ
system, a group of tasks, not to one program. Thousands of tasks can share about 11
systems. Very few parts are vital; everything else must restart after a crash.
Activity-shared keys are only the fallback for untrusted native programs.
Measured on QEMU: 5 domains run per second at idle, 7-8 at peak while typing.

**Order changed.** Organ keys wait until programs run side by side. Before that,
sharing keys weakens isolation and gains nothing, because only one program runs at
a time and every call still enters the kernel.

**Why a reusable slot is not a small change.** Identity is immutable by design:

- `LifecycleGrant` holds the target and task in each service's capability, and
  `authorize` compares against `app.task`. A new task id breaks every holder.
- `ServiceCustody.retired` records one notice per row, forever. Display and Shell
  keep tombstones per surface. All assume a row dies once.
- `retire` revokes capabilities permanently and gives the frames to sext's pool.
- `Task` has one life: `Constructed` to `Dead`, with a boot-allocated kernel stack
  and a sealed self-wait inventory that holds its pointer.

**Proposed design.** Add a generation to identity, so (target, generation) replaces
"target died once":

1. Kernel: `App` gains `generation`. Grants keep their immutable target and rights
   but authorize against the row's current task and generation. For program rows
   `retire` keeps capabilities and frames, and zeroes the frames itself.
2. Kernel: a `rebirth` activation kind, allowed only after every service has acked
   and cleanup is complete. It reloads the ELF with W^X, resets the same Task object
   with a fresh task id, then admits it.
3. Services: `ServiceCustody` and Shell's `app_custody` learn the new generation
   and clear their per-row record only on a kernel-attested rebirth. Display clears
   the surface tombstone the same way. Old messages carry the old task id and stay
   fenced.
4. Gate: run hello 100 times, leak nothing, and show a message from an old run is
   refused.

Open question: whether the same generation should later replace reserved rows for
Quil, Spindle and Browser. It would remove the fixed reopen counts and shrink the
catalog, so it probably should.
