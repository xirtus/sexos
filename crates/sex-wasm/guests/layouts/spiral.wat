;; SPDX-License-Identifier: GPL-3.0-only
;; The built-in window layout plugin (`silk layout WS plugin:spiral`), a Fibonacci
;; spiral: each window takes half of what is left, turning left, top, right, bottom,
;; and the last window takes all that remains. Every tile keeps 4 px to each side.
;;
;; `layout(n, w, h)` writes n rectangles at offset 1024 and returns 1024: x, y (i32),
;; w, h (u32), relative to the work area's top-left corner. It imports nothing.
;; The test `layout_guests_are_current` checks spiral.wasm was built from this file.
(module
  (memory (export "memory") 1)
  (func $put (param $i i32) (param $x i32) (param $y i32) (param $w i32) (param $h i32)
    (local $p i32)
    (local.set $p (i32.add (i32.const 1024) (i32.shl (local.get $i) (i32.const 4))))
    (i32.store (local.get $p) (i32.add (local.get $x) (i32.const 4)))
    (i32.store offset=4 (local.get $p) (i32.add (local.get $y) (i32.const 4)))
    (i32.store offset=8 (local.get $p) (i32.sub (local.get $w) (i32.const 8)))
    (i32.store offset=12 (local.get $p) (i32.sub (local.get $h) (i32.const 8))))
  (func (export "layout") (param $n i32) (param $w i32) (param $h i32) (result i32)
    (local $i i32) (local $x i32) (local $y i32) (local $half i32) (local $k i32)
    ;; At most 64 rectangles fit before the end of the page; the host asks for fewer.
    (if (i32.gt_u (local.get $n) (i32.const 64)) (then (local.set $n (i32.const 64))))
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (if (i32.eq (local.get $i) (i32.sub (local.get $n) (i32.const 1)))
          (then
            (call $put (local.get $i) (local.get $x) (local.get $y) (local.get $w) (local.get $h))
            (br $done)))
        (local.set $k (i32.and (local.get $i) (i32.const 3)))
        (if (i32.eq (local.get $k) (i32.const 0))
          (then ;; the left half
            (local.set $half (i32.shr_u (local.get $w) (i32.const 1)))
            (call $put (local.get $i) (local.get $x) (local.get $y) (local.get $half) (local.get $h))
            (local.set $x (i32.add (local.get $x) (local.get $half)))
            (local.set $w (i32.sub (local.get $w) (local.get $half)))))
        (if (i32.eq (local.get $k) (i32.const 1))
          (then ;; the top half
            (local.set $half (i32.shr_u (local.get $h) (i32.const 1)))
            (call $put (local.get $i) (local.get $x) (local.get $y) (local.get $w) (local.get $half))
            (local.set $y (i32.add (local.get $y) (local.get $half)))
            (local.set $h (i32.sub (local.get $h) (local.get $half)))))
        (if (i32.eq (local.get $k) (i32.const 2))
          (then ;; the right half
            (local.set $half (i32.shr_u (local.get $w) (i32.const 1)))
            (call $put (local.get $i) (i32.sub (i32.add (local.get $x) (local.get $w)) (local.get $half)) (local.get $y) (local.get $half) (local.get $h))
            (local.set $w (i32.sub (local.get $w) (local.get $half)))))
        (if (i32.eq (local.get $k) (i32.const 3))
          (then ;; the bottom half
            (local.set $half (i32.shr_u (local.get $h) (i32.const 1)))
            (call $put (local.get $i) (local.get $x) (i32.sub (i32.add (local.get $y) (local.get $h)) (local.get $half)) (local.get $w) (local.get $half))
            (local.set $h (i32.sub (local.get $h) (local.get $half)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (i32.const 1024)))
