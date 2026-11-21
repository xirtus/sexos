;; SPDX-License-Identifier: GPL-3.0-only
;; A bad layout plugin for tests and the gate: `layout` never returns, so its fuel
;; runs out and the Shell keeps the workspace's previous layout (reason=fuel).
(module
  (memory (export "memory") 1)
  (func (export "layout") (param $n i32) (param $w i32) (param $h i32) (result i32)
    (loop $forever (br $forever))
    (i32.const 0)))
