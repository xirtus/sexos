// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Sound Open Firmware IPC version 3 messages, as little-endian bytes.
//!
//! IPC3 is the message ABI that Tiger Lake's SOF firmware (`sof-tgl.ri`,
//! `sof-tgl-h.ri`) speaks by default. Every message starts with a command header
//! (`size`, `cmd`): `size` is the length of the whole message in bytes and `cmd`
//! is a global type in the top four bits ([`GLB_TPLG_MSG`], [`GLB_STREAM_MSG`], ...)
//! ORed with a command inside that type ([`TPLG_COMP_NEW`], [`STREAM_TRIG_START`],
//! ...). The DSP answers each host message with a reply that starts with the same
//! header plus a signed error code.
//!
//! This module only turns values into bytes and bytes into values. It does not
//! touch hardware: [`crate::boot`] owns the mailbox windows and the doorbells.
//!
//! - Encoders write one message into a caller buffer and return its length. They
//!   zero the whole message first, so reserved fields are always zero, and fail
//!   with [`Error::BufferTooSmall`] instead of writing a partial message.
//! - Decoders read replies and notifications from the DSP's mailbox bytes and
//!   check that the bytes are long enough before reading a field.
//!
//! The layouts mirror the C structures in SOF's `src/include/ipc/*.h` and Linux's
//! `include/sound/sof/*.h` (both BSD-3-Clause; all structures are packed). The
//! sizes asserted in `tests/ipc3_*.rs` were measured by compiling those headers.
//! How the fields are filled follows Linux `sound/soc/sof/ipc3*.c`.

/// SOF ABI major version this crate speaks.
pub const ABI_MAJOR: u32 = 3;
/// SOF ABI minor version the layouts were taken from (Linux `uapi/sound/sof/abi.h`).
pub const ABI_MINOR: u32 = 23;
/// SOF ABI patch version the layouts were taken from.
pub const ABI_PATCH: u32 = 1;

/// Pack an ABI version the way SOF does: major in bits 31..24, minor in 23..12,
/// patch in 11..0.
pub const fn abi_version(major: u32, minor: u32, patch: u32) -> u32 {
    ((major & 0xff) << 24) | ((minor & 0xfff) << 12) | (patch & 0xfff)
}

/// The ABI version this crate was written against (3.23.1).
pub const ABI_VERSION: u32 = abi_version(ABI_MAJOR, ABI_MINOR, ABI_PATCH);

/// Major part of a packed ABI version.
pub const fn abi_major(v: u32) -> u32 {
    (v >> 24) & 0xff
}
/// Minor part of a packed ABI version.
pub const fn abi_minor(v: u32) -> u32 {
    (v >> 12) & 0xfff
}
/// Patch part of a packed ABI version.
pub const fn abi_patch(v: u32) -> u32 {
    v & 0xfff
}

/// Whether firmware reporting `fw_abi` can be driven by this crate. Like Linux,
/// only the major number has to match; a newer minor keeps the old layouts.
pub const fn abi_compatible(fw_abi: u32) -> bool {
    abi_major(fw_abi) == ABI_MAJOR
}

/// The magic number at the start of SOF ABI data blobs ("SOF\0").
pub const ABI_MAGIC: u32 = 0x0046_4F53;

/// Largest message either side may put in a mailbox, in bytes.
pub const MSG_MAX_SIZE: usize = 384;

// ---------------------------------------------------------------------------
// Command words
// ---------------------------------------------------------------------------

/// Shift of the global type inside `cmd`.
pub const GLB_TYPE_SHIFT: u32 = 28;
/// Mask of the global type inside `cmd`.
pub const GLB_TYPE_MASK: u32 = 0xf << GLB_TYPE_SHIFT;
/// Shift of the command inside `cmd`.
pub const CMD_TYPE_SHIFT: u32 = 16;
/// Mask of the command inside `cmd`.
pub const CMD_TYPE_MASK: u32 = 0xfff << CMD_TYPE_SHIFT;

const fn glb(x: u32) -> u32 {
    x << GLB_TYPE_SHIFT
}
const fn cmd(x: u32) -> u32 {
    x << CMD_TYPE_SHIFT
}

/// A reply from the DSP to a host message.
pub const GLB_REPLY: u32 = glb(0x1);
/// Several messages in one mailbox write.
pub const GLB_COMPOUND: u32 = glb(0x2);
/// Building or tearing down the audio graph.
pub const GLB_TPLG_MSG: u32 = glb(0x3);
/// Power management.
pub const GLB_PM_MSG: u32 = glb(0x4);
/// Runtime values of one component (volume, switches, data blobs).
pub const GLB_COMP_MSG: u32 = glb(0x5);
/// PCM streams: parameters, triggers, positions.
pub const GLB_STREAM_MSG: u32 = glb(0x6);
/// Sent once by the firmware when it has booted.
pub const FW_READY: u32 = glb(0x7);
/// Configuring a digital audio interface (SSP, DMIC, HDA link).
pub const GLB_DAI_MSG: u32 = glb(0x8);
/// Firmware trace (log) DMA.
pub const GLB_TRACE_MSG: u32 = glb(0x9);
/// Debugger.
pub const GLB_GDB_DEBUG: u32 = glb(0xA);
/// Test messages.
pub const GLB_TEST_MSG: u32 = glb(0xB);
/// Data probes.
pub const GLB_PROBE: u32 = glb(0xC);
/// Debug queries.
pub const GLB_DEBUG: u32 = glb(0xD);

/// Create a component (host, DAI, volume, ...).
pub const TPLG_COMP_NEW: u32 = cmd(0x001);
/// Free a component.
pub const TPLG_COMP_FREE: u32 = cmd(0x002);
/// Connect a buffer and a component.
pub const TPLG_COMP_CONNECT: u32 = cmd(0x003);
/// Create a pipeline (the scheduling unit).
pub const TPLG_PIPE_NEW: u32 = cmd(0x010);
/// Free a pipeline.
pub const TPLG_PIPE_FREE: u32 = cmd(0x011);
/// Connect two pipelines.
pub const TPLG_PIPE_CONNECT: u32 = cmd(0x012);
/// Tell the firmware a pipeline has all its parts and connections.
pub const TPLG_PIPE_COMPLETE: u32 = cmd(0x013);
/// Create a buffer between two components.
pub const TPLG_BUFFER_NEW: u32 = cmd(0x020);
/// Free a buffer.
pub const TPLG_BUFFER_FREE: u32 = cmd(0x021);

/// Save the DSP context before suspend.
pub const PM_CTX_SAVE: u32 = cmd(0x001);
/// Restore the DSP context after resume.
pub const PM_CTX_RESTORE: u32 = cmd(0x002);
/// Ask for the context size.
pub const PM_CTX_SIZE: u32 = cmd(0x003);
/// Set a clock.
pub const PM_CLK_SET: u32 = cmd(0x004);
/// Read a clock.
pub const PM_CLK_GET: u32 = cmd(0x005);
/// Request a clock.
pub const PM_CLK_REQ: u32 = cmd(0x006);
/// Enable or disable DSP cores.
pub const PM_CORE_ENABLE: u32 = cmd(0x007);
/// Power and clock gating policy.
pub const PM_GATE: u32 = cmd(0x008);

/// Set a component value.
pub const COMP_SET_VALUE: u32 = cmd(0x001);
/// Read a component value.
pub const COMP_GET_VALUE: u32 = cmd(0x002);
/// Set a component data blob.
pub const COMP_SET_DATA: u32 = cmd(0x003);
/// Read a component data blob.
pub const COMP_GET_DATA: u32 = cmd(0x004);
/// A component notification from the DSP.
pub const COMP_NOTIFICATION: u32 = cmd(0x005);

/// Configure a DAI.
pub const DAI_CONFIG: u32 = cmd(0x001);
/// DAI loopback.
pub const DAI_LOOPBACK: u32 = cmd(0x002);

/// Set the parameters of a PCM stream (format, rate, host ring).
pub const STREAM_PCM_PARAMS: u32 = cmd(0x001);
/// The DSP's answer to [`STREAM_PCM_PARAMS`].
pub const STREAM_PCM_PARAMS_REPLY: u32 = cmd(0x002);
/// Release a PCM stream's parameters.
pub const STREAM_PCM_FREE: u32 = cmd(0x003);
/// Start a stream.
pub const STREAM_TRIG_START: u32 = cmd(0x004);
/// Stop a stream.
pub const STREAM_TRIG_STOP: u32 = cmd(0x005);
/// Pause a stream.
pub const STREAM_TRIG_PAUSE: u32 = cmd(0x006);
/// Resume a paused stream.
pub const STREAM_TRIG_RELEASE: u32 = cmd(0x007);
/// Drain a stream.
pub const STREAM_TRIG_DRAIN: u32 = cmd(0x008);
/// Overrun or underrun notification from the DSP.
pub const STREAM_TRIG_XRUN: u32 = cmd(0x009);
/// Stream position notification from the DSP.
pub const STREAM_POSITION: u32 = cmd(0x00a);

/// Set up the trace DMA.
pub const TRACE_DMA_PARAMS: u32 = cmd(0x001);
/// Trace DMA position notification.
pub const TRACE_DMA_POSITION: u32 = cmd(0x002);
/// Set up the trace DMA (extended form).
pub const TRACE_DMA_PARAMS_EXT: u32 = cmd(0x003);
/// Change the trace filter (ABI 3.17).
pub const TRACE_FILTER_UPDATE: u32 = cmd(0x004);
/// Free the trace DMA (ABI 3.20).
pub const TRACE_DMA_FREE: u32 = cmd(0x005);

/// The global type part of a command word.
pub const fn glb_type(cmd_word: u32) -> u32 {
    cmd_word & GLB_TYPE_MASK
}
/// The command part of a command word.
pub const fn cmd_type(cmd_word: u32) -> u32 {
    cmd_word & CMD_TYPE_MASK
}

// ---------------------------------------------------------------------------
// Enumerations used inside messages
// ---------------------------------------------------------------------------

/// Component type: host DMA endpoint (`SOF_COMP_HOST`).
pub const COMP_HOST: u32 = 1;
/// Component type: DAI endpoint (`SOF_COMP_DAI`).
pub const COMP_DAI: u32 = 2;
/// Component type: buffer (`SOF_COMP_BUFFER`).
pub const COMP_BUFFER: u32 = 12;

/// DAI type: Intel SSP (I2S).
pub const DAI_INTEL_SSP: u32 = 1;
/// DAI type: Intel digital microphone controller.
pub const DAI_INTEL_DMIC: u32 = 2;
/// DAI type: Intel HD Audio link.
pub const DAI_INTEL_HDA: u32 = 3;
/// DAI type: Intel SoundWire link (ALH).
pub const DAI_INTEL_ALH: u32 = 4;

/// DAI config sent with no stage information.
pub const DAI_CONFIG_FLAGS_NONE: u8 = 0;
/// DAI config sent while setting stream parameters.
pub const DAI_CONFIG_FLAGS_HW_PARAMS: u8 = 1 << 0;
/// DAI config sent while releasing stream parameters.
pub const DAI_CONFIG_FLAGS_HW_FREE: u8 = 1 << 1;
/// DAI config sent on pause (ABI 3.20).
pub const DAI_CONFIG_FLAGS_PAUSE: u8 = 1 << 2;

/// Number of PDM controllers in Intel's DMIC block.
pub const DMIC_NUM_CTRL: usize = 4;

/// Stream direction: playback (host to DSP).
pub const STREAM_PLAYBACK: u32 = 0;
/// Stream direction: capture (DSP to host).
pub const STREAM_CAPTURE: u32 = 1;

/// Sample format: signed 16-bit little-endian.
pub const FRAME_S16_LE: u32 = 0;
/// Sample format: signed 24-bit in a 32-bit little-endian container.
pub const FRAME_S24_4LE: u32 = 1;
/// Sample format: signed 32-bit little-endian.
pub const FRAME_S32_LE: u32 = 2;
/// Sample format: 32-bit float.
pub const FRAME_FLOAT: u32 = 3;

/// Interleaved samples (L R L R ...).
pub const BUFFER_INTERLEAVED: u32 = 0;
/// Non-interleaved samples.
pub const BUFFER_NONINTERLEAVED: u32 = 1;

/// Most channels a stream can carry in IPC3.
pub const MAX_CHANNELS: usize = 8;

/// Buffer memory capability: RAM.
pub const MEM_CAPS_RAM: u32 = 1 << 0;
/// Buffer memory capability: ROM.
pub const MEM_CAPS_ROM: u32 = 1 << 1;
/// Buffer memory capability: external memory.
pub const MEM_CAPS_EXT: u32 = 1 << 2;
/// Buffer memory capability: low power memory.
pub const MEM_CAPS_LP: u32 = 1 << 3;
/// Buffer memory capability: high performance memory.
pub const MEM_CAPS_HP: u32 = 1 << 4;
/// Buffer memory capability: reachable by DMA.
pub const MEM_CAPS_DMA: u32 = 1 << 5;
/// Buffer memory capability: cacheable.
pub const MEM_CAPS_CACHE: u32 = 1 << 6;
/// Buffer memory capability: executable.
pub const MEM_CAPS_EXEC: u32 = 1 << 7;
/// Buffer memory capability: L3 memory.
pub const MEM_CAPS_L3: u32 = 1 << 8;

/// Pipeline scheduled from a DMA interrupt.
pub const TIME_DOMAIN_DMA: u32 = 0;
/// Pipeline scheduled from a timer.
pub const TIME_DOMAIN_TIMER: u32 = 1;

/// PCM flag: stop the stream on any overrun or underrun.
pub const PCM_FLAG_XRUN_STOP: u32 = 1 << 0;

/// Extended FW_READY data: memory windows.
pub const EXT_WINDOW: u32 = 1;
/// Extended FW_READY data: compiler information.
pub const EXT_CC_INFO: u32 = 2;
/// Extended FW_READY data: probe support.
pub const EXT_PROBE_INFO: u32 = 3;
/// Extended FW_READY data: user ABI information.
pub const EXT_USER_ABI_INFO: u32 = 4;

/// Window region: host-to-DSP mailbox (Linux calls it the outbox).
pub const REGION_DOWNBOX: u32 = 0;
/// Window region: DSP-to-host mailbox (Linux calls it the inbox).
pub const REGION_UPBOX: u32 = 1;
/// Window region: early trace.
pub const REGION_TRACE: u32 = 2;
/// Window region: debug (firmware status, panic codes).
pub const REGION_DEBUG: u32 = 3;
/// Window region: stream positions.
pub const REGION_STREAM: u32 = 4;
/// Window region: registers.
pub const REGION_REGS: u32 = 5;
/// Window region: exception dump.
pub const REGION_EXCEPTION: u32 = 6;

/// Most windows FW_READY can describe.
pub const MAX_WINDOWS: usize = 16;

// ---------------------------------------------------------------------------
// Sizes of the C structures (packed), in bytes
// ---------------------------------------------------------------------------

/// `struct sof_ipc_cmd_hdr`.
pub const CMD_HDR_SIZE: usize = 8;
/// `struct sof_ipc_reply`.
pub const REPLY_SIZE: usize = 12;
/// `struct sof_ipc_comp`.
pub const COMP_SIZE: usize = 28;
/// `struct sof_ipc_comp_config`.
pub const COMP_CONFIG_SIZE: usize = 36;
/// `struct sof_ipc_buffer`.
pub const BUFFER_SIZE: usize = 44;
/// `struct sof_ipc_comp_host`, without the optional UUID.
pub const COMP_HOST_SIZE: usize = 76;
/// `struct sof_ipc_comp_dai`, without the optional UUID.
pub const COMP_DAI_SIZE: usize = 80;
/// The component UUID Linux appends after a component as extended data.
pub const UUID_SIZE: usize = 16;
/// `struct sof_ipc_free`.
pub const FREE_SIZE: usize = 12;
/// `struct sof_ipc_pipe_new`.
pub const PIPE_NEW_SIZE: usize = 48;
/// `struct sof_ipc_pipe_ready` (pipeline complete).
pub const PIPE_READY_SIZE: usize = 12;
/// `struct sof_ipc_pipe_comp_connect`.
pub const COMP_CONNECT_SIZE: usize = 16;
/// `struct sof_ipc_dai_dmic_pdm_ctrl`.
pub const DMIC_PDM_CTRL_SIZE: usize = 24;
/// `struct sof_ipc_dai_dmic_params`.
pub const DMIC_PARAMS_SIZE: usize = 164;
/// `struct sof_ipc_dai_config`; the union is as large as its DMIC member.
pub const DAI_CONFIG_SIZE: usize = 216;
/// `struct sof_ipc_host_buffer`.
pub const HOST_BUFFER_SIZE: usize = 28;
/// `struct sof_ipc_stream_params`, without extended data.
pub const STREAM_PARAMS_SIZE: usize = 84;
/// `struct sof_ipc_pcm_params`.
pub const PCM_PARAMS_SIZE: usize = 108;
/// `struct sof_ipc_pcm_params_reply`.
pub const PCM_PARAMS_REPLY_SIZE: usize = 20;
/// `struct sof_ipc_stream` (triggers and PCM free).
pub const STREAM_SIZE: usize = 12;
/// `struct sof_ipc_stream_posn`.
pub const STREAM_POSN_SIZE: usize = 76;
/// `struct sof_ipc_fw_version`.
pub const FW_VERSION_SIZE: usize = 60;
/// `struct sof_ipc_fw_ready`.
pub const FW_READY_SIZE: usize = 108;
/// `struct sof_ipc_ext_data_hdr`.
pub const EXT_DATA_HDR_SIZE: usize = 12;
/// `struct sof_ipc_window_elem`.
pub const WINDOW_ELEM_SIZE: usize = 24;
/// `struct sof_ipc_window` with all sixteen elements.
pub const WINDOW_SIZE: usize = 400;
/// `struct sof_ipc_pm_ctx` without elements.
pub const PM_CTX_SIZE_BYTES: usize = 76;
/// `struct sof_ipc_pm_core_config`.
pub const PM_CORE_CONFIG_SIZE: usize = 12;
/// `struct sof_ipc_pm_gate`.
pub const PM_GATE_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// What can go wrong while encoding or decoding a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The output buffer cannot hold the message.
    BufferTooSmall { need: usize, have: usize },
    /// The input ends before the structure does.
    Truncated { need: usize, have: usize },
    /// The header's `size` field is smaller than the structure or larger than
    /// the bytes given.
    BadSize(u32),
    /// The command word is not the one this decoder reads.
    UnexpectedCmd(u32),
    /// The firmware answered with a negative error code.
    Firmware(i32),
    /// A value does not fit the message (named for the log).
    Invalid(&'static str),
}

// ---------------------------------------------------------------------------
// Little-endian helpers
// ---------------------------------------------------------------------------

fn start(buf: &mut [u8], size: usize, cmd_word: u32) -> Result<(), Error> {
    if buf.len() < size {
        return Err(Error::BufferTooSmall { need: size, have: buf.len() });
    }
    buf[..size].fill(0);
    put_u32(buf, 0, size as u32);
    put_u32(buf, 4, cmd_word);
    Ok(())
}

fn put_u8(buf: &mut [u8], off: usize, v: u8) {
    buf[off] = v;
}
fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn need(bytes: &[u8], n: usize) -> Result<(), Error> {
    if bytes.len() < n {
        Err(Error::Truncated { need: n, have: bytes.len() })
    } else {
        Ok(())
    }
}
fn get_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn get_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn get_u64(b: &[u8], off: usize) -> u64 {
    (get_u32(b, off) as u64) | ((get_u32(b, off + 4) as u64) << 32)
}

// ---------------------------------------------------------------------------
// Topology messages
// ---------------------------------------------------------------------------

/// The `sof_ipc_comp_config` block shared by host and DAI components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CompConfig {
    /// Periods in the sink buffer (0 means variable).
    pub periods_sink: u32,
    /// Periods in the source buffer (0 means variable).
    pub periods_source: u32,
    /// One of the `FRAME_*` sample formats.
    pub frame_fmt: u32,
    /// What to do on overrun or underrun (0: nothing special).
    pub xrun_action: u32,
}

fn put_comp(buf: &mut [u8], id: u32, comp_type: u32, pipeline_id: u32, core: u32, ext: u32) {
    put_u32(buf, 8, id);
    put_u32(buf, 12, comp_type);
    put_u32(buf, 16, pipeline_id);
    put_u32(buf, 20, core);
    put_u32(buf, 24, ext);
}

fn put_config(buf: &mut [u8], c: &CompConfig) {
    // The config block carries its own header; Linux sets only its size.
    let o = COMP_SIZE;
    put_u32(buf, o, COMP_CONFIG_SIZE as u32);
    put_u32(buf, o + 8, c.periods_sink);
    put_u32(buf, o + 12, c.periods_source);
    put_u32(buf, o + 20, c.frame_fmt);
    put_u32(buf, o + 24, c.xrun_action);
}

/// `TPLG_BUFFER_NEW`: a buffer between two components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferNew {
    /// Component id of the buffer.
    pub id: u32,
    /// Pipeline the buffer belongs to.
    pub pipeline_id: u32,
    /// DSP core the buffer lives on.
    pub core: u32,
    /// Size in bytes.
    pub size: u32,
    /// `MEM_CAPS_*` bits for where the firmware may place it.
    pub caps: u32,
    /// Overrun and underrun permissions (`SOF_BUF_*`).
    pub flags: u32,
}

/// Write a `TPLG_BUFFER_NEW` message (44 bytes).
pub fn encode_buffer_new(buf: &mut [u8], b: &BufferNew) -> Result<usize, Error> {
    start(buf, BUFFER_SIZE, GLB_TPLG_MSG | TPLG_BUFFER_NEW)?;
    put_comp(buf, b.id, COMP_BUFFER, b.pipeline_id, b.core, 0);
    put_u32(buf, 28, b.size);
    put_u32(buf, 32, b.caps);
    put_u32(buf, 36, b.flags);
    Ok(BUFFER_SIZE)
}

/// `TPLG_COMP_NEW` for a host component: the DSP end of a host DMA stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostComp {
    /// Component id; PCM parameters and triggers name the stream by this id.
    pub id: u32,
    /// Pipeline the component belongs to.
    pub pipeline_id: u32,
    /// DSP core.
    pub core: u32,
    /// Period counts and sample format.
    pub config: CompConfig,
    /// [`STREAM_PLAYBACK`] or [`STREAM_CAPTURE`].
    pub direction: u32,
    /// 1 to stop the periodic interrupt to the host.
    pub no_irq: u32,
    /// DMA engine specific value.
    pub dmac_config: u32,
    /// Component UUID from the topology, appended as extended data when present.
    /// Current firmware finds the component driver by this UUID.
    pub uuid: Option<[u8; UUID_SIZE]>,
}

/// Write a host `TPLG_COMP_NEW` (76 bytes, or 92 with the UUID).
pub fn encode_host_comp(buf: &mut [u8], h: &HostComp) -> Result<usize, Error> {
    let size = COMP_HOST_SIZE + if h.uuid.is_some() { UUID_SIZE } else { 0 };
    start(buf, size, GLB_TPLG_MSG | TPLG_COMP_NEW)?;
    let ext = if h.uuid.is_some() { UUID_SIZE as u32 } else { 0 };
    put_comp(buf, h.id, COMP_HOST, h.pipeline_id, h.core, ext);
    put_config(buf, &h.config);
    put_u32(buf, 64, h.direction);
    put_u32(buf, 68, h.no_irq);
    put_u32(buf, 72, h.dmac_config);
    if let Some(u) = h.uuid {
        buf[COMP_HOST_SIZE..size].copy_from_slice(&u);
    }
    Ok(size)
}

/// `TPLG_COMP_NEW` for a DAI component: the DSP end of a hardware interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaiComp {
    /// Component id.
    pub id: u32,
    /// Pipeline the component belongs to.
    pub pipeline_id: u32,
    /// DSP core.
    pub core: u32,
    /// Period counts and sample format.
    pub config: CompConfig,
    /// [`STREAM_PLAYBACK`] or [`STREAM_CAPTURE`].
    pub direction: u32,
    /// Which DAI of this type (DMIC 0 is the 48 kHz FIFO A).
    pub dai_index: u32,
    /// One of the `DAI_INTEL_*` types.
    pub dai_type: u32,
    /// Component UUID, appended as extended data when present.
    pub uuid: Option<[u8; UUID_SIZE]>,
}

/// Write a DAI `TPLG_COMP_NEW` (80 bytes, or 96 with the UUID).
pub fn encode_dai_comp(buf: &mut [u8], d: &DaiComp) -> Result<usize, Error> {
    let size = COMP_DAI_SIZE + if d.uuid.is_some() { UUID_SIZE } else { 0 };
    start(buf, size, GLB_TPLG_MSG | TPLG_COMP_NEW)?;
    let ext = if d.uuid.is_some() { UUID_SIZE as u32 } else { 0 };
    put_comp(buf, d.id, COMP_DAI, d.pipeline_id, d.core, ext);
    put_config(buf, &d.config);
    put_u32(buf, 64, d.direction);
    put_u32(buf, 68, d.dai_index);
    put_u32(buf, 72, d.dai_type);
    if let Some(u) = d.uuid {
        buf[COMP_DAI_SIZE..size].copy_from_slice(&u);
    }
    Ok(size)
}

/// `TPLG_PIPE_NEW`: a pipeline, the unit the firmware schedules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeNew {
    /// Component id of the pipeline itself (used by complete and free).
    pub comp_id: u32,
    /// Pipeline id that components name in their `pipeline_id`.
    pub pipeline_id: u32,
    /// Component that drives scheduling (the DAI, for a capture pipeline).
    pub sched_id: u32,
    /// DSP core the pipeline runs on.
    pub core: u32,
    /// Execution period in microseconds.
    pub period_us: u32,
    /// Priority, 0 (low) to 10 (high).
    pub priority: u32,
    /// Worst case instruction count per period, in thousands.
    pub period_mips: u32,
    /// Output frames per run, 0 for variable.
    pub frames_per_sched: u32,
    /// Report xruns longer than this, in microseconds.
    pub xrun_limit_us: u32,
    /// [`TIME_DOMAIN_DMA`] or [`TIME_DOMAIN_TIMER`].
    pub time_domain: u32,
}

/// Write a `TPLG_PIPE_NEW` message (48 bytes).
pub fn encode_pipe_new(buf: &mut [u8], p: &PipeNew) -> Result<usize, Error> {
    start(buf, PIPE_NEW_SIZE, GLB_TPLG_MSG | TPLG_PIPE_NEW)?;
    put_u32(buf, 8, p.comp_id);
    put_u32(buf, 12, p.pipeline_id);
    put_u32(buf, 16, p.sched_id);
    put_u32(buf, 20, p.core);
    put_u32(buf, 24, p.period_us);
    put_u32(buf, 28, p.priority);
    put_u32(buf, 32, p.period_mips);
    put_u32(buf, 36, p.frames_per_sched);
    put_u32(buf, 40, p.xrun_limit_us);
    put_u32(buf, 44, p.time_domain);
    Ok(PIPE_NEW_SIZE)
}

/// Write a `TPLG_COMP_CONNECT` message (16 bytes). IPC3 only connects a buffer
/// to a component or a component to a buffer, never two components directly.
pub fn encode_comp_connect(buf: &mut [u8], source_id: u32, sink_id: u32) -> Result<usize, Error> {
    start(buf, COMP_CONNECT_SIZE, GLB_TPLG_MSG | TPLG_COMP_CONNECT)?;
    put_u32(buf, 8, source_id);
    put_u32(buf, 12, sink_id);
    Ok(COMP_CONNECT_SIZE)
}

/// Write a `TPLG_PIPE_COMPLETE` message (12 bytes) for the pipeline whose
/// component id is `pipe_comp_id`.
pub fn encode_pipe_complete(buf: &mut [u8], pipe_comp_id: u32) -> Result<usize, Error> {
    start(buf, PIPE_READY_SIZE, GLB_TPLG_MSG | TPLG_PIPE_COMPLETE)?;
    put_u32(buf, 8, pipe_comp_id);
    Ok(PIPE_READY_SIZE)
}

fn encode_free(buf: &mut [u8], what: u32, id: u32) -> Result<usize, Error> {
    start(buf, FREE_SIZE, GLB_TPLG_MSG | what)?;
    put_u32(buf, 8, id);
    Ok(FREE_SIZE)
}

/// Write a `TPLG_COMP_FREE` message (12 bytes).
pub fn encode_comp_free(buf: &mut [u8], comp_id: u32) -> Result<usize, Error> {
    encode_free(buf, TPLG_COMP_FREE, comp_id)
}
/// Write a `TPLG_BUFFER_FREE` message (12 bytes).
pub fn encode_buffer_free(buf: &mut [u8], buffer_id: u32) -> Result<usize, Error> {
    encode_free(buf, TPLG_BUFFER_FREE, buffer_id)
}
/// Write a `TPLG_PIPE_FREE` message (12 bytes) for the pipeline's component id.
pub fn encode_pipe_free(buf: &mut [u8], pipe_comp_id: u32) -> Result<usize, Error> {
    encode_free(buf, TPLG_PIPE_FREE, pipe_comp_id)
}

// ---------------------------------------------------------------------------
// DAI config (DMIC)
// ---------------------------------------------------------------------------

/// One PDM controller: a pair of microphones sharing a clock and data line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DmicPdm {
    /// Controller number (0..3).
    pub id: u16,
    /// Use microphone A (left, clock rising edge).
    pub enable_mic_a: u16,
    /// Use microphone B (right, clock falling edge).
    pub enable_mic_b: u16,
    /// Invert microphone A.
    pub polarity_mic_a: u16,
    /// Invert microphone B.
    pub polarity_mic_b: u16,
    /// Swap the data clock edge.
    pub clk_edge: u16,
    /// Data sampling skew against the clock (0..15).
    pub skew: u16,
}

/// `sof_ipc_dai_dmic_params`: the DMIC controller's global settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmicParams {
    /// Must match the firmware DMIC driver's version (1).
    pub driver_ipc_version: u32,
    /// Lowest microphone clock in Hz.
    pub pdmclk_min: u32,
    /// Highest microphone clock in Hz.
    pub pdmclk_max: u32,
    /// FIFO sample rate in Hz.
    pub fifo_fs: u32,
    /// FIFO word length (16 or 32).
    pub fifo_bits: u16,
    /// Deprecated since ABI 3.0.1; Linux copies `fifo_bits` here only for older
    /// firmware.
    pub fifo_bits_b: u16,
    /// Lowest clock duty cycle in percent.
    pub duty_min: u16,
    /// Highest clock duty cycle in percent.
    pub duty_max: u16,
    /// How many entries of `pdm` are in use (1..4).
    pub num_pdm_active: u32,
    /// Microphone wake-up time in microseconds (audio is muted meanwhile).
    pub wake_up_time_us: u32,
    /// Shortest time the clock stays on, in microseconds.
    pub min_clock_on_time_us: u32,
    /// Length of the unmute gain ramp in milliseconds.
    pub unmute_ramp_time_ms: u32,
    /// Per-controller settings; entries past `num_pdm_active` are ignored.
    pub pdm: [DmicPdm; DMIC_NUM_CTRL],
}

/// `GLB_DAI_MSG | DAI_CONFIG` for a DMIC DAI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaiConfigDmic {
    /// Which DMIC DAI (0: FIFO A, 1: FIFO B).
    pub dai_index: u32,
    /// `SOF_DAI_FMT_*` bits from the topology link (0 for DMIC).
    pub format: u16,
    /// DAI group, 0 for none.
    pub group_id: u8,
    /// One of the `DAI_CONFIG_FLAGS_*` stages.
    pub flags: u8,
    /// Controller settings.
    pub dmic: DmicParams,
}

/// Write a DMIC `DAI_CONFIG` message (216 bytes: the full union size, as Linux
/// sends it).
pub fn encode_dai_config_dmic(buf: &mut [u8], c: &DaiConfigDmic) -> Result<usize, Error> {
    let m = &c.dmic;
    if m.num_pdm_active as usize > DMIC_NUM_CTRL {
        return Err(Error::Invalid("dmic num_pdm_active"));
    }
    start(buf, DAI_CONFIG_SIZE, GLB_DAI_MSG | DAI_CONFIG)?;
    put_u32(buf, 8, DAI_INTEL_DMIC);
    put_u32(buf, 12, c.dai_index);
    put_u16(buf, 16, c.format);
    put_u8(buf, 18, c.group_id);
    put_u8(buf, 19, c.flags);
    // Union at 52. Linux leaves the DMIC block's own header size at zero.
    let o = 52;
    put_u32(buf, o + 4, m.driver_ipc_version);
    put_u32(buf, o + 8, m.pdmclk_min);
    put_u32(buf, o + 12, m.pdmclk_max);
    put_u32(buf, o + 16, m.fifo_fs);
    put_u16(buf, o + 24, m.fifo_bits);
    put_u16(buf, o + 26, m.fifo_bits_b);
    put_u16(buf, o + 28, m.duty_min);
    put_u16(buf, o + 30, m.duty_max);
    put_u32(buf, o + 32, m.num_pdm_active);
    put_u32(buf, o + 36, m.wake_up_time_us);
    put_u32(buf, o + 40, m.min_clock_on_time_us);
    put_u32(buf, o + 44, m.unmute_ramp_time_ms);
    for (i, p) in m.pdm.iter().enumerate().take(m.num_pdm_active as usize) {
        let q = o + 68 + i * DMIC_PDM_CTRL_SIZE;
        put_u16(buf, q + 4, p.id);
        put_u16(buf, q + 6, p.enable_mic_a);
        put_u16(buf, q + 8, p.enable_mic_b);
        put_u16(buf, q + 10, p.polarity_mic_a);
        put_u16(buf, q + 12, p.polarity_mic_b);
        put_u16(buf, q + 14, p.clk_edge);
        put_u16(buf, q + 16, p.skew);
    }
    Ok(DAI_CONFIG_SIZE)
}

// ---------------------------------------------------------------------------
// Stream messages
// ---------------------------------------------------------------------------

/// `sof_ipc_host_buffer`: where the host ring is, via its page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HostBuffer {
    /// Bus address of the compressed page table (see
    /// [`crate::topology::encode_page_table`]).
    pub phy_addr: u32,
    /// Number of 4 KiB pages in the ring.
    pub pages: u32,
    /// Ring size in bytes.
    pub size: u32,
}

/// `STREAM_PCM_PARAMS`: the stream format and host ring of a host component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmParams {
    /// Host component id.
    pub comp_id: u32,
    /// `PCM_FLAG_*` bits.
    pub flags: u32,
    /// The host ring.
    pub buffer: HostBuffer,
    /// [`STREAM_PLAYBACK`] or [`STREAM_CAPTURE`].
    pub direction: u32,
    /// One of the `FRAME_*` formats.
    pub frame_fmt: u32,
    /// [`BUFFER_INTERLEAVED`] or [`BUFFER_NONINTERLEAVED`].
    pub buffer_fmt: u32,
    /// Sample rate in Hz.
    pub rate: u32,
    /// HD Audio stream tag of the decoupled host DMA stream (1..15).
    pub stream_tag: u16,
    /// Channel count.
    pub channels: u16,
    /// Meaningful bytes per sample.
    pub sample_valid_bytes: u16,
    /// Bytes per sample in memory.
    pub sample_container_bytes: u16,
    /// Bytes per period, for position updates.
    pub host_period_bytes: u32,
    /// 1: the host reads positions itself (DPIB), the DSP sends none.
    pub no_stream_position: u16,
    /// 1: the DSP updates the position continuously.
    pub cont_update_posn: u8,
    /// Channel map (`SOF_CHMAP_*`); Linux sends zeros.
    pub chmap: [u16; MAX_CHANNELS],
}

/// Write a `STREAM_PCM_PARAMS` message (108 bytes).
pub fn encode_pcm_params(buf: &mut [u8], p: &PcmParams) -> Result<usize, Error> {
    if p.channels as usize > MAX_CHANNELS || p.channels == 0 {
        return Err(Error::Invalid("pcm channels"));
    }
    start(buf, PCM_PARAMS_SIZE, GLB_STREAM_MSG | STREAM_PCM_PARAMS)?;
    put_u32(buf, 8, p.comp_id);
    put_u32(buf, 12, p.flags);
    let o = 24; // sof_ipc_stream_params
    put_u32(buf, o, STREAM_PARAMS_SIZE as u32);
    // host buffer at o + 4; Linux leaves its own header size at zero.
    put_u32(buf, o + 8, p.buffer.phy_addr);
    put_u32(buf, o + 12, p.buffer.pages);
    put_u32(buf, o + 16, p.buffer.size);
    put_u32(buf, o + 32, p.direction);
    put_u32(buf, o + 36, p.frame_fmt);
    put_u32(buf, o + 40, p.buffer_fmt);
    put_u32(buf, o + 44, p.rate);
    put_u16(buf, o + 48, p.stream_tag);
    put_u16(buf, o + 50, p.channels);
    put_u16(buf, o + 52, p.sample_valid_bytes);
    put_u16(buf, o + 54, p.sample_container_bytes);
    put_u32(buf, o + 56, p.host_period_bytes);
    put_u16(buf, o + 60, p.no_stream_position);
    put_u8(buf, o + 62, p.cont_update_posn);
    // ext_data_length (o + 64) stays 0: no extended data.
    for (i, c) in p.chmap.iter().enumerate() {
        put_u16(buf, o + 68 + 2 * i, *c);
    }
    Ok(PCM_PARAMS_SIZE)
}

/// Write a 12-byte `sof_ipc_stream` message: `stream_cmd` is one of
/// [`STREAM_TRIG_START`], [`STREAM_TRIG_STOP`], [`STREAM_TRIG_PAUSE`],
/// [`STREAM_TRIG_RELEASE`] or [`STREAM_PCM_FREE`].
pub fn encode_stream(buf: &mut [u8], stream_cmd: u32, comp_id: u32) -> Result<usize, Error> {
    start(buf, STREAM_SIZE, GLB_STREAM_MSG | stream_cmd)?;
    put_u32(buf, 8, comp_id);
    Ok(STREAM_SIZE)
}

/// Write a `STREAM_TRIG_START` for the host component.
pub fn encode_trigger_start(buf: &mut [u8], comp_id: u32) -> Result<usize, Error> {
    encode_stream(buf, STREAM_TRIG_START, comp_id)
}
/// Write a `STREAM_TRIG_STOP` for the host component.
pub fn encode_trigger_stop(buf: &mut [u8], comp_id: u32) -> Result<usize, Error> {
    encode_stream(buf, STREAM_TRIG_STOP, comp_id)
}
/// Write a `STREAM_PCM_FREE` for the host component.
pub fn encode_pcm_free(buf: &mut [u8], comp_id: u32) -> Result<usize, Error> {
    encode_stream(buf, STREAM_PCM_FREE, comp_id)
}

// ---------------------------------------------------------------------------
// Power management messages
// ---------------------------------------------------------------------------

/// Write a `PM_CORE_ENABLE` message (12 bytes): bit N of `enable_mask` keeps
/// core N powered.
pub fn encode_pm_core_enable(buf: &mut [u8], enable_mask: u32) -> Result<usize, Error> {
    start(buf, PM_CORE_CONFIG_SIZE, GLB_PM_MSG | PM_CORE_ENABLE)?;
    put_u32(buf, 8, enable_mask);
    Ok(PM_CORE_CONFIG_SIZE)
}

/// Write a `PM_GATE` message (32 bytes) with platform specific gating `flags`.
pub fn encode_pm_gate(buf: &mut [u8], flags: u32) -> Result<usize, Error> {
    start(buf, PM_GATE_SIZE, GLB_PM_MSG | PM_GATE)?;
    put_u32(buf, 8, flags);
    Ok(PM_GATE_SIZE)
}

/// Write a `PM_CTX_SAVE` message (76 bytes, no context elements), sent before
/// powering the DSP down.
pub fn encode_pm_ctx_save(buf: &mut [u8]) -> Result<usize, Error> {
    start(buf, PM_CTX_SIZE_BYTES, GLB_PM_MSG | PM_CTX_SAVE)?;
    Ok(PM_CTX_SIZE_BYTES)
}

// ---------------------------------------------------------------------------
// Decoders
// ---------------------------------------------------------------------------

/// The first eight bytes of every message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmdHeader {
    /// Whole message length in bytes.
    pub size: u32,
    /// Global type ORed with command.
    pub cmd: u32,
}

impl CmdHeader {
    /// Read the header from the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        need(bytes, CMD_HDR_SIZE)?;
        Ok(Self { size: get_u32(bytes, 0), cmd: get_u32(bytes, 4) })
    }
    /// The global type ([`GLB_REPLY`], [`FW_READY`], ...).
    pub fn glb_type(&self) -> u32 {
        glb_type(self.cmd)
    }
    /// The command inside the global type.
    pub fn cmd_type(&self) -> u32 {
        cmd_type(self.cmd)
    }
}

/// Read a header and check that `size` covers `min` bytes and lies inside `bytes`.
fn header_sized(bytes: &[u8], min: usize) -> Result<CmdHeader, Error> {
    need(bytes, min)?;
    let h = CmdHeader::parse(bytes)?;
    if (h.size as usize) < min || h.size as usize > bytes.len() {
        return Err(Error::BadSize(h.size));
    }
    Ok(h)
}

/// The generic reply every host message gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Header of the reply. The firmware echoes the request's command with
    /// [`GLB_REPLY`] or the request's global type.
    pub hdr: CmdHeader,
    /// 0 on success, a negative errno otherwise.
    pub error: i32,
}

impl Reply {
    /// Read a reply from the DSP's mailbox bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let hdr = header_sized(bytes, REPLY_SIZE)?;
        Ok(Self { hdr, error: get_u32(bytes, 8) as i32 })
    }
    /// `Ok` when the firmware reported success, [`Error::Firmware`] otherwise.
    pub fn result(&self) -> Result<(), Error> {
        if self.error < 0 {
            Err(Error::Firmware(self.error))
        } else {
            Ok(())
        }
    }
}

/// The reply to [`STREAM_PCM_PARAMS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmParamsReply {
    /// The generic part.
    pub reply: Reply,
    /// The host component the parameters were applied to.
    pub comp_id: u32,
    /// Offset of this stream's [`StreamPosn`] inside the stream window.
    pub posn_offset: u32,
}

impl PcmParamsReply {
    /// Read the reply. A firmware error is returned as [`Error::Firmware`]
    /// (the firmware then sends only the 12-byte generic reply).
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let reply = Reply::parse(bytes)?;
        reply.result()?;
        header_sized(bytes, PCM_PARAMS_REPLY_SIZE)?;
        Ok(Self { reply, comp_id: get_u32(bytes, 12), posn_offset: get_u32(bytes, 16) })
    }
}

/// `sof_ipc_stream_posn`: where a stream is, read from the stream window (or
/// sent as a [`STREAM_POSITION`] notification).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamPosn {
    /// Header and error of the record.
    pub reply: Reply,
    /// Host component id.
    pub comp_id: u32,
    /// `SOF_TIME_*` validity bits.
    pub flags: u32,
    /// Wall clock frequency in Hz.
    pub wallclock_hz: u32,
    /// Timestamp resolution in nanoseconds.
    pub timestamp_ns: u32,
    /// Host DMA position in bytes.
    pub host_posn: u64,
    /// DAI DMA position in bytes.
    pub dai_posn: u64,
    /// Component position in bytes.
    pub comp_posn: u64,
    /// Audio wall clock.
    pub wallclock: u64,
    /// System timestamp.
    pub timestamp: u64,
    /// Component that over- or underran.
    pub xrun_comp_id: u32,
    /// Size of the xrun in bytes.
    pub xrun_size: i32,
}

impl StreamPosn {
    /// Read a position record. The size field is not checked against the bytes
    /// because the stream window holds bare records whose header the firmware
    /// may leave unset; only the length of `bytes` is checked.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        need(bytes, STREAM_POSN_SIZE)?;
        let hdr = CmdHeader::parse(bytes)?;
        Ok(Self {
            reply: Reply { hdr, error: get_u32(bytes, 8) as i32 },
            comp_id: get_u32(bytes, 12),
            flags: get_u32(bytes, 16),
            wallclock_hz: get_u32(bytes, 20),
            timestamp_ns: get_u32(bytes, 24),
            host_posn: get_u64(bytes, 28),
            dai_posn: get_u64(bytes, 36),
            comp_posn: get_u64(bytes, 44),
            wallclock: get_u64(bytes, 52),
            timestamp: get_u64(bytes, 60),
            xrun_comp_id: get_u32(bytes, 68),
            xrun_size: get_u32(bytes, 72) as i32,
        })
    }
}

/// `sof_ipc_fw_version`: what firmware is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FwVersion {
    /// Firmware release major number.
    pub major: u16,
    /// Firmware release minor number.
    pub minor: u16,
    /// Firmware release micro number.
    pub micro: u16,
    /// Build number.
    pub build: u16,
    /// Build date, NUL padded ASCII (e.g. `Sep 30 2026`).
    pub date: [u8; 12],
    /// Build time, NUL padded ASCII (e.g. `12:00:00`).
    pub time: [u8; 10],
    /// Source tag, NUL padded ASCII.
    pub tag: [u8; 6],
    /// Packed ABI version the firmware speaks (see [`abi_version`]).
    pub abi_version: u32,
    /// Hash of the firmware source tree.
    pub src_hash: u32,
}

fn ascii(field: &[u8]) -> &str {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let s = &field[..end];
    if s.iter().all(|b| b.is_ascii() && !b.is_ascii_control()) {
        // All bytes are printable ASCII, hence valid UTF-8.
        core::str::from_utf8(s).unwrap_or("")
    } else {
        ""
    }
}

impl FwVersion {
    /// Build date as text, or "" if it is not printable ASCII.
    pub fn date_str(&self) -> &str {
        ascii(&self.date)
    }
    /// Build time as text, or "" if it is not printable ASCII.
    pub fn time_str(&self) -> &str {
        ascii(&self.time)
    }
    /// Source tag as text, or "" if it is not printable ASCII.
    pub fn tag_str(&self) -> &str {
        ascii(&self.tag)
    }
}

/// `sof_ipc_fw_ready`: the firmware's first message after boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FwReady {
    /// Offset of the DSP-initiated mailbox (older firmware; current firmware
    /// describes mailboxes in the window list instead).
    pub dspbox_offset: u32,
    /// Offset of the host-initiated mailbox.
    pub hostbox_offset: u32,
    /// Size of the DSP-initiated mailbox.
    pub dspbox_size: u32,
    /// Size of the host-initiated mailbox.
    pub hostbox_size: u32,
    /// Firmware version and ABI.
    pub version: FwVersion,
    /// `SOF_IPC_INFO_*` bits (build, locks, gdb, D3 persistent).
    pub flags: u64,
}

impl FwReady {
    /// Read FW_READY from the start of the DSP's mailbox. The extended data that
    /// follows at [`FW_READY_SIZE`] is read with [`ExtData`].
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        need(bytes, FW_READY_SIZE)?;
        let h = CmdHeader::parse(bytes)?;
        if h.glb_type() != FW_READY {
            return Err(Error::UnexpectedCmd(h.cmd));
        }
        if (h.size as usize) < FW_READY_SIZE {
            return Err(Error::BadSize(h.size));
        }
        let v = 24;
        let mut date = [0u8; 12];
        let mut time = [0u8; 10];
        let mut tag = [0u8; 6];
        date.copy_from_slice(&bytes[v + 12..v + 24]);
        time.copy_from_slice(&bytes[v + 24..v + 34]);
        tag.copy_from_slice(&bytes[v + 34..v + 40]);
        Ok(Self {
            dspbox_offset: get_u32(bytes, 8),
            hostbox_offset: get_u32(bytes, 12),
            dspbox_size: get_u32(bytes, 16),
            hostbox_size: get_u32(bytes, 20),
            version: FwVersion {
                major: get_u16(bytes, v + 4),
                minor: get_u16(bytes, v + 6),
                micro: get_u16(bytes, v + 8),
                build: get_u16(bytes, v + 10),
                date,
                time,
                tag,
                abi_version: get_u32(bytes, v + 40),
                src_hash: get_u32(bytes, v + 44),
            },
            flags: get_u64(bytes, 84),
        })
    }
}

/// One record of FW_READY's extended data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtItem<'a> {
    /// One of the `EXT_*` types.
    pub ext_type: u32,
    /// The whole record, header included (`hdr.size` bytes).
    pub bytes: &'a [u8],
}

/// Walks the extended data records that follow FW_READY in the mailbox. Each
/// record starts with a command header whose command is [`FW_READY`]; the walk
/// ends at the first header that is not, as in Linux.
pub struct ExtData<'a> {
    rest: &'a [u8],
    failed: bool,
}

impl<'a> ExtData<'a> {
    /// Start a walk over `bytes`, the mailbox from offset [`FW_READY_SIZE`] on.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes, failed: false }
    }
}

impl<'a> Iterator for ExtData<'a> {
    type Item = Result<ExtItem<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.rest.len() < EXT_DATA_HDR_SIZE {
            return None;
        }
        let h = CmdHeader::parse(self.rest).ok()?;
        if h.cmd != FW_READY {
            return None;
        }
        let size = h.size as usize;
        if size < EXT_DATA_HDR_SIZE || size > self.rest.len() {
            self.failed = true;
            return Some(Err(Error::BadSize(h.size)));
        }
        let (rec, rest) = self.rest.split_at(size);
        self.rest = rest;
        Some(Ok(ExtItem { ext_type: get_u32(rec, 8), bytes: rec }))
    }
}

/// `sof_ipc_window_elem`: one memory window region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WindowElem {
    /// One of the `REGION_*` types.
    pub region: u32,
    /// Platform window number; the host maps it to a BAR offset.
    pub id: u32,
    /// Access flags.
    pub flags: u32,
    /// Size in bytes.
    pub size: u32,
    /// Offset inside the window.
    pub offset: u32,
}

/// The window list from FW_READY's extended data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Windows {
    /// How many of `elems` are valid.
    pub count: usize,
    /// The regions.
    pub elems: [WindowElem; MAX_WINDOWS],
}

/// Where the mailboxes are, as offsets in the DSP BAR (after mapping window ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MailboxLayout {
    /// DSP-to-host mailbox (replies and notifications): offset, size.
    pub dsp_box: (u32, u32),
    /// Host-to-DSP mailbox (requests): offset, size.
    pub host_box: (u32, u32),
    /// Stream position records: offset, size (0, 0 when absent).
    pub stream_box: (u32, u32),
    /// Debug region: offset, size (0, 0 when absent).
    pub debug_box: (u32, u32),
}

impl Windows {
    /// Read a window record ([`ExtItem::bytes`] of type [`EXT_WINDOW`]).
    pub fn parse(rec: &[u8]) -> Result<Self, Error> {
        need(rec, EXT_DATA_HDR_SIZE + 4)?;
        if get_u32(rec, 8) != EXT_WINDOW {
            return Err(Error::Invalid("not a window record"));
        }
        let count = get_u32(rec, 12) as usize;
        if count > MAX_WINDOWS {
            return Err(Error::Invalid("too many windows"));
        }
        need(rec, 16 + count * WINDOW_ELEM_SIZE)?;
        let mut elems = [WindowElem::default(); MAX_WINDOWS];
        for (i, e) in elems.iter_mut().enumerate().take(count) {
            let o = 16 + i * WINDOW_ELEM_SIZE;
            *e = WindowElem {
                region: get_u32(rec, o + 4),
                id: get_u32(rec, o + 8),
                flags: get_u32(rec, o + 12),
                size: get_u32(rec, o + 16),
                offset: get_u32(rec, o + 20),
            };
        }
        Ok(Self { count, elems })
    }

    /// Find the window list in the extended data after FW_READY.
    pub fn from_ext_data(ext: &[u8]) -> Result<Self, Error> {
        for item in ExtData::new(ext) {
            let item = item?;
            if item.ext_type == EXT_WINDOW {
                return Self::parse(item.bytes);
            }
        }
        Err(Error::Invalid("no window record"))
    }

    /// The valid regions.
    pub fn as_slice(&self) -> &[WindowElem] {
        &self.elems[..self.count]
    }

    /// The first region of type `region`, if any.
    pub fn find(&self, region: u32) -> Option<WindowElem> {
        self.as_slice().iter().copied().find(|e| e.region == region)
    }

    /// Mailbox offsets in the DSP BAR. `window_base` maps a window id to the
    /// BAR offset of that window (a platform fact the boot code knows); regions
    /// whose window has no base are skipped, as in Linux. Both mailboxes must
    /// be present.
    pub fn layout(&self, window_base: impl Fn(u32) -> Option<u32>) -> Result<MailboxLayout, Error> {
        let mut l = MailboxLayout::default();
        for e in self.as_slice() {
            let Some(base) = window_base(e.id) else { continue };
            let at = (base.wrapping_add(e.offset), e.size);
            match e.region {
                REGION_UPBOX => l.dsp_box = at,
                REGION_DOWNBOX => l.host_box = at,
                REGION_STREAM => l.stream_box = at,
                REGION_DEBUG => l.debug_box = at,
                _ => {}
            }
        }
        if l.dsp_box.1 == 0 || l.host_box.1 == 0 {
            return Err(Error::Invalid("mailbox window missing"));
        }
        Ok(l)
    }
}
