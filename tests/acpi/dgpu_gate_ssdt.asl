/*
 * SPDX-License-Identifier: GPL-3.0-only
 * A laptop-style discrete GPU for scripts/dgpu_power_gate.py: a PCIe root port at
 * 00:1c.0 whose power resource (like Dell's PG01 on PEG1) is named by _PR0/_PR3, and the
 * GPU below it at _ADR 0. _ON/_OFF write 'N'/'F' to the debug console port 0x402, so the
 * gate sees the firmware methods really ran. SXGT asks SexOS to power the GPU back on and
 * off again, to exercise power_on; real firmware never defines it.
 */
DefinitionBlock ("", "SSDT", 2, "SEXOS", "DGPUGATE", 1)
{
    External (\_SB.PCI0, DeviceObj)

    Scope (\_SB.PCI0)
    {
        OperationRegion (SXDC, SystemIO, 0x402, 1)
        Field (SXDC, ByteAcc, NoLock, Preserve) { SXDB, 8 }

        Device (RP05)
        {
            Name (_ADR, 0x001C0000)
            PowerResource (PG05, 0, 0)
            {
                Name (ONST, One)
                Method (_STA, 0, NotSerialized) { Return (ONST) }
                Method (_ON, 0, NotSerialized) { ONST = One
                    SXDB = 0x4E }
                Method (_OFF, 0, NotSerialized) { ONST = Zero
                    SXDB = 0x46 }
            }
            Name (_PR0, Package (0x01) { PG05 })
            Name (_PR3, Package (0x01) { PG05 })

            Device (PEGP)
            {
                Name (_ADR, Zero)
                Name (SXGT, One)
                Method (_PS0, 0, NotSerialized) { SXDB = 0x30 }
                Method (_PS3, 0, NotSerialized) { SXDB = 0x33 }
            }
        }
    }
}
