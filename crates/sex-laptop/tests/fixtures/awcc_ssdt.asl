/*
 * SPDX-License-Identifier: GPL-3.0-only
 * Fixture for crates/sex-laptop/tests/awcc_aml.rs: the pretend Alienware x17 R1 WMI device
 * from tests/acpi/laptop_gate_ssdt.asl, and a package lookup through DerefOf. Rebuild the
 * .aml after editing: iasl -p awcc_ssdt awcc_ssdt.asl
 */
DefinitionBlock ("", "SSDT", 2, "SEXOS", "AWCC", 1)
{
    Scope (\_SB)
    {
        Device (AMW0)
        {
            Name (_HID, EisaId ("PNP0C14"))
            Name (_UID, "AWCC")
            Method (_STA, 0, NotSerialized) { Return (0x0F) }
            Name (_WDG, Buffer (40)
            {
                0xCE, 0x91, 0x05, 0xA7, 0x97, 0xA9, 0xDA, 0x11, 0xB0, 0x12, 0xB6, 0x22, 0xA1, 0xEF, 0x54, 0x92,
                0x41, 0x58, 0x01, 0x02,
                0x94, 0x59, 0xBB, 0x9D, 0x97, 0xA9, 0xDA, 0x11, 0xB0, 0x12, 0xB6, 0x22, 0xA1, 0xEF, 0x54, 0x92,
                0xD0, 0x00, 0x01, 0x08
            })
            Name (PROF, 0xA0)
            Name (NTFD, Zero)
            Name (RIDS, Package (9) { 0x132, 0x133, 0x101, 0x106, 0x103, 0xA0, 0xA4, 0xA3, 0xAB })
            Method (WMAX, 3, Serialized)
            {
                CreateByteField (Arg2, Zero, WOPR)
                CreateByteField (Arg2, One, WARG)
                If (Arg1 == 0x14)
                {
                    If (WOPR == 0x02)
                    {
                        If (NTFD == Zero) { NTFD = One
                            Notify (AMW0, 0xD0) }
                        Return (0x04000302)
                    }
                    If (WOPR == 0x03)
                    {
                        If (WARG < 0x09) { Return (DerefOf (RIDS [WARG])) }
                        Return (0xFFFFFFFF)
                    }
                    If (WOPR == 0x04)
                    {
                        If (WARG == 0x01) { Return (41) }
                        If (WARG == 0x06) { Return (38) }
                        Return (Zero)
                    }
                    If (WOPR == 0x05)
                    {
                        If (WARG == 0x32) { Return (2400) }
                        If (WARG == 0x33) { Return (2100) }
                        Return (0xFFFFFFFF)
                    }
                    If (WOPR == 0x08) { Return (Zero) }
                    If (WOPR == 0x09)
                    {
                        If (WARG == 0x32) { Return (5600) }
                        Return (5900)
                    }
                    If (WOPR == 0x0B) { Return (PROF) }
                }
                If (Arg1 == 0x15)
                {
                    If (WOPR == One) { PROF = WARG
                        Return (Zero) }
                }
                Return (0xFFFFFFFF)
            }
            Method (_WED, 1, NotSerialized)
            {
                If (Arg0 == 0xD0) { Return (Buffer (8) { 0x02, 0x00, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00 }) }
                Return (Buffer (One) { 0x00 })
            }
        }

        Device (DWMI)
        {
            Name (_HID, EisaId ("PNP0C14"))
            Name (_UID, "DELL")
            Method (_STA, 0, NotSerialized) { Return (0x0F) }
            Name (_WDG, Buffer (40)
            {
                0xCE, 0x93, 0x05, 0xA8, 0x97, 0xA9, 0xDA, 0x11, 0xB0, 0x12, 0xB6, 0x22, 0xA1, 0xEF, 0x54, 0x92,
                0x42, 0x43, 0x01, 0x02,
                0xBC, 0xDC, 0x9D, 0x8D, 0x97, 0xA9, 0xDA, 0x11, 0xB0, 0x12, 0xB6, 0x22, 0xA1, 0xEF, 0x54, 0x92,
                0x42, 0x44, 0x01, 0x00
            })
            Method (WQBD, 1, NotSerialized)
            {
                Return (Buffer (16) { 0x44, 0x45, 0x4C, 0x4C, 0x20, 0x57, 0x4D, 0x49, 0x01, 0x00, 0x00, 0x00, 0x30, 0x00, 0x00, 0x00 })
            }
            Name (T341, Zero)
            Name (T342, Zero)
            Name (T343, Zero)
            Name (T346, One)
            Name (T349, 50)
            Name (T34A, 90)
            Method (TGET, 1, Serialized)
            {
                If (Arg0 == 0x0341) { Return (T341) }
                If (Arg0 == 0x0342) { Return (T342) }
                If (Arg0 == 0x0343) { Return (T343) }
                If (Arg0 == 0x0346) { Return (T346) }
                If (Arg0 == 0x0349) { Return (T349) }
                If (Arg0 == 0x034A) { Return (T34A) }
                Return (Ones)
            }
            Method (TSET, 2, Serialized)
            {
                If (Arg0 < 0x0348) { T341 = Zero
                    T342 = Zero
                    T343 = Zero
                    T346 = Zero }
                If (Arg0 == 0x0341) { T341 = Arg1 }
                If (Arg0 == 0x0342) { T342 = Arg1 }
                If (Arg0 == 0x0343) { T343 = Arg1 }
                If (Arg0 == 0x0346) { T346 = Arg1 }
                If (Arg0 == 0x0349) { T349 = Arg1 }
                If (Arg0 == 0x034A) { T34A = Arg1 }
            }
            Method (WMBC, 3, Serialized)
            {
                CreateWordField (Arg2, Zero, DCLS)
                CreateDWordField (Arg2, 0x04, DIN0)
                CreateDWordField (Arg2, 0x08, DIN1)
                CreateDWordField (Arg2, 0x14, DOU0)
                CreateDWordField (Arg2, 0x18, DOU1)
                Local0 = TGET (DIN0)
                If ((Arg1 != One) || (Local0 == Ones)) { DOU0 = 0xFFFFFFFE }
                ElseIf (DCLS == Zero) { DOU1 = Local0
                    DOU0 = Zero }
                ElseIf (DCLS == One) { TSET (DIN0, DIN1)
                    DOU0 = Zero }
                Else { DOU0 = 0xFFFFFFFE }
                Return (Arg2)
            }
        }
    }

    Name (LKUP, Package (3) { 0x132, 0x133, 0x101 })
    Method (DREF, 1, Serialized) { Return (DerefOf (LKUP [Arg0])) }
    Name (BYTS, Buffer (3) { 0x10, 0x20, 0x30 })
    Method (BREF, 1, Serialized) { Return (DerefOf (BYTS [Arg0])) }
    Method (BFLD, 1, Serialized)
    {
        CreateWordField (Arg0, One, WRD1)
        If (WRD1 == 0x0302) { Return (WRD1 + One) }
        Return (Zero)
    }
}
