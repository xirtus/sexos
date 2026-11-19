; SPDX-License-Identifier: GPL-3.0-only
; Drag D2 test guest: the smallest "kernel" that boots through the Linux x86 boot
; protocol the way a bzImage does, and talks to the two devices Drag D2 gives a guest.
;
;   nasm -f bin -o drag_test_guest.bin drag_test_guest.asm
;
; Layout (a real bzImage's): a boot sector holding the setup header at 0x1F1, two setup
; sectors, then the protected-mode payload at file offset 0x600, which the loader puts
; at code32_start (1 MiB). xloadflags says the 64-bit entry at payload + 0x200 exists.
;
; Two ways in, the same 64-bit code after:
; - Drag (and any boot-protocol 2.12+ loader with a 64-bit entry) enters startup_64 in
;   long mode with identity paging, __BOOT_CS = 0x10, __BOOT_DS = 0x18 and RSI = the
;   boot_params (zero page).
; - A loader that runs the real-mode setup code (QEMU's -kernel, through linuxboot or
;   qboot) starts rm_start at (setup segment + 0x20):0 with DS = the setup segment,
;   which holds the header. rm_start does what Drag's loader does (zero page at 0x7000
;   with the header and the E820 map, from INT 15h as Linux's setup code reads it;
;   identity tables at 0x9000; GDT at 0x500) and jumps to startup_64. This is how the host gate
;   checks the guest itself against QEMU's own 16550 and virtio-mmio.
;
; startup_64 prints over the 16550 at 0x3F8 (polled, no interrupts), checks the zero
; page, prints its command line, finds the virtio-blk device named by a
; "virtio_mmio.device=SIZE@0xBASE:IRQ" word on that command line (the form Linux
; itself reads), negotiates VIRTIO_F_VERSION_1, reads sector 0 and prints its first
; line, writes sector 1 and reads it back, then halts with interrupts off.

bits 16
section setup start=0 vstart=0
	times 0x1F1 db 0
	db 2                            ; 0x1F1 setup_sects
	dw 0                            ; 0x1F2 root_flags
	dd (payload_end - payload_start + 15) / 16 ; 0x1F4 syssize
	dw 0                            ; 0x1F8 ram_size
	dw 0xFFFF                       ; 0x1FA vid_mode
	dw 0                            ; 0x1FC root_dev
	dw 0xAA55                       ; 0x1FE boot_flag
	db 0xEB, rm_start - 0x202       ; 0x200 jump
	db 'HdrS'                       ; 0x202 header
	dw 0x020F                       ; 0x206 version 2.15
	dd 0                            ; 0x208 realmode_swtch
	dw 0x1000                       ; 0x20C start_sys_seg
	dw 0                            ; 0x20E kernel_version
	db 0                            ; 0x210 type_of_loader
	db 1                            ; 0x211 loadflags: LOADED_HIGH
	dw 0x8000                       ; 0x212 setup_move_size
	dd 0x100000                     ; 0x214 code32_start
	dd 0                            ; 0x218 ramdisk_image
	dd 0                            ; 0x21C ramdisk_size
	dd 0                            ; 0x220 bootsect_kludge
	dw 0                            ; 0x224 heap_end_ptr
	db 0                            ; 0x226 ext_loader_ver
	db 0                            ; 0x227 ext_loader_type
	dd 0                            ; 0x228 cmd_line_ptr
	dd 0x7FFFFFFF                   ; 0x22C initrd_addr_max
	dd 0x200000                     ; 0x230 kernel_alignment
	db 0                            ; 0x234 relocatable_kernel
	db 21                           ; 0x235 min_alignment
	dw 1                            ; 0x236 xloadflags: XLF_KERNEL_64
	dd 2047                         ; 0x238 cmdline_size
	dd 0                            ; 0x23C hardware_subarch
	dq 0                            ; 0x240 hardware_subarch_data
	dd 0                            ; 0x248 payload_offset
	dd 0                            ; 0x24C payload_length
	dq 0                            ; 0x250 setup_data
	dq 0x100000                     ; 0x258 pref_address
	dd 0x100000                     ; 0x260 init_size
	dd 0                            ; 0x264 handover_offset
	dd 0                            ; 0x268 kernel_info_offset

rm_start:                               ; real mode, CS = setup + 0x20, DS = setup
	cli
	cld
	xor ax, ax                      ; a fresh zero page at 0x7000 (where Drag puts it)
	mov es, ax
	mov di, 0x7000
	xor eax, eax
	mov cx, 0x400
	db 0xF3, 0x66, 0xAB              ; rep stosd, spelled out: NASM 2 and 3 order these prefixes differently
	mov si, 0x1F1                   ; with the setup header the loader filled in
	mov di, 0x71F1
	mov cx, 0x290 - 0x1F1
	rep movsb
	mov di, 0x72D0                  ; and the BIOS memory map, as Linux's setup code
	xor ebx, ebx                    ; reads it (INT 15h E820)
	xor bp, bp
.e820:	mov eax, 0xE820
	mov edx, 0x534D4150             ; "SMAP"
	mov ecx, 20
	int 0x15
	jc .mapped
	cmp eax, 0x534D4150
	jne .mapped
	add di, 20
	inc bp
	cmp bp, 128
	jae .mapped
	test ebx, ebx
	jnz .e820
.mapped:
	mov ax, bp
	mov [es:0x71E8], al
	mov di, 0x9000                  ; identity tables: PML4 0x9000, PDPT 0xA000, PD 0xB000..0xEFFF
	xor eax, eax
	mov cx, 0x1800
	db 0xF3, 0x66, 0xAB              ; rep stosd, spelled out: NASM 2 and 3 order these prefixes differently
	mov dword [es:0x9000], 0xA003
	mov dword [es:0xA000], 0xB003
	mov dword [es:0xA008], 0xC003
	mov dword [es:0xA010], 0xD003
	mov dword [es:0xA018], 0xE003
	mov di, 0xB000
	mov eax, 0x83                   ; present, writable, 2 MiB
	mov cx, 2048
.pd:	mov [es:di], eax
	add eax, 0x200000
	add di, 8
	loop .pd
	mov dword [es:0x500], 0         ; GDT: null, null, __BOOT_CS, __BOOT_DS
	mov dword [es:0x504], 0
	mov dword [es:0x508], 0
	mov dword [es:0x50C], 0
	mov dword [es:0x510], 0x0000FFFF
	mov dword [es:0x514], 0x00AF9B00
	mov dword [es:0x518], 0x0000FFFF
	mov dword [es:0x51C], 0x00CF9300
	mov word [es:0x5E0], 0x1F
	mov dword [es:0x5E2], 0x500
	o32 lgdt [es:0x5E0]
	mov ebx, 0x7000                 ; boot_params
	mov eax, [ds:0x214]
	add eax, 0x200                  ; startup_64
	mov [es:0x5F0], eax
	mov word [es:0x5F4], 0x10
	mov eax, 0x9000
	mov cr3, eax
	mov eax, cr4
	or eax, 0x20                    ; PAE
	mov cr4, eax
	mov ecx, 0xC0000080
	rdmsr
	or eax, 0x100                   ; EFER.LME
	wrmsr
	mov eax, cr0
	or eax, 0x80000001              ; PG | PE
	mov cr0, eax
	mov esi, ebx
	jmp far dword [es:0x5F0]

	times 0x600 - ($ - $$) db 0

section payload start=0x600 vstart=0x100000
default rel
QUEUE     equ 0x150000                  ; descriptors; avail at +0x100; used at +0x1000
REQ       equ 0x152000
DATA      equ 0x153000
STATUS    equ 0x154000
STACK_TOP equ 0x14F000
UART      equ 0x3F8

payload_start:
bits 32
startup_32:                             ; this guest has no 32-bit path
	cli
.h:	hlt
	jmp .h
	times 0x200 - ($ - payload_start) db 0

bits 64
startup_64:
	cli
	cld
	mov esi, esi
	mov ax, 0x18
	mov ds, ax
	mov es, ax
	mov ss, ax
	mov rsp, STACK_TOP
	mov r15, rsi                    ; boot_params
	call uart_init
	lea rsi, [msg_hello]
	call puts

	cmp dword [r15 + 0x202], 'HdrS'
	jne fail
	movzx ebx, byte [r15 + 0x1E8]   ; e820_entries
	test ebx, ebx
	jz fail
	lea rsi, [msg_params]
	call puts
	mov rax, rbx
	call puthex
	call newline

	mov r14d, dword [r15 + 0x228]   ; cmd_line_ptr
	test r14, r14
	jz fail
	lea rsi, [msg_cmdline]
	call puts
	mov rsi, r14
	call puts
	call newline

	; Find "@0x<base>" words on the command line and take the first virtio-blk there.
	mov rdi, r14
	mov r13d, 2048
.scan:	dec r13d
	jz no_blk
	mov al, [rdi]
	test al, al
	jz no_blk
	cmp al, '@'
	jne .next
	cmp word [rdi + 1], '0x'
	jne .next
	lea rsi, [rdi + 3]
	call parse_hex
	mov rdi, rsi
	mov rbx, rax
	mov eax, [rbx]                  ; device registers are read with plain
	cmp eax, 0x74726976             ; loads (as Linux's readl): "virt"
	jne .scan
	mov eax, [rbx + 4]
	cmp eax, 2                      ; virtio-mmio version 2 (modern)
	jne .scan
	mov eax, [rbx + 8]
	cmp eax, 2                      ; block device
	je found
	jmp .scan
.next:	inc rdi
	jmp .scan

found:
	lea rsi, [msg_blk]
	call puts
	mov rax, rbx
	call puthex
	lea rsi, [msg_sectors]
	call puts
	mov eax, [rbx + 0x100]          ; capacity (low half), in 512-byte sectors
	call puthex
	call newline

	mov dword [rbx + 0x70], 0       ; reset
	mov dword [rbx + 0x70], 1       ; ACKNOWLEDGE
	mov dword [rbx + 0x70], 3       ; DRIVER
	mov dword [rbx + 0x14], 1
	mov eax, [rbx + 0x10]
	test eax, 1                     ; VIRTIO_F_VERSION_1
	jz fail
	mov dword [rbx + 0x24], 0
	mov dword [rbx + 0x20], 0
	mov dword [rbx + 0x24], 1
	mov dword [rbx + 0x20], 1
	mov dword [rbx + 0x70], 11      ; FEATURES_OK
	mov eax, [rbx + 0x70]
	test eax, 8
	jz fail
	mov dword [rbx + 0x30], 0       ; queue 0
	mov eax, [rbx + 0x34]
	cmp eax, 8
	jb fail
	mov dword [rbx + 0x38], 8
	mov rdi, QUEUE
	mov ecx, 0x2000 / 8
	xor eax, eax
	rep stosq
	mov dword [rbx + 0x80], QUEUE
	mov dword [rbx + 0x84], 0
	mov dword [rbx + 0x90], QUEUE + 0x100
	mov dword [rbx + 0x94], 0
	mov dword [rbx + 0xA0], QUEUE + 0x1000
	mov dword [rbx + 0xA4], 0
	mov dword [rbx + 0x44], 1       ; queue ready
	mov dword [rbx + 0x70], 15      ; DRIVER_OK

	xor edi, edi                    ; read sector 0
	xor edx, edx
	call blk_request
	jc fail
	lea rsi, [msg_disk]
	call puts
	mov rsi, DATA
	mov ecx, 72
.line:	mov al, [rsi]
	cmp al, 0x20
	jb .eol
	cmp al, 0x7E
	ja .eol
	call putc
	inc rsi
	dec ecx
	jnz .line
.eol:	call newline

	mov rdi, DATA                   ; write sector 1, read it back
	lea rsi, [msg_mark]
	mov ecx, 32
	rep movsb
	mov edi, 1
	mov edx, 1
	call blk_request
	jc fail
	mov rdi, DATA
	mov ecx, 512 / 8
	xor eax, eax
	rep stosq
	xor edi, edi
	mov edx, 1
	call blk_request
	jc fail
	mov rsi, DATA
	lea rdi, [msg_mark]
	mov ecx, 32
	repe cmpsb
	jne fail
	lea rsi, [msg_written]
	call puts

	lea rsi, [msg_done]
	call puts
halt:	cli
	hlt
	jmp halt

no_blk:
	lea rsi, [msg_noblk]
	call puts
	jmp halt
fail:
	lea rsi, [msg_fail]
	call puts
	jmp halt

; edi = request type (0 IN, 1 OUT), rdx = sector; rbx = the device. CF on failure.
default abs                             ; the queue is at fixed addresses
blk_request:
	mov dword [REQ], edi
	mov dword [REQ + 4], 0
	mov qword [REQ + 8], rdx
	mov byte [STATUS], 0xFF
	mov rax, REQ
	mov qword [QUEUE], rax
	mov dword [QUEUE + 8], 16
	mov word [QUEUE + 12], 1        ; NEXT
	mov word [QUEUE + 14], 1
	mov rax, DATA
	mov qword [QUEUE + 16], rax
	mov dword [QUEUE + 24], 512
	mov eax, 1                      ; NEXT, and WRITE (device writes) for a read
	test edi, edi
	jnz .out
	or eax, 2
.out:	mov word [QUEUE + 28], ax
	mov word [QUEUE + 30], 2
	mov rax, STATUS
	mov qword [QUEUE + 32], rax
	mov dword [QUEUE + 40], 1
	mov word [QUEUE + 44], 2        ; WRITE
	mov word [QUEUE + 46], 0
	movzx eax, word [QUEUE + 0x102]
	mov ecx, eax
	and ecx, 7
	mov word [QUEUE + 0x104 + rcx * 2], 0
	inc eax
	mfence
	mov word [QUEUE + 0x102], ax
	mfence
	mov dword [rbx + 0x50], 0       ; notify queue 0
	mov ecx, 100000000
.wait:	cmp word [QUEUE + 0x1002], ax
	je .used
	pause
	dec ecx
	jnz .wait
	stc
	ret
.used:	cmp byte [STATUS], 0
	je .ok
	stc
	ret
.ok:	clc
	ret
default rel

uart_init:
	mov dx, UART + 1
	xor al, al
	out dx, al                      ; no interrupts
	mov dx, UART + 3
	mov al, 0x80
	out dx, al                      ; DLAB
	mov dx, UART
	mov al, 1
	out dx, al                      ; 115200 baud
	mov dx, UART + 1
	xor al, al
	out dx, al
	mov dx, UART + 3
	mov al, 3
	out dx, al                      ; 8N1
	mov dx, UART + 2
	mov al, 0xC7
	out dx, al                      ; FIFO on
	mov dx, UART + 4
	mov al, 3
	out dx, al                      ; DTR, RTS
	ret

putc:                                   ; al
	push rdx
	push rcx
	push rax
	mov dx, UART + 5
	mov ecx, 1000000
.w:	in al, dx
	test al, 0x20                   ; THR empty
	jnz .go
	dec ecx
	jnz .w
.go:	pop rax
	mov dx, UART
	out dx, al
	pop rcx
	pop rdx
	ret

puts:                                   ; rsi: a string ending in 0 (at most 512 bytes)
	push rcx
	mov ecx, 512
.l:	mov al, [rsi]
	test al, al
	jz .e
	call putc
	inc rsi
	dec ecx
	jnz .l
.e:	pop rcx
	ret

newline:
	mov al, 10
	jmp putc

puthex:                                 ; rax, as 0x followed by hex digits
	push rcx
	push rdx
	mov rdx, rax
	mov al, '0'
	call putc
	mov al, 'x'
	call putc
	mov ecx, 16
.skip:	cmp ecx, 1                      ; drop leading zeros
	je .d
	mov rax, rdx
	shr rax, 60
	jnz .d
	shl rdx, 4
	dec ecx
	jmp .skip
.d:	mov rax, rdx
	shr rax, 60
	add al, '0'
	cmp al, '9'
	jbe .p
	add al, 'a' - '0' - 10
.p:	call putc
	shl rdx, 4
	dec ecx
	jnz .d
	pop rdx
	pop rcx
	ret

parse_hex:                              ; rsi -> hex digits; rax = value, rsi after them
	xor eax, eax
	mov ecx, 16
.l:	movzx edx, byte [rsi]
	cmp dl, '0'
	jb .e
	cmp dl, '9'
	jbe .dig
	or dl, 0x20                     ; lower case
	cmp dl, 'a'
	jb .e
	cmp dl, 'f'
	ja .e
	sub dl, 'a' - 10
	jmp .add
.dig:	sub dl, '0'
.add:	shl rax, 4
	or rax, rdx
	inc rsi
	dec ecx
	jnz .l
.e:	ret

msg_hello:   db "[guest] Drag D2 test guest: 64-bit entry through the Linux boot protocol", 10, 0
msg_params:  db "[guest] boot_params ok, e820 entries ", 0
msg_cmdline: db "[guest] cmdline: ", 0
msg_blk:     db "[guest] virtio-blk at ", 0
msg_sectors: db " sectors ", 0
msg_disk:    db "[guest] disk: ", 0
msg_mark:    db "drag-d2 guest wrote sector one.", 0
msg_written: db "[guest] disk write and read back ok", 10, 0
msg_done:    db "[guest] done", 10, 0
msg_noblk:   db "[guest] no virtio-blk on the command line", 10, 0
msg_fail:    db "[guest] FAILED", 10, 0
payload_end:
