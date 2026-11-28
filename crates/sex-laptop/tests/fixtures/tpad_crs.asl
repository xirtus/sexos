DefinitionBlock ("", "SSDT", 2, "SEXOS", "TPADTST", 1)
{
    Name (OSYS, 0x07E5)
    External (\TPDF, IntObj)
    Scope (\_SB) {
      Device (I2C0) {
        Name (_ADR, 0x00150000)
        Device (TPAD) {
            Name (_HID, "DELL0A8F")
            Name (TPID, Package () {
                Package () { 0x01, 0x2C, 0x20 },
                Package () { 0x02, 0x15, 0x01 },
                Package () { 0xFE, 0x00, 0x00 }
            })
            Name (ADR0, Zero)
            Name (HID2, Zero)
            Name (SBFB, ResourceTemplate () { I2cSerialBusV2 (0x002C, ControllerInitiated, 400000, AddressingMode7Bit, "\\_SB.I2C0", 0x00, ResourceConsumer, , Exclusive, ) })
            Name (SBFI, ResourceTemplate () { Interrupt (ResourceConsumer, Level, ActiveLow, Exclusive, ,, ) {0x1B} })
            Name (SBFG, ResourceTemplate () { GpioInt (Level, ActiveLow, ExclusiveAndWake, PullDefault, 0x0000, "\\_SB.GPI0", 0x00, ResourceConsumer, ,) {0x0} })
            Method (_CRS, 0, Serialized) {
                If (~CondRefOf (TPDF)) { Name (TPDF, 0xFE) }
                Local0 = Zero
                Local1 = Zero
                Local1 = DerefOf (DerefOf (TPID [Local0]) [Zero])
                While (((Local1 != 0xFE) && (Local1 != TPDF))) {
                    Local0++
                    Local1 = DerefOf (DerefOf (TPID [Local0]) [Zero])
                    If (!(Local0 < SizeOf (TPID))) { Break }
                }
                ADR0 = DerefOf (DerefOf (TPID [Local0]) [One])
                HID2 = DerefOf (DerefOf (TPID [Local0]) [0x02])
                If ((OSYS < 0x07DC)) { Return (SBFI) }
                If (Ones) { Return (ConcatenateResTemplate (SBFB, SBFG)) }
                Return (ConcatenateResTemplate (SBFB, SBFI))
            }
        }
      }
    }
}
