/*
 * SPDX-License-Identifier: GPL-3.0-only
 * Fixture for crates/sex-laptop/tests/awcc_aml.rs: unary maths with a target (Not,
 * FindSetLeftBit, FindSetRightBit), which panicked the interpreter on the x17 R1, and a
 * divide by zero, which must fail its method rather than the interpreter.
 * Rebuild after editing: iasl -p math_ssdt math_ssdt.asl
 */
DefinitionBlock ("", "SSDT", 2, "SEXOS", "MATH", 1)
{
    Name (MSKV, 0)
    Method (MATH, 2, Serialized)
    {
        Local0 = Arg0
        Not (Local0, MSKV)
        Local1 = (MSKV & 0xFF)
        FindSetLeftBit (Arg1, Local2)
        FindSetRightBit (Arg1, Local3)
        Local4 = (Arg1 >> 3)
        Return ((Local1 << 16) | (Local2 << 8) | (Local3 << 4) | Local4)
    }
    Method (DIV0, 1, Serialized)
    {
        Return (10 / Arg0)
    }
}
