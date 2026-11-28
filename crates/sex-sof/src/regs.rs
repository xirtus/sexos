// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Register map of the Tiger Lake (cAVS 2.5) audio controller, as far as the DSP
//! boot needs it.
//!
//! Two views, both little-endian MMIO:
//!
//! - [`hda`]: BAR0, the HD Audio controller. Bell already drives the codec here. We
//!   only touch the global control, the stream descriptor of the one output stream
//!   we borrow for the code loader (and one input stream for the ICCMAX recipe), the
//!   Intel vendor registers, and the extended capabilities (processing pipe, SPIB).
//! - [`dsp`]: BAR4, the DSP itself: core power/reset/stall, interrupt control, the
//!   IPC doorbells and the SRAM windows (ROM status, mailboxes).
//!
//! Offsets and bits were studied from Linux `sound/soc/sof/intel/hda.h` and
//! `shim.h` and from the HD Audio specification; the values are hardware facts.

/// A byte range inside the BAR4 DSP view (a mailbox or another SRAM window part).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// Offset from the start of BAR4.
    pub offset: u32,
    /// Length in bytes.
    pub size: u32,
}

/// HD Audio registers in BAR0.
pub mod hda {
    /// Global capabilities (16 bit). Bits 11:8 = number of input streams (ISS),
    /// bits 15:12 = number of output streams (OSS).
    pub const GCAP: u32 = 0x00;
    /// Global control (32 bit).
    pub const GCTL: u32 = 0x08;
    /// GCTL bit 0, controller reset: reads 1 once the controller is out of reset and
    /// ready. Bell brings the controller up; the boot only checks this bit.
    pub const GCTL_CRST: u32 = 1 << 0;
    /// Linked-list capability header (32 bit): offset of the first extended
    /// capability in BAR0 (low 16 bits).
    pub const LLCH: u32 = 0x14;
    /// Interrupt control (32 bit). Bit n enables the interrupt of stream index n.
    pub const INTCTL: u32 = 0x20;

    /// First stream descriptor. Streams are numbered inputs first, then outputs.
    pub const SD_BASE: u32 = 0x80;
    /// Size of one stream descriptor.
    pub const SD_STRIDE: u32 = 0x20;
    /// Offset of the stream descriptor with index `index` (0-based over all streams).
    pub const fn sd(index: u32) -> u32 { SD_BASE + SD_STRIDE * index }

    /// SDxCTL, stream control (24 bits, accessed as the low 3 bytes of a 32-bit word).
    pub const SD_CTL: u32 = 0x00;
    /// SDxCTL bit 0, stream reset.
    pub const SD_CTL_SRST: u32 = 1 << 0;
    /// SDxCTL bit 1, stream run (DMA start).
    pub const SD_CTL_RUN: u32 = 1 << 1;
    /// SDxCTL bits 4:2: interrupt on completion, FIFO error, descriptor error.
    pub const SD_CTL_INT_MASK: u32 = 0x1c;
    /// SDxCTL bits 23:20, stream tag.
    pub const SD_CTL_TAG_SHIFT: u32 = 20;
    /// Mask of the stream tag field in SDxCTL.
    pub const SD_CTL_TAG_MASK: u32 = 0xf << SD_CTL_TAG_SHIFT;
    /// SDxSTS, stream status (8 bit). Bits 4:2 are write-1-to-clear interrupt status.
    pub const SD_STS: u32 = 0x03;
    /// SDxSTS interrupt status bits (buffer complete, FIFO error, descriptor error).
    pub const SD_STS_INT_MASK: u8 = 0x1c;
    /// SDxCBL, cyclic buffer length in bytes (32 bit).
    pub const SD_CBL: u32 = 0x08;
    /// SDxLVI, last valid index of the buffer descriptor list (16 bit).
    pub const SD_LVI: u32 = 0x0c;
    /// SDxFIFOS, FIFO size (16 bit).
    pub const SD_FIFOS: u32 = 0x10;
    /// SDxFMT, stream format (16 bit).
    pub const SD_FMT: u32 = 0x12;
    /// SDxBDPL, buffer descriptor list address, low 32 bits (128-byte aligned).
    pub const SD_BDPL: u32 = 0x18;
    /// SDxBDPU, buffer descriptor list address, high 32 bits.
    pub const SD_BDPU: u32 = 0x1c;

    /// The format the code loader stream is programmed with (Linux
    /// `HDA_CL_STREAM_FORMAT`): 48 kHz base, 32-bit containers, one channel. The DSP
    /// ROM only moves bytes; the value just has to be a legal format.
    pub const CL_STREAM_FORMAT: u16 = 0x40;

    /// Intel vendor register EM2 (32 bit).
    pub const VS_EM2: u32 = 0x1030;
    /// EM2 bit 13, DMI link L1 entry allowed. cAVS host DMA streams that are not
    /// "L1 compatible" (the code loader is not) need it cleared while they run.
    pub const VS_EM2_L1SEN: u32 = 1 << 13;
    /// Intel vendor register LTRP (8 bit used): latency tolerance guardband.
    pub const VS_LTRP: u32 = 0x1048;
    /// LTRP guardband field (bits 5:0).
    pub const VS_LTRP_GB_MASK: u8 = 0x3f;
    /// Guardband value recommended by Intel during firmware boot: 95 us.
    pub const LTRP_GB_BOOT_US: u8 = 95;

    /// Extended capability header: bits 27:16 = capability id.
    pub const CAP_ID_SHIFT: u32 = 16;
    /// Extended capability id mask (after shifting).
    pub const CAP_ID_MASK: u32 = 0xfff;
    /// Extended capability header: bits 15:0 = offset of the next capability (0 = end).
    pub const CAP_NEXT_MASK: u32 = 0xffff;
    /// Upper bound on the capability list walk (Linux walks at most 10 + 2).
    pub const MAX_CAPS: u32 = 12;
    /// Capability id of multi-link (ML).
    pub const CAP_ID_ML: u32 = 0x2;
    /// Capability id of the processing pipe (PP): the DSP's hook into the streams.
    pub const CAP_ID_PP: u32 = 0x3;
    /// Capability id of software position in buffer (SPIB).
    pub const CAP_ID_SPIB: u32 = 0x4;
    /// Capability id of DMA resume (DRSM). Not used by the boot.
    pub const CAP_ID_DRSM: u32 = 0x5;

    /// PP capability: PPCTL, processing pipe control (32 bit), offset from the PP
    /// capability header.
    pub const PP_PPCTL: u32 = 0x04;
    /// PPCTL bit 31, processing pipe interrupt enable (the DSP may interrupt the host).
    pub const PPCTL_PIE: u32 = 1 << 31;
    /// PPCTL bit 30, global processing enable: the DSP takes part in stream
    /// processing. Streams stay coupled (plain HDA) until their PROCEN bit is set.
    pub const PPCTL_GPROCEN: u32 = 1 << 30;
    /// PPCTL bit n (n < 30), PROCEN for stream index n: decouple that stream's host
    /// DMA from the link so its data goes to (or comes from) the DSP.
    pub const fn ppctl_procen(index: u32) -> u32 { 1 << index }

    /// SPIB capability: SPBFCCTL, SPIB enable per stream index (32 bit), offset from
    /// the SPIB capability header.
    pub const SPIB_SPBFCCTL: u32 = 0x04;
    /// SPIB capability: offset of the per-stream SPIB register of stream `index`
    /// (32 bit, the software position in the buffer, here the firmware size).
    pub const fn spib(index: u32) -> u32 { 0x08 + 0x08 * index }

    /// One buffer descriptor list entry is 16 bytes: address (64 bit), length (32
    /// bit), flags (bit 0 = interrupt on completion).
    pub const BDLE_SIZE: usize = 16;
    /// Buffer descriptor lists and buffers must start on 128-byte boundaries.
    pub const DMA_ALIGN: u64 = 128;
}

/// DSP registers in BAR4 (cAVS 1.8 and later: Cannon Lake, Ice Lake, Tiger Lake).
pub mod dsp {
    use super::Region;

    /// ADSPCS, DSP core control and status (32 bit).
    pub const ADSPCS: u32 = 0x04;
    /// ADSPCS bits 7:0, CRST: core n held in reset.
    pub const fn adspcs_crst(core_mask: u32) -> u32 { core_mask }
    /// ADSPCS bits 15:8, CSTALL: core n stalled (not executing).
    pub const fn adspcs_cstall(core_mask: u32) -> u32 { core_mask << 8 }
    /// ADSPCS bits 23:16, SPA: host requests power for core n.
    pub const fn adspcs_spa(core_mask: u32) -> u32 { core_mask << 16 }
    /// ADSPCS bits 31:24, CPA: core n is actually powered (follows SPA).
    pub const fn adspcs_cpa(core_mask: u32) -> u32 { core_mask << 24 }
    /// The boot (primary) core. On TGL-H and TGL-LP the host only manages core 0;
    /// the firmware powers the others itself.
    pub const CORE0: u32 = 1 << 0;

    /// ADSPIC, DSP interrupt control (32 bit).
    pub const ADSPIC: u32 = 0x08;
    /// ADSPIS, DSP interrupt status (32 bit).
    pub const ADSPIS: u32 = 0x0c;
    /// ADSPIC/ADSPIS bit 0: IPC interrupt.
    pub const ADSPIC_IPC: u32 = 1 << 0;
    /// ADSPIC/ADSPIS bit 1: code loader DMA interrupt.
    pub const ADSPIC_CL_DMA: u32 = 1 << 1;

    /// HIPCTDR, target (DSP) to host doorbell request (32 bit). The DSP sets BUSY
    /// when it posts a message; the host writes BUSY back (write 1 to clear).
    pub const HIPCTDR: u32 = 0xc0;
    /// HIPCTDA, target to host doorbell acknowledge (32 bit): the host sets DONE when
    /// it has consumed the DSP's message.
    pub const HIPCTDA: u32 = 0xc4;
    /// HIPCTDD, target to host doorbell data (32 bit): extension word of the DSP's
    /// message (for a panic: where the panic record is).
    pub const HIPCTDD: u32 = 0xc8;
    /// HIPCIDR, host to target (DSP) doorbell request (32 bit): the host writes BUSY
    /// (plus an optional message in bits 30:0) to post a message.
    pub const HIPCIDR: u32 = 0xd0;
    /// HIPCIDA, host to target doorbell acknowledge (32 bit): the DSP sets DONE when
    /// it has handled the host's message and the reply is in the mailbox; the host
    /// writes DONE back to clear it.
    pub const HIPCIDA: u32 = 0xd4;
    /// HIPCIDD, host to target doorbell data (32 bit): extension word.
    pub const HIPCIDD: u32 = 0xd8;
    /// HIPCCTL, IPC interrupt control (32 bit).
    pub const HIPCCTL: u32 = 0xe8;

    /// HIPCIDR bit 31: a host message is pending.
    pub const HIPCIDR_BUSY: u32 = 1 << 31;
    /// HIPCIDR message bits.
    pub const HIPCIDR_MSG_MASK: u32 = 0x7fff_ffff;
    /// HIPCIDA bit 31: the DSP has handled the host message.
    pub const HIPCIDA_DONE: u32 = 1 << 31;
    /// HIPCIDA message bits (the ROM's reply code).
    pub const HIPCIDA_MSG_MASK: u32 = 0x7fff_ffff;
    /// HIPCTDR bit 31: a DSP message is pending.
    pub const HIPCTDR_BUSY: u32 = 1 << 31;
    /// HIPCTDR message bits.
    pub const HIPCTDR_MSG_MASK: u32 = 0x7fff_ffff;
    /// HIPCTDA bit 31: the host has consumed the DSP message.
    pub const HIPCTDA_DONE: u32 = 1 << 31;
    /// HIPCTDD message bits.
    pub const HIPCTDD_MSG_MASK: u32 = 0x7fff_ffff;
    /// HIPCCTL bit 0: interrupt when the DSP posts a message (BUSY).
    pub const HIPCCTL_BUSY: u32 = 1 << 0;
    /// HIPCCTL bit 1: interrupt when the DSP acknowledges ours (DONE).
    pub const HIPCCTL_DONE: u32 = 1 << 1;

    /// ROM IPC: "ROM control" command code, written to HIPCIDR with BUSY.
    pub const ROM_IPC_CONTROL: u32 = 0x0100_0000;
    /// ROM IPC flag: purge and load the base firmware from a host DMA stream.
    pub const ROM_IPC_PURGE_FW: u32 = 0x0000_4000;
    /// ROM IPC: the host DMA stream (tag - 1) the firmware will arrive on, bits 13:9.
    pub const ROM_IPC_DMA_SHIFT: u32 = 9;

    /// A DSP-initiated message whose bits 27:12 read this is a firmware panic.
    pub const IPC_PANIC_MAGIC: u32 = 0x0dea_d000;
    /// Mask applied to HIPCTDR before comparing with [`IPC_PANIC_MAGIC`].
    pub const IPC_PANIC_MAGIC_MASK: u32 = 0x0fff_f000;

    /// SSP (I2S) port register blocks start here on cAVS 1.8+.
    pub const SSP_BASE: u32 = 0x10000;
    /// Size of one SSP register block.
    pub const SSP_STRIDE: u32 = 0x1000;
    /// Number of SSP ports on Tiger Lake.
    pub const SSP_COUNT: u32 = 3;
    /// SSC1 register inside an SSP block.
    pub const SSP_SSC1: u32 = 0x04;
    /// SSC1 bits 25:24: bit clock and frame clock are inputs (DSP is consumer). Linux
    /// sets these on every SSP right after core power-up so a codec on an SSP does
    /// not see clocks before the firmware configures the port.
    pub const SSP_SSC1_CONSUMER: u32 = (1 << 25) | (1 << 24);

    /// Start of the SRAM windows in BAR4; window n is at `0x80000 + n * 0x20000`.
    pub const SRAM_WINDOW_BASE: u32 = 0x80000;
    /// Distance between SRAM windows.
    pub const SRAM_WINDOW_STRIDE: u32 = 0x20000;
    /// Number of SRAM windows that fit the 1 MiB BAR4.
    pub const SRAM_WINDOWS: u32 = 4;
    /// Offset of SRAM window `id`.
    pub const fn sram_window(id: u32) -> u32 { SRAM_WINDOW_BASE + SRAM_WINDOW_STRIDE * id }

    /// Window 0 word 0: ROM / firmware status register (FSR). Bits 23:0 state,
    /// 27:24 wait state, 30:28 module, 31 halted.
    pub const SRAM_ROM_STATUS: u32 = SRAM_WINDOW_BASE;
    /// Window 0 word 1: ROM error code (while the ROM runs) or firmware status.
    pub const SRAM_ROM_ERROR: u32 = SRAM_WINDOW_BASE + 0x04;
    /// Mask of the state field of the FSR.
    pub const FSR_STATE_MASK: u32 = 0x00ff_ffff;
    /// FSR bit 31: the DSP halted.
    pub const FSR_HALTED: u32 = 1 << 31;
    /// FSR state: ROM initialised and waiting for the firmware.
    pub const FSR_STATE_INIT_DONE: u32 = 0x1;
    /// FSR state: the base firmware was loaded, verified and entered.
    pub const FSR_STATE_FW_ENTERED: u32 = 0x5;

    /// Where the firmware puts FW_READY (window 0 + 4 KiB, the default upward mailbox).
    pub const MBOX_UPLINK: u32 = SRAM_WINDOW_BASE + 0x1000;
    /// Default DSP-to-host mailbox ("inbox", SOF `UPBOX`): window 0 + 4 KiB, 4 KiB.
    /// The real layout comes with FW_READY; this is what TGL SOF images declare.
    pub const DEFAULT_DSP_BOX: Region = Region { offset: MBOX_UPLINK, size: 0x1000 };
    /// Default host-to-DSP mailbox ("outbox", SOF `DOWNBOX`): window 1, 8 KiB. The
    /// host writes requests here and reads the replies back from the same place.
    pub const DEFAULT_HOST_BOX: Region = Region { offset: sram_window(1), size: 0x2000 };
    /// Size of the BAR4 view.
    pub const BAR4_SIZE: u32 = 0x10_0000;
}

/// Timeouts and poll intervals, in microseconds, as Linux uses them.
pub mod timeout {
    /// Interval between polls of DSP registers.
    pub const POLL_US: u32 = 500;
    /// Controller ready (GCTL.CRST reads 1).
    pub const CTRL_RESET_US: u32 = 100_000;
    /// Core power up (CPA follows SPA), reset enter and reset leave.
    pub const CORE_US: u32 = 50_000;
    /// Core power down (CPA clears).
    pub const CORE_POWER_DOWN_US: u32 = 50_000;
    /// The ROM acknowledges the purge IPC (HIPCIDA.DONE).
    pub const ROM_IPC_US: u32 = 500_000;
    /// The ROM reports INIT_DONE (TGL `rom_init_timeout` = 300 ms).
    pub const ROM_INIT_US: u32 = 300_000;
    /// The ROM loads, verifies and enters the base firmware (FW_ENTERED).
    pub const BASEFW_US: u32 = 3_000_000;
    /// A stream descriptor starts or stops running, or enters or leaves reset.
    pub const STREAM_US: u32 = 300;
    /// Interval between polls of stream registers.
    pub const STREAM_POLL_US: u32 = 10;
    /// The firmware posts FW_READY after FW_ENTERED (Linux boot timeout, 2 s).
    pub const FW_READY_US: u32 = 2_000_000;
    /// The firmware answers an IPC (Linux IPC timeout, 500 ms).
    pub const IPC_US: u32 = 500_000;
    /// Linux tries the ROM handshake this many times before giving up.
    pub const BOOT_ATTEMPTS: u32 = 3;
}
