# SexOS

### A single-address-space Rust microkernel

[Website](https://sexos.dev) · [Architecture](ARCHITECTURE.md) · [Manual](docs/handoff_html/index.html) · [YouTube](https://youtube.com/playlist?list=PLtv6PF9o7eC59PC5cn49Wia-8ogZ6AxUn) · [Releases](https://github.com/xirtus/sexos/releases)

![SexOS](docs/hero.jpg)

SexOS is an experimental operating system built around a **single global virtual address space** instead of per-process page tables. Memory is globally addressable; authority is capability-controlled through `sex-pdx`; Intel PKU is used as an optional hardware enforcement accelerator. The kernel is written in Rust, and the userland is a set of protection-domain servers.

It is not a Linux distribution, not a drop-in replacement for Linux, and not production-ready. It is a testbed for a different OS model: one address space, explicit capabilities, deterministic epochs, and a very small kernel.

## What is in the box

- **GLOBAL_VAS** — one virtual address space for kernel, servers, and processes.
- **sex-pdx** — the capability and communication substrate. All inter-domain access is capability-mediated; there is no ambient authority.
- **PKU enforcement** — protection keys accelerate domain boundaries; the authority model still comes from `sex-pdx`.
- **BOOT_DAG** — static, boot-time capability genesis; immutable once the epoch engine starts.
- **Deterministic execution model** — state is a function of `(epoch_id, input_stream, capability_graph)`; transitions are snapshot → transform → swap → commit.
- **User-space servers** — `sext` (demand pager), `silk-shell`, `sexdisplay`, `silkbar`, `sexfiles`, `sexinput`, `sexusb`, `sexnet`, `sexstore`, `linen`, `quil`, and apps such as `sexdrive`, `kaleidoscope`, and `spindle`.
- **Boot path** — Limine, x86_64 UEFI/BIOS, QEMU; ARM64 port phases 2–6 are in the tree.

The full invariants and server model are in [ARCHITECTURE.md](ARCHITECTURE.md). The generated manual is in [docs/handoff_html/index.html](docs/handoff_html/index.html).

## Status

**Experimental.** The x86_64 image boots under QEMU and reaches the `silkbar` event loop. The ARM64 port is in progress. Syscall/ABI surfaces are unstable, there is no production security audit, and hardware coverage is narrow.

What works today:

- x86_64 QEMU boot from the prebuilt ISO.
- Kernel bring-up, paging/PKU setup, PDX call path, scheduler/yield path, serial console.
- `silkbar` cadence/event loop and the display/input path in the QEMU configuration below.
- A build pipeline sealed to `scripts/entrypoint_build.sh`.

What does not work / is not claimed:

- General-purpose desktop use, Linux binary compatibility, or a stable user ABI.
- Production security or isolation guarantees.
- Broad hardware support.
- Independently reproduced performance comparisons against Linux.

## Quick start (prebuilt ISO)

Download the latest experimental ISO from [Releases](https://github.com/xirtus/sexos/releases), verify it, and boot it:

```
SHA-256 (v0.1.0): 187acaf5fb4664f25751acbcf991f8c8c37c9fc9e1acc4fe3d39f2f90542b801
```


```bash
# v0.1.0 asset name
qemu-system-x86_64 \
  -M q35 -m 512M -cpu max,+pku \
  -cdrom sexos-v0.1.0.iso \
  -serial stdio -display none -no-reboot
```

You should see the boot log and the `silkbar` loop. Press `Ctrl-C` to stop QEMU.

For a graphical QEMU window, put the ISO in the repo root as `sexos-v0.1.0.iso` (or set `SEXOS_ISO=/path/to.iso`):

```bash
./dev.sh run
```

On Apple Silicon macOS, `bash boot_mac.sh` uses TCG emulation and the local ISO.

## Build from source

Requirements: a Linux build environment (CI uses Ubuntu), Rust nightly with the `x86_64-unknown-none` target, `ripgrep`, `sha256sum`, and QEMU for running. The build is intentionally sealed to one entrypoint:

```bash
make full
```

Direct `make iso` and `make run-sasos` are hard-fail stubs; use the entrypoint. See `scripts/entrypoint_build.sh`, `sexos_build_spec.toml`, and `sexos_contract.toml` for the pinned contract.

## Architecture in one page

```
GLOBAL_VAS  →  addressing only
sex-pdx     →  authority: every access decision
PKU         →  hardware enforcement accelerator (optional)
BOOT_DAG    →  capability genesis (immutable after lock)
```

No domain interaction happens outside `sex-pdx`. No capability exists without `BOOT_DAG` sanction. PKU never makes an authority decision; it only enforces one. The kernel traps, forwards, and resumes — it is a transition arbiter, not a memory authority. `sext` owns demand paging and mapping in ring 3.

If you want the longer version, read [ARCHITECTURE.md](ARCHITECTURE.md), then the [manual](docs/handoff_html/index.html).

## Performance claims

The website lists strong comparative claims (for example, “96× smaller than Linux” and “3.5× faster IPC”). Those are project claims, not independently verified results. A reproducible benchmark page with hardware, kernel configuration, workload, raw numbers, and caveats is not yet in this repository. Treat the numbers as hypotheses until that page exists.

## Repository layout

```
kernel/          kernel and arch ports
servers/         sex-pdx servers and shell/display/input stack
crates/          shared crates (sex-pdx, graphics, models)
apps/            user-space apps
docs/            handoff notes, manual, architecture material
scripts/         build and QEMU entrypoints
website/         sexos.dev source
ARCHITECTURE.md  canonical system invariants
```

## Docs and media

- [Architecture](ARCHITECTURE.md)
- [Manual](docs/handoff_html/index.html)
- [Website source](website/index.html) — live at https://sexos.dev
- [YouTube playlist](https://youtube.com/playlist?list=PLtv6PF9o7eC59PC5cn49Wia-8ogZ6AxUn)
- [Handoff notes](docs/handoff/) — implementation history and open lanes

## License

GPL-3.0. See [LICENSE](LICENSE).

## Contributing and security

- Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR.
- Report security issues privately via GitHub Security Advisories; see [SECURITY.md](SECURITY.md).
- The architecture invariants in [ARCHITECTURE.md](ARCHITECTURE.md) are not suggestions. PRs that bypass `sex-pdx`, add ambient authority, or reintroduce per-process page tables will be rejected.
