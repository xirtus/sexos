# SPDX-License-Identifier: GPL-3.0-only
# PKU gate stub template (kernel redesign Phase 6, docs/handoff/PKU_GATES_STUDY_2026_11_18.md).
# One stub per (caller C, callee E, entry). scripts/gen_gate_stub.py assembles this with
# different values for each symbol below and records where each one lands, so the Rust
# generator (crates/sex-gate) only patches fields; it never assembles.
#
# Symbols the generator fills:
#   MATRIX   u64  address of this stub's matrix byte (bit 0 = allowed)
#   STUBID   u32  nonzero stub id
#   PKRU_G   u32  frame key writable, every other key disabled
#   PKRU_E   u32  callee keys (frame key read-only)
#   PKRU_C   u32  caller keys (frame key read-only)
#   CPAGE    u32  area offset of the task's C slot page (C key)
#   EPAGE    u32  area offset of the task's E slot page (E key)
#   EMASK    u64  bit of E in the active-domain mask
#   ENTRY    i32  rel32 to the callee entry
#   AREAS    u64  base of the per-task gate areas
#
# Task identity: the kernel loads IA32_TSC_AUX with the running task's gate-area index
# on every switch. RDPID reads it in one instruction, ring 3 cannot write it, and it is
# per task, so a migration can't make it name another task. The area is
# AREAS + (index << 18): 256 KiB of address space, mapped sparsely. The area address is
# derived afresh (AREA macro) right after every WRPKRU check, so a jumper that lands on
# a WRPKRU never reaches a memory access with an R11 it chose.
#
# Frame page at the area base (frame key; read-only to every domain, writable only under PKRU_G):
#   0x00 CUR (u32 PKRU of the current domain)  0x08 DEPTH  0x10 ACTIVE
#   0x40 FRAMES[32] (u64 stub ids)             0xFF0 scratch stack top (RSB fill)
# Slot page of domain D (D's key): 0x00 pending RSP, 0x08 pending stub id,
#   0x10 rbx, 0x18 rbp, 0x20 r12, 0x28 r13, 0x30 r14, 0x38 r15, 0x40 D's callee stack top.
#
# Rule (study section 4.2): every WRPKRU writes an immediate and is checked against it;
# after PKRU_G only the fixed path runs; after PKRU_E the frame page must show this stub
# on top before the direct call; after PKRU_C the frame page must show C current and the
# C slot must hold this stub's pending call, and RSP comes only from that slot.

.intel_syntax noprefix
.macro AREA
    movabs r10, AREAS
    rdpid r11
    shl r11, 18
    add r11, r10
.endm
.text
.globl gate_stub
gate_stub:
    # 1. Caller's keys, nothing changed yet.
    movabs r10, MATRIX
    test byte ptr [r10], 1
    jz .Ldeny
    AREA
    cmp qword ptr [r11 + CPAGE + 0x08], 0
    jne .Ldeny
    mov [r11 + CPAGE + 0x00], rsp
    mov [r11 + CPAGE + 0x10], rbx
    mov [r11 + CPAGE + 0x18], rbp
    mov [r11 + CPAGE + 0x20], r12
    mov [r11 + CPAGE + 0x28], r13
    mov [r11 + CPAGE + 0x30], r14
    mov [r11 + CPAGE + 0x38], r15
    mov qword ptr [r11 + CPAGE + 0x08], STUBID
    mov r8, rdx
    mov r9, rcx

    # 2. Frame key only.
    mov eax, PKRU_G
    xor ecx, ecx
    xor edx, edx
.globl gate_wrpkru_1
gate_wrpkru_1:
    wrpkru
    cmp eax, PKRU_G
    jne .Lfault
    AREA
    cmp dword ptr [r11 + 0x00], PKRU_C
    jne .Lfault
    mov rax, [r11 + 0x08]
    cmp rax, 32
    jae .Lfault
    movabs r10, EMASK
    test [r11 + 0x10], r10
    jnz .Lfault
    or [r11 + 0x10], r10
    mov qword ptr [r11 + 0x40 + rax*8], STUBID
    inc rax
    mov [r11 + 0x08], rax
    mov dword ptr [r11 + 0x00], PKRU_E

    # 3. Scrub everything but the arguments (rdi, rsi, r8 = rdx, r9 = rcx).
    xor ebx, ebx
    xor ebp, ebp
    xor r10d, r10d
    xor r11d, r11d
    xor r12d, r12d
    xor r13d, r13d
    xor r14d, r14d
    xor r15d, r15d
    cld
    vzeroall

    # 4. Callee keys, then prove step 2 ran for this stub before entering.
    mov eax, PKRU_E
    xor ecx, ecx
    xor edx, edx
.globl gate_wrpkru_2
gate_wrpkru_2:
    wrpkru
    cmp eax, PKRU_E
    jne .Lfault
    AREA
    cmp dword ptr [r11 + 0x00], PKRU_E
    jne .Lfault
    mov rax, [r11 + 0x08]
    test rax, rax
    jz .Lfault
    cmp qword ptr [r11 + 0x38 + rax*8], STUBID
    jne .Lfault
    mov rsp, [r11 + EPAGE + 0x40]
    mov rdx, r8
    mov rcx, r9
    xor r8d, r8d
    xor r9d, r9d
    xor eax, eax
    .byte 0xE8
    .long ENTRY
.globl gate_return
gate_return:
    # 5. Back from the callee: results in rax, rdx.
    mov r8, rax
    mov r9, rdx
    mov eax, PKRU_G
    xor ecx, ecx
    xor edx, edx
.globl gate_wrpkru_3
gate_wrpkru_3:
    wrpkru
    cmp eax, PKRU_G
    jne .Lfault
    AREA
    cmp dword ptr [r11 + 0x00], PKRU_E
    jne .Lfault
    mov rax, [r11 + 0x08]
    test rax, rax
    jz .Lfault
    dec rax
    cmp qword ptr [r11 + 0x40 + rax*8], STUBID
    jne .Lfault
    mov qword ptr [r11 + 0x40 + rax*8], 0
    mov [r11 + 0x08], rax
    movabs r10, EMASK
    not r10
    and [r11 + 0x10], r10
    mov dword ptr [r11 + 0x00], PKRU_C

    # 6. Fill the return stack buffer on the frame page's scratch stack, then scrub.
    lea rsp, [r11 + 0xFF0]
    mov ecx, 16
.Lrsb:
    call .Lrsb_next
    int3
.Lrsb_next:
    add rsp, 8
    dec ecx
    jnz .Lrsb
    xor ebx, ebx
    xor ebp, ebp
    xor esi, esi
    xor edi, edi
    xor r10d, r10d
    xor r11d, r11d
    xor r12d, r12d
    xor r13d, r13d
    xor r14d, r14d
    xor r15d, r15d
    vzeroall

    # 7. Caller keys; RSP and saved registers come only from C's own slot.
    mov eax, PKRU_C
    xor ecx, ecx
    xor edx, edx
.globl gate_wrpkru_4
gate_wrpkru_4:
    wrpkru
    cmp eax, PKRU_C
    jne .Lfault
    AREA
    cmp dword ptr [r11 + 0x00], PKRU_C
    jne .Lfault
    cmp qword ptr [r11 + CPAGE + 0x08], STUBID
    jne .Lfault
    mov rsp, [r11 + CPAGE + 0x00]
    mov rbx, [r11 + CPAGE + 0x10]
    mov rbp, [r11 + CPAGE + 0x18]
    mov r12, [r11 + CPAGE + 0x20]
    mov r13, [r11 + CPAGE + 0x28]
    mov r14, [r11 + CPAGE + 0x30]
    mov r15, [r11 + CPAGE + 0x38]
    mov qword ptr [r11 + CPAGE + 0x08], 0
    mov rax, r8
    mov rdx, r9
    xor r8d, r8d
    xor r9d, r9d
    ret

.Ldeny:
    mov rax, -1
    ret
.Lfault:
    ud2
.globl gate_stub_end
gate_stub_end:
