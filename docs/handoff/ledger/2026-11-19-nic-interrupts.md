# Wired network cards wake SexNet by interrupt (2026-11-19, native drivers N1)

- **Why:** `docs/handoff/NATIVE_DRIVERS_AUDIT_2026_11_19.md` section 3.1. SexNet polled
  every 2 ticks with the link up, stalled 2 ms after every 32-frame burst even while
  frames were waiting (about 190 Mbit/s ceiling, reasoned), and woke 500 times a second
  when idle.
- **Manifest:** `sex_hwdb::manifest::NET` asks for 1 vector per device, so BOOT programs
  MSI-X entry 0 (or MSI) for the NIC, masked. Wi-Fi gets one too and leaves it masked.
- **Card side** (`servers/sexnet/src/ethernet.rs`, `realtek.rs`), only once Running:
  - Intel in MSI mode (I219, igb/igc without MSI-X): stale ICR cleared, IMS = TXDW, LSC,
    RXDMT0, RXO, RXT0.
  - 82574 in MSI-X mode: IVAR 0xE4 puts RX/TX/other on vector 0, CTRL_EXT.PBA_support,
    IMS adds RxQ0/TxQ0/Other.
  - igb/igc (82576, I210, I350, I225) in MSI-X mode: GPIE.Multiple_MSIX|PBA, IVAR0 queue 0
    RX/TX and IVAR_MISC to vector 0, EIAC/EIMS bit 0, IMS = LSC.
  - Realtek r8169/r8125: status cleared, IntrMask = RxOK, RxErr, TxOK, TxErr, RDU,
    LinkChg, RxFIFOOver at the register's own width.
  - Each wake: the raised causes are written back (write one to clear; MSI-X mode does
    not clear on read) before the ring is drained.
- **Service side** (`servers/sexnet/src/main.rs`, `irq_wait.rs`): after a full burst it
  goes straight back to the ring; with the interrupt armed and nothing else pending it
  sleeps `QUIET_TICKS` = 10 instead of 2. If 8 frames turn up after sleeps that ended by
  their deadline and no interrupt was ever seen, it masks the vector and polls as before
  (`[sexnet.irq.silent]`). Records: `[sexnet.irq] armed=1 mode= model= quiet_ticks=10`.
- **Gates:** host tests `tests/sexnet_ethernet.rs` (+4), `tests/sexnet_realtek.rs` (+1),
  `tests/sexnet_irq_wait.rs` (new) in `scripts/sexnet_service_host_gate.sh`;
  `scripts/check_self_wait.py` admits SexNet's 10-tick park and pins the new source.
- **References:** Intel 82574, 82576, I210, I225 datasheets for the cause, IVAR and GPIE
  layouts; Linux `r8169` and `igb` read for the Realtek cause set and the single-vector
  setup (no code copied; ledger table 2 rows already list both drivers).
- **Not yet:** interrupt moderation (ITR/EITR, Realtek IntrMitigate); proof on real
  RTL8125 and I219 parts (QEMU has e1000e and igb models only, its e1000 has no MSI).
