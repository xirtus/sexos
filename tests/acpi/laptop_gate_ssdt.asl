/*
 * SPDX-License-Identifier: GPL-3.0-only
 * Laptop firmware for scripts/laptop_power_gate.py: an AC adapter (unplugged), a battery
 * (43 of 86 Wh last-full, 90 Wh design, drawing 21.5 W: 50%, two hours left, 96% health),
 * a lid that closes for a few seconds every 40 reads, a thermal zone, and a panel with ACPI
 * brightness (_BCL/_BCM/_BQC). On its third read the battery's _BST sends the panel the
 * "brightness up" notification (0x86), as a laptop's EC _Qxx method does for the Fn key.
 * _BCM writes 'B' to the debug console port 0x402.
 *
 * A Serial IO I2C controller (\_SB.PCI0.I2C1, PCI 00:15.1, firmware clock counts in FMCN)
 * carries an I2C HID touchpad at 0x2C, 400 kHz, HID descriptor register 0x20 (_DSM), GPIO
 * interrupt pin 0x15C. QEMU has no Serial IO function, so the ACPI service must describe
 * the pad and Input must say its controller is not leased. Each _PS0 writes 'P'.
 *
 * A power engine plug-in (\_SB.PEPD, INT33A1) answers Intel's low-power S0 idle _DSM:
 * screen off 'd', entry 'S', exit 'W', screen on 'D' on port 0x402.
 *
 * An ACPI-WMI device (\_SB.AMW0, PNP0C14) plays an Alienware x17 R1: _WDG lists the
 * Alienware Command Center method block (GUID A70591CE-..., WMAX) and the Dell hotkey event
 * block (GUID 9DBB5994-..., notify 0xD0). WMAX answers like the real fan controller: two
 * fans (CPU 0x32, GPU 0x33), three sensors, four profiles (balanced 0xA0, performance 0xA4,
 * quiet 0xA3, G-Mode 0xAB), and it remembers the profile it is told to activate. The first
 * system description read raises the hotkey event, and _WED(0xD0) reports the mic-mute key.
 * Sensors stay cooler than the thermal zone, so the sensor line keeps the zone's reading.
 *
 * Built with -DTHERMAL_TRIP, the zone heats past its critical trip point (_CRT) after a
 * few reads, so the machine must shut itself down cleanly.
 */
DefinitionBlock ("", "SSDT", 2, "SEXOS", "LAPTOP", 1)
{
    External (\_SB.PCI0, DeviceObj)

    Scope (\_SB)
    {
        OperationRegion (SXDC, SystemIO, 0x402, 1)
        Field (SXDC, ByteAcc, NoLock, Preserve) { SXDB, 8 }

        Device (PEPD)
        {
            Name (_HID, "INT33A1")
            Name (_CID, EisaId ("PNP0D80"))
            Name (_UID, One)
            Method (_STA, 0, NotSerialized) { Return (0x0F) }
            Method (_DSM, 4, Serialized)
            {
                If (Arg0 == ToUUID ("c4eb40a0-6cd2-11e2-bcfd-0800200c9a66"))
                {
                    If (Arg2 == Zero) { Return (Buffer (One) { 0x79 }) }
                    If (Arg2 == 0x03) { SXDB = 0x64 }
                    If (Arg2 == 0x04) { SXDB = 0x44 }
                    If (Arg2 == 0x05) { SXDB = 0x53 }
                    If (Arg2 == 0x06) { SXDB = 0x57 }
                    Return (Zero)
                }
                Return (Buffer (One) { 0x00 })
            }
        }

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

        Device (ADP1)
        {
            Name (_HID, "ACPI0003")
            Method (_STA, 0, NotSerialized) { Return (0x0F) }
            Method (_PSR, 0, NotSerialized) { Return (Zero) }
        }

        Device (BAT0)
        {
            Name (_HID, EisaId ("PNP0C0A"))
            Name (_UID, One)
            Name (NBST, Zero)
            Method (_STA, 0, NotSerialized) { Return (0x1F) }
            Name (BIXP, Package (0x15)
            {
                One, Zero, 90000, 86000, One, 11400, 4300, 860, 300,
                95000, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF,
                100, 100, "SXGATE", "0001", "LION", "SEXOS", Zero
            })
            Method (_BIX, 0, NotSerialized) { Return (BIXP) }
            Method (_BST, 0, NotSerialized)
            {
                NBST++
                If (NBST == 3) { Notify (\_SB.PCI0.GFX0.DD1F, 0x86) }
                Return (Package (0x04) { One, 21500, 43000, 11400 })
            }
        }

        Device (LID0)
        {
            Name (_HID, EisaId ("PNP0C0D"))
            Name (NLID, Zero)
            Method (_LID, 0, NotSerialized)
            {
                NLID++
                If ((NLID % 40) >= 30) { If ((NLID % 40) < 34) { Return (Zero) } }
                Return (One)
            }
        }
    }

    Scope (\_SB.PCI0)
    {
        Device (GFX0)
        {
            Name (_ADR, 0x00020000)
            Device (DD1F)
            {
                Name (_ADR, 0x1F)
                Name (BCLV, 50)
                Method (_BCL, 0, NotSerialized)
                {
                    Return (Package (0x0C) { 80, 50, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100 })
                }
                Method (_BCM, 1, NotSerialized)
                {
                    BCLV = Arg0
                    \_SB.SXDB = 0x42
                }
                Method (_BQC, 0, NotSerialized) { Return (BCLV) }
            }
        }
    }

    Scope (\_SB.PCI0)
    {
        Device (I2C1)
        {
            Name (_ADR, 0x00150001)
            Method (FMCN, 0, NotSerialized) { Return (Package (0x03) { 0x52, 0x9C, 0x1E }) }
            Method (_PS0, 0, NotSerialized) { \_SB.SXDB = 0x50 }
            Method (_PS3, 0, NotSerialized) { }

            Device (TPD0)
            {
                Name (_HID, "SXTP0001")
                Name (_CID, "PNP0C50")
                Name (_UID, One)
                Method (_STA, 0, NotSerialized) { Return (0x0F) }
                Method (_PS0, 0, NotSerialized) { \_SB.SXDB = 0x50 }
                Method (_PS3, 0, NotSerialized) { }
                Method (_CRS, 0, NotSerialized)
                {
                    Return (ResourceTemplate ()
                    {
                        I2cSerialBusV2 (0x002C, ControllerInitiated, 400000, AddressingMode7Bit, "\\_SB.PCI0.I2C1",
                            0x00, ResourceConsumer, , Exclusive, )
                        GpioInt (Level, ActiveLow, Exclusive, PullUp, 0x0000, "\\_SB.GPI0", 0x00, ResourceConsumer, , )
                        {
                            0x015C
                        }
                    })
                }
                Method (_DSM, 4, Serialized)
                {
                    If (Arg0 == ToUUID ("3cdff6f7-4267-4555-ad05-b30a3d8938de"))
                    {
                        If (Arg2 == Zero) { Return (Buffer (One) { 0x03 }) }
                        If (Arg2 == One) { Return (0x20) }
                    }
                    Return (Buffer (One) { 0x00 })
                }
            }
        }
    }

    Scope (\_TZ)
    {
        ThermalZone (TZ00)
        {
            Name (NTMP, Zero)
            Method (_TMP, 0, NotSerialized)
            {
                NTMP++
#ifdef THERMAL_TRIP
                If (NTMP > 8) { Return (3800) }
#endif
                Return (3182)
            }
            Method (_CRT, 0, NotSerialized) { Return (3732) }
        }
    }
}
