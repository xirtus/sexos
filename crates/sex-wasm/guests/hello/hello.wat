;; SPDX-License-Identifier: GPL-3.0-only
;; The Wasm program's built-in guest, on the native `sex` imports: shows a greeting,
;; then echoes what is typed until Enter or Close. The test `hello_guest_is_current`
;; checks hello.wasm was built from this file (wat::parse_str).
(module
  (import "sex" "title" (func $title (param i32 i32) (result i32)))
  (import "sex" "show" (func $show (param i32 i32) (result i32)))
  (import "sex" "log" (func $log (param i32 i32) (result i32)))
  (import "sex" "next" (func $next (result i32)))
  (import "sex" "exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "Hello")
  (data (i32.const 16) "hello.ready")
  (data (i32.const 64) "Hello from WebAssembly on SexOS.\nThis guest runs in an interpreter inside the wasm program.\nType something; Enter ends.\n\n> ")
  (global $head i32 (i32.const 64))
  (global $headlen i32 (i32.const 123))
  (func (export "_start") (local $k i32) (local $n i32)
    (drop (call $title (i32.const 0) (i32.const 5)))
    (local.set $n (global.get $headlen))
    (drop (call $show (global.get $head) (local.get $n)))
    (drop (call $log (i32.const 16) (i32.const 11)))
    (loop $l
      (local.set $k (call $next))
      (if (i32.eq (local.get $k) (i32.const -1)) (then (call $exit (i32.const 0))))
      (if (i32.eq (local.get $k) (i32.const 10)) (then (call $exit (i32.sub (local.get $n) (global.get $headlen)))))
      (if (i32.eq (local.get $k) (i32.const 8))
        (then (if (i32.gt_u (local.get $n) (global.get $headlen)) (then (local.set $n (i32.sub (local.get $n) (i32.const 1))))))
        (else (if (i32.and (i32.lt_u (local.get $k) (i32.const 256)) (i32.lt_u (local.get $n) (i32.const 4000)))
          (then
            (i32.store8 (i32.add (global.get $head) (local.get $n)) (local.get $k))
            (local.set $n (i32.add (local.get $n) (i32.const 1)))))))
      (drop (call $show (global.get $head) (local.get $n)))
      (br $l))))
