# 2026-11-23 AML interpreter no longer panics on the x17 R1's DSDT

- **The touchpad, battery, lid and brightness come back on the x17 R1**: SexACPI's AML
  interpreter (vendor/acpi-6.1.1) panicked on `Not (x, Target)` and the FindSetBit ops,
  started with two arguments but matched as one, after the field list fix let the DSDT
  load further. A panic sets all AML aside for the boot.
- Those three now store into their target. Divide or Mod by zero is an `AmlError`, shifts
  of 64 or more give 0, and every other argument mismatch or unsupported store target is
  `LibUnimplemented`, which fails only the method that used it.
- Host test `unary_maths_and_divide_by_zero` (crates/sex-laptop/tests/awcc_aml.rs, fixture
  math_ssdt.asl).
