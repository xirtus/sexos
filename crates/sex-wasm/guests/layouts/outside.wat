;; SPDX-License-Identifier: GPL-3.0-only
;; A bad layout plugin for tests and the gate: every rectangle starts past the work
;; area's right edge, so the Shell must refuse the answer (reason=outside).
(module
  (memory (export "memory") 1)
  (func (export "layout") (param $n i32) (param $w i32) (param $h i32) (result i32)
    (local $i i32) (local $p i32)
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (i32.and (local.get $n) (i32.const 63))))
        (local.set $p (i32.add (i32.const 1024) (i32.shl (local.get $i) (i32.const 4))))
        (i32.store (local.get $p) (i32.add (local.get $w) (i32.const 100)))
        (i32.store offset=4 (local.get $p) (i32.const 0))
        (i32.store offset=8 (local.get $p) (i32.const 300))
        (i32.store offset=12 (local.get $p) (i32.const 200))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (i32.const 1024)))
