# 2026-11-25 Scroll Lock always reaches Spindle

- Change: Loom asks Shell for desktop keys so F3 preview reaches it. That also took Scroll Lock, so a person in Loom could not toggle Spindle (and the Files jobs gate timed out on ToggleSpindle). Shell now keeps Scroll Lock, like F11, even for a program that asks for desktop keys.
- Also in PR #820: Spindle command receipts name every built-in command (not only a short allowlist), so linen-list, notify and net-* receipts are visible again; unknown words still record as `other`.
- Tests: GitHub CI desktop and Files gates.
