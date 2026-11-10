/* SexOS native QuickJS: the runtime unwinds traps with __builtin_setjmp/longjmp. */
#pragma once
typedef void *jmp_buf[5];
