# SEXOS ARCHITECTURE.md — SASOS CANONICAL REFERENCE

## 0. System Model

SexOS is a **Single Address Space OS (SASOS)**.

- ONE global virtual address space (GLOBAL_VAS)
- NO per-process page tables
- Memory is globally addressable
- Memory access is capability-controlled — NOT paging-isolated

### Memory Model Hierarchy (System Invariant — Never Violate)

```
GLOBAL_VAS  →  defines ADDRESSING only (what can be named)
sex-pdx     →  defines AUTHORITY only  (what can be accessed)
PKU         →  defines ENFORCEMENT     (hardware acceleration of sex-pdx boundaries)
```

**This is a strict layered invariant, not a description:**
- GLOBAL_VAS does NOT imply visibility or authority
- sex-pdx is the sole source of access authority — PKU never is
- PKU is an optional hardware accelerator for sex-pdx enforcement
- PKU being absent does NOT weaken the authority model — sex-pdx enforcement still holds
- No layer may substitute for or override the layer above it

### Precedence Chain (System Invariant — Single Source of Truth)

```
BOOT_DAG  →  capability genesis   (static, boot-time only — immutable after lock)
sex-pdx   →  runtime authority    (all access decisions, enforced at every invocation)
PKU       →  enforcement accel    (hardware acceleration of sex-pdx decisions only)
```

Rules (never override):
1. BOOT_DAG defines what capabilities MAY exist. No capability exists without BOOT_DAG sanction.
2. sex-pdx makes ALL runtime access decisions. No other layer may override or substitute.
3. PKU never participates in authority decisions. Absent PKU = same authority model, softer enforcement.
4. This chain is enforced at every capability invocation. No shortcut paths exist.

### Determinism Invariant (System Invariant — Never Violate)

> All system state is a deterministic function of:
> `f(epoch_id, input_stream, capability_graph)`
>
> **Closure conditions:**
> - No external I/O influences state except via `input_stream`
> - No hidden hardware state affects output except as explicit snapshot input
> - No hidden state exists — all state is derivable from the above triple
> - Replay from epoch_id=0 with same inputs must produce identical state

All state transitions are: **snapshot → transform → swap → commit**.
No in-place mutation outside epoch boundaries. Execution is fully replayable.

---

## 1. Core Architecture

### 1.1 sex-pdx — Global Capability + Communication Substrate

sex-pdx is NOT just IPC.
sex-pdx IS the entire OS execution graph connectivity model:

- Sole authorization layer for all inter-domain access
- Routing substrate for all domain communication
- Enforcer of the capability graph
- Provider of all domain interaction primitives:
  - `pdx_call`     — invoke a capability target
  - `pdx_reply`    — complete a synchronous exchange
  - `pdx_listen`   — receive inbound capability invocations
  - `pdx_spawn_pd` — instantiate a new protection domain (BOOT_DAG gated)

No domain interaction occurs outside sex-pdx. No ambient authority.

### 1.2 sext — Demand Pager (User-Space Fault Resolver)

sext is a Ring-3 protection domain server.
sext is NOT part of kernel memory management.
sext is NOT a Linux-style paging subsystem.

**Kernel role boundary:** The kernel is a transition arbiter only — not a memory
authority. It traps the fault, forwards it via sex-pdx, and resumes execution.
It does NOT allocate frames, does NOT map memory, does NOT own GLOBAL_VAS.

sext responsibilities:
- Receive page fault dispatch from sex-pdx
- Allocate physical frames on demand
- Map frames into GLOBAL_VAS
- Apply PKU keys where applicable
- Return capability-backed memory mappings

**Fault flow (canonical):**
```
CPU #PF exception
  → kernel trap (transition arbiter — ONLY ring-0 component)
  → kernel/src/ipc/pagefault.rs::forward_page_fault()
  → sex-pdx dispatch → sext (slot 2)
  → sext (ring-3): frame allocator
  → GLOBAL_VAS::map_pku_range()
  → PKU key assignment (enforcement layer — see ARCHITECTURE.md §0)
  → resume faulting domain
```

Frame allocation and GLOBAL_VAS mapping are sext's exclusive responsibility.

### 1.3 silk-shell — Execution Orchestration Layer

silk-shell is NOT a service.
silk-shell IS part of the execution topology:

- Runtime orchestration layer
- Capsule lifecycle manager (spawn, suspend, resume, destroy)
- Execution composition system
- Domain execution entry point for interactive sessions

silk-shell is a node in the capability graph, not a peripheral utility.

### 1.4 UiIR — Userland Transport Primitive

UiIR is the shared-memory frame protocol between execution domains and the compositor.

- Shared-memory ring buffers with epoch-gated visibility
- Fully ordered per-sender causal streams
- No cross-epoch mutation visibility
- Used by: silk-shell → SexCompositor → SexDisplay pipeline

UiIR is NOT an IPC mechanism. It is a data transport substrate.
Authority for UiIR ring access is granted via sex-pdx capability.
UiIR ring contents constitute part of the `input_stream` in the determinism invariant.

### 1.5 SMP Model

Each core is an independent execution lane.
Ordering defined by epoch boundaries + per-sender causal streams.
No global scheduler in early phases — epoch transitions are explicit.

### 1.6 Boot System (BOOT_DAG)

Static dependency graph resolved at boot.
Domains spawned via `pdx_spawn_pd` — gated by BOOT_DAG.
Capability graph is immutable once epoch engine starts.
**BOOT_DAG is the ONLY authority for capability genesis.**

### 1.7 Security Model

All access is capability-mediated. No ambient authority.
Authority model: §0 Memory Model Hierarchy + Precedence Chain.
Full traceability via capability graph + epoch log.

---

## 2. Userland Servers

| Server        | Role |
|---------------|------|
| sext          | Demand pager — fault-resolution memory authority |
| silk-shell    | Execution orchestration — capsule lifecycle + composition |
| SexCompositor | Deterministic frame graph reducer (UiIR consumer) |
| SexDisplay    | Buffer ownership + atomic scanout |
| Linen FM      | Causal filesystem view |

All communicate via sex-pdx capability invocations + UiIR transport rings.

---

## 3. Hard Constraints

- NO per-process page tables
- NO Linux-style process model
- NO ambient authority — all access via sex-pdx capability
- NO nondeterministic IPC ordering
- NO in-place rendering mutation outside epoch boundaries
- NO implicit kernel policy engines
- NO hidden state — all state derivable from (epoch_id, input_stream, capability_graph)
- NO drivers, filesystems, protocol stacks or firmware interpreters in the kernel; NO Linux-shaped syscalls (fork, pipes, fds, sockets). Compatibility layers are libraries; Linux drivers are temporary ring-3 bridges with a tracked native replacement. See `docs/DESIGN_GUARDRAILS.md` and `docs/BRIDGE_LEDGER.md`.

---

## 4. Repository Layout

```
kernel/              — ring-0 transition arbiter only (trap + forward)
crates/sex-pdx/      — global capability substrate (userland-facing ABI)
crates/sext/         — demand pager server (ring-3, PDX-dispatched)
servers/silk-shell/  — execution orchestration layer
servers/sexdisplay/  — scanout controller + compositor
apps/linen/          — filesystem view
```

---

## 5. Capability Slot Table

| Slot | Domain     | Role |
|------|------------|------|
| 0    | kernel     | Bootstrapped capability bridge — sex-pdx mediated, no ambient authority granted |
| 1    | sexfiles   | VFS |
| 2    | sext       | Demand pager — fault resolver |
| 3    | sexnode    | Node translator |
| 4    | sexnet     | Network manager |
| 5    | SexDisplay | Compositor + scanout |
| 6    | silk-shell | Execution orchestration entry |

Slot 0 is NOT an escape hatch from the capability model. All slot-0 invocations
remain sex-pdx mediated. The kernel does not grant ambient execution authority.

---

## 6. Execution Roadmap

**M1 — Silicon Bootstrap**
DRM init, framebuffer bind → purple screen proof

**M2 — ABI Coherence**
Flat atomic packets, cache flush + fence barriers, triple-buffer swapchain

**M3 — SFDP Reactor**
VBlank-driven pipeline: acquire → build_frame → admit → raster → commit. UiIR ring active.

**M4 — Recovery Layer**
GPU hang detection, last-good-frame latch, safe rebind pipeline

**M5 — Interaction Lattice**
UiIR producer (hello-uiir), hardware cursor, hit-test pipeline, silk-shell capsule spawn

**M6 — Semantic Unification**
Shared-memory UiIR ring, multi-node scene graph, Wayland interop

---

## 7. Success Criterion

System is valid when a UiIR rectangle moves across the screen for 60 seconds at
native refresh rate (60–360 Hz) with:
- zero tearing
- deterministic frame drops (epoch-bounded)
- zero undefined rendering state
- perfect latch recovery on forced GPU interruption
- full epoch replay from `f(epoch_id=0, input_stream, capability_graph)` → identical output

---

## 8. Build Authority (Sealed)

Build execution authority is sealed to a single deterministic path:

- `sexos_build_spec.toml` — single declarative build specification of truth
- `scripts/entrypoint_build.sh` — only valid build root
- `scripts/sexos_build_trace.sh` — linear interpreter of the build spec

Legacy direct routes (`build_payload.sh`, direct `make iso`/`run-sasos`) are invalid and must fail.
