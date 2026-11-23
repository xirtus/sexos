/*
 * SPDX-License-Identifier: GPL-3.0-only
 * Fixture for crates/sex-laptop/tests/awcc_aml.rs: field lists with AccessAs, Connection
 * and extended AccessAs, as laptop DSDTs declare GPIO and I2C fields (the x17 R1's killed
 * the interpreter). Rebuild after editing: iasl -p fields_ssdt fields_ssdt.asl
 */
DefinitionBlock ("", "SSDT", 2, "SEXOS", "FIELDS", 1)
{
    Scope (\_SB)
    {
        Device (GPI0) { Name (_HID, "INT34C5") }
        Device (I2C1) { Name (_HID, "INT34E9") }
        Name (CONB, ResourceTemplate () {
            I2cSerialBusV2 (0x2C, ControllerInitiated, 400000, AddressingMode7Bit, "\\_SB.I2C1", 0x00, ResourceConsumer, , Exclusive, )
        })
        OperationRegion (GPOR, GeneralPurposeIo, Zero, One)
        Field (GPOR, ByteAcc, NoLock, Preserve)
        {
            Connection (GpioIo (Exclusive, PullDefault, 0, 0, IoRestrictionOutputOnly, "\\_SB.GPI0", 0, ResourceConsumer, , ) { 0x10 }),
            GPO1, 1,
            Connection (CONB),
            AccessAs (BufferAcc, AttribBytes (4)),
            SMB1, 8
        }
        OperationRegion (SMBR, GenericSerialBus, Zero, 0x100)
        Field (SMBR, BufferAcc, NoLock, Preserve)
        {
            Connection (CONB),
            AccessAs (BufferAcc, AttribRawProcessBytes (2)),
            RPB0, 8
        }
    }
    Name (DONE, 0x2A)
    Method (CHK, 0, NotSerialized) { Return (DONE + One) }
}
