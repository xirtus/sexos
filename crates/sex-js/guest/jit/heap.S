# SPDX-License-Identifier: GPL-3.0-only
# SysV: rdi = validated host frame, rsi = bounded output. No calls/relocations.
.intel_syntax noprefix
.section .text.own,"ax"
.global own
own:
    xor eax, eax
    test DWORD PTR [rdi+40], 48
    jnz 1f
    mov rcx, [rdi]
    mov edx, [rcx]
    cmp edx, DWORD PTR [rdi+24]
    jne 1f
    mov rcx, [rdi+8]
    mov edx, [rcx]
    cmp edx, DWORD PTR [rdi+32]
    jne 1f
    mov rcx, [rdi+16]
    mov rdx, [rcx]
    mov [rsi], rdx
    mov eax, 1
1:  ret

.macro strings
    mov r8, [rdi]
    mov r9, [rdi+8]
    mov r10, [rdi+16]
    mov r11, [rdi+24]
    mov rcx, [rdi+32]
    mov rdx, [rdi+40]
.endm
.section .text.compare,"ax"
.global compare
compare:
    strings
2:  test r10, r10
    jz 5f
    test r11, r11
    jz 6f
    movzx eax, BYTE PTR [r8]
    cmp ecx, 2
    jne 3f
    movzx eax, WORD PTR [r8]
3:  movzx edi, BYTE PTR [r9]
    cmp edx, 2
    jne 4f
    movzx edi, WORD PTR [r9]
4:  cmp eax, edi
    jb 7f
    ja 6f
    add r8, rcx
    add r9, rdx
    dec r10
    dec r11
    jmp 2b
5:  xor eax, eax
    test r11, r11
    jz 8f
7:  mov eax, -1
    jmp 8f
6:  mov eax, 1
8:  mov [rsi], eax
    mov eax, 1
    ret
.section .text.concat,"ax"
.global concat
concat:
    strings
9:  test r10, r10
    jz 11f
    movzx eax, BYTE PTR [r8]
    cmp ecx, 2
    jne 10f
    movzx eax, WORD PTR [r8]
10: mov [rsi], ax
    add rsi, 2
    add r8, rcx
    dec r10
    jmp 9b
11: test r11, r11
    jz 13f
    movzx eax, BYTE PTR [r9]
    cmp edx, 2
    jne 12f
    movzx eax, WORD PTR [r9]
12: mov [rsi], ax
    add rsi, 2
    add r9, rdx
    dec r11
    jmp 11b
13: mov eax, 1
    ret
