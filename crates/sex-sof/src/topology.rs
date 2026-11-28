// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The fixed digital-microphone capture graph, sent as IPC3 messages.
//!
//! A SOF topology file is a recipe of IPC3 messages. Instead of parsing one,
//! SexOS sends the graph that the DMIC part of `sof-hda-generic-2ch.tplg`
//! builds, trimmed to the parts that move samples:
//!
//! ```text
//!   DMIC DAI 0 (FIFO A, 48 kHz) --> buffer --> host component --> host DMA --> Bell's ring
//! ```
//!
//! The tplg's pipeline 10 also has a high-pass IIR filter and a volume stage
//! between the DAI and the host; both need control data (filter coefficients,
//! gain tables) before they pass audio, so the first version leaves them out.
//!
//! [`DmicCapture`] holds every number the messages need, with defaults taken
//! from `sof-tplg/sof-hda-generic-2ch.tplg` in sof-bin v2026.09.2 (decoded from
//! the binary, see the stage ledger). [`DmicCapture::start_steps`] and
//! [`DmicCapture::stop_steps`] list the messages in the order to send them, and
//! [`DmicCapture::encode`] writes one step into a caller buffer. Nothing
//! allocates; the caller loops over the steps, sends each message, and checks
//! each reply before the next.

use crate::ipc3::{self, Error};

/// Size of one page in the host ring's page table.
pub const PAGE_SIZE: u64 = 4096;

/// How many page frames a page table of `table_len` bytes can hold (each takes
/// 20 bits).
pub const fn page_table_capacity(table_len: usize) -> usize {
    table_len * 2 / 5
}

/// Build the compressed page table that SOF's host component reads to find the
/// ring, the way Linux `snd_sof_create_page_table` does: each 4 KiB page's
/// frame number (bus address >> 12) is packed into 20 bits, little-endian, two
/// frames per five bytes. `pages` are the bus addresses of the ring's pages in
/// order. The table is zeroed first; the number of pages is returned.
///
/// Fails if a page is not 4 KiB aligned, lies at or above 4 GiB (20-bit frame
/// numbers), or the table is too small.
pub fn encode_page_table(pages: &[u64], table: &mut [u8]) -> Result<u32, Error> {
    if pages.is_empty() {
        return Err(Error::Invalid("page table: no pages"));
    }
    if pages.len() > page_table_capacity(table.len()) {
        return Err(Error::BufferTooSmall { need: (pages.len() * 5).div_ceil(2), have: table.len() });
    }
    table.fill(0);
    for (i, &addr) in pages.iter().enumerate() {
        if addr % PAGE_SIZE != 0 {
            return Err(Error::Invalid("page table: page not 4 KiB aligned"));
        }
        let pfn = addr / PAGE_SIZE;
        if pfn >= 1 << 20 {
            return Err(Error::Invalid("page table: page above 4 GiB"));
        }
        let pfn = pfn as u32;
        let idx = (5 * i) >> 1;
        if i & 1 == 0 {
            // Bits 0..19 starting at a byte boundary.
            table[idx] = pfn as u8;
            table[idx + 1] = (pfn >> 8) as u8;
            table[idx + 2] = (pfn >> 16) as u8 & 0x0f;
        } else {
            // Bits 4..23 of the word at idx: keep the previous frame's nibble.
            table[idx] = (table[idx] & 0x0f) | ((pfn << 4) as u8);
            table[idx + 1] = (pfn >> 4) as u8;
            table[idx + 2] = (pfn >> 12) as u8;
        }
    }
    Ok(pages.len() as u32)
}

/// The host side ring that capture samples land in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HostRing {
    /// Bus address of the page holding the compressed page table.
    pub page_table_addr: u64,
    /// Number of 4 KiB pages in the ring (what [`encode_page_table`] returned).
    pub pages: u32,
    /// Ring size in bytes.
    pub bytes: u32,
    /// Bytes per period (one period is what Bell consumes per wake).
    pub period_bytes: u32,
}

/// Component UUID of SOF's host component, as stored in the tplg
/// (`8b9d100c-6d78-418f-90a3-e0e805d0852b`, raw byte order).
pub const UUID_HOST: [u8; 16] = [
    0x0c, 0x10, 0x9d, 0x8b, 0x78, 0x6d, 0x8f, 0x41, 0x90, 0xa3, 0xe0, 0xe8, 0x05, 0xd0, 0x85, 0x2b,
];
/// Component UUID of SOF's DAI component, as stored in the tplg
/// (`c2b00d27-ffbc-4150-a51a-245c79c5e54b`, raw byte order).
pub const UUID_DAI: [u8; 16] = [
    0x27, 0x0d, 0xb0, 0xc2, 0xbc, 0xff, 0x50, 0x41, 0xa5, 0x1a, 0x24, 0x5c, 0x79, 0xc5, 0xe5, 0x4b,
];

/// The PCM number of the DMIC capture device in the tplg (Linux `hw:0,6`). It is
/// not part of any IPC3 message; kept for logs.
pub const TPLG_DMIC_PCM_ID: u32 = 6;

const PDM_OFF: ipc3::DmicPdm =
    ipc3::DmicPdm { id: 0, enable_mic_a: 0, enable_mic_b: 0, polarity_mic_a: 0, polarity_mic_b: 0, clk_edge: 0, skew: 0 };

/// From this ABI on, Linux creates the pipeline before its components; older
/// firmware wants it after the components and connections.
pub const ABI_PIPELINE_FIRST: u32 = ipc3::abi_version(3, 19, 0);

/// Everything needed to build, start, stop and free the DMIC capture graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmicCapture {
    /// Pipeline id (tplg: 10).
    pub pipeline_id: u32,
    /// Component id of the pipeline (its scheduler widget).
    pub pipeline_comp_id: u32,
    /// Component id of the host component; triggers name the stream by it.
    pub host_comp_id: u32,
    /// Component id of the buffer between DAI and host.
    pub buffer_comp_id: u32,
    /// Component id of the DMIC DAI component.
    pub dai_comp_id: u32,
    /// DSP core everything runs on (tplg: 0).
    pub core: u32,
    /// Pipeline period in microseconds (tplg: 1000).
    pub period_us: u32,
    /// Pipeline priority (tplg: 0).
    pub priority: u32,
    /// Pipeline worst case load (tplg: 5000).
    pub period_mips: u32,
    /// Frames per run, 0 for variable (tplg: 0).
    pub frames_per_sched: u32,
    /// Xrun report limit in microseconds (tplg: unset, 0).
    pub xrun_limit_us: u32,
    /// Scheduling time domain (tplg: timer).
    pub time_domain: u32,
    /// Buffer size in bytes (tplg: 768 = two 1 ms periods of 2 x 32-bit at 48 kHz).
    pub buffer_size: u32,
    /// Buffer memory capabilities (tplg: 113 = RAM | HP | DMA | CACHE).
    pub buffer_caps: u32,
    /// Host component config (tplg: sink periods 0, source periods 2, no format).
    pub host_config: ipc3::CompConfig,
    /// DAI component config (tplg: sink periods 2, source periods 0, S32_LE).
    pub dai_config: ipc3::CompConfig,
    /// DMIC DAI index (tplg: 0, the 48 kHz FIFO A).
    pub dai_index: u32,
    /// DMIC controller settings (tplg link `dmic01`).
    pub dmic: ipc3::DmicParams,
    /// Send the component UUIDs as extended data, as Linux does.
    pub send_uuids: bool,
    /// Send PIPE_NEW first (firmware ABI 3.19 and later) or after the
    /// connections (older firmware); see [`DmicCapture::for_firmware_abi`].
    pub pipeline_first: bool,
    /// Capture rate in Hz (48000).
    pub rate: u32,
    /// Channels (2: mic A and mic B of PDM 0).
    pub channels: u16,
    /// Sample format in the host ring (S32_LE).
    pub frame_fmt: u32,
    /// Bytes per sample in the ring (4).
    pub sample_bytes: u16,
    /// HD Audio stream tag of the decoupled capture DMA stream Bell set up.
    pub stream_tag: u16,
    /// The host ring.
    pub ring: HostRing,
    /// The host reads positions from DPIB itself (Linux's HDA default), so the
    /// firmware sends no position messages.
    pub no_stream_position: bool,
}

impl DmicCapture {
    /// The tplg's DMIC 48 kHz capture, with SexOS's own component ids, for
    /// firmware at ABI 3.19 or later. The stream tag and the ring must be set
    /// before PCM_PARAMS can be encoded.
    pub const TGL_DEFAULT: Self = Self {
        pipeline_id: 10,
        pipeline_comp_id: 4,
        host_comp_id: 1,
        buffer_comp_id: 2,
        dai_comp_id: 3,
        core: 0,
        period_us: 1000,
        priority: 0,
        period_mips: 5000,
        frames_per_sched: 0,
        xrun_limit_us: 0,
        time_domain: ipc3::TIME_DOMAIN_TIMER,
        buffer_size: 768,
        buffer_caps: ipc3::MEM_CAPS_RAM | ipc3::MEM_CAPS_HP | ipc3::MEM_CAPS_DMA | ipc3::MEM_CAPS_CACHE,
        host_config: ipc3::CompConfig { periods_sink: 0, periods_source: 2, frame_fmt: 0, xrun_action: 0 },
        dai_config: ipc3::CompConfig {
            periods_sink: 2,
            periods_source: 0,
            frame_fmt: ipc3::FRAME_S32_LE,
            xrun_action: 0,
        },
        dai_index: 0,
        dmic: ipc3::DmicParams {
            driver_ipc_version: 1,
            pdmclk_min: 2_400_000,
            pdmclk_max: 4_800_000,
            fifo_fs: 48_000,
            fifo_bits: 32,
            fifo_bits_b: 0,
            duty_min: 40,
            duty_max: 60,
            num_pdm_active: 1,
            wake_up_time_us: 0,
            min_clock_on_time_us: 0,
            unmute_ramp_time_ms: 200,
            pdm: [
                ipc3::DmicPdm {
                    id: 0,
                    enable_mic_a: 1,
                    enable_mic_b: 1,
                    polarity_mic_a: 0,
                    polarity_mic_b: 0,
                    clk_edge: 0,
                    skew: 0,
                },
                PDM_OFF,
                PDM_OFF,
                PDM_OFF,
            ],
        },
        send_uuids: true,
        pipeline_first: true,
        rate: 48_000,
        channels: 2,
        frame_fmt: ipc3::FRAME_S32_LE,
        sample_bytes: 4,
        stream_tag: 0,
        ring: HostRing { page_table_addr: 0, pages: 0, bytes: 0, period_bytes: 0 },
        no_stream_position: true,
    };

    /// Adjust the message order to the ABI the firmware reported in FW_READY.
    pub const fn for_firmware_abi(mut self, fw_abi: u32) -> Self {
        self.pipeline_first = fw_abi >= ABI_PIPELINE_FIRST;
        self
    }

    /// The messages that build, configure and start the graph, in order.
    pub fn start_steps(&self) -> &'static [Step] {
        if self.pipeline_first {
            &START_PIPELINE_FIRST
        } else {
            &START_PIPELINE_LAST
        }
    }

    /// The messages that stop the stream and free the graph, in order.
    pub fn stop_steps(&self) -> &'static [Step] {
        &STOP
    }

    /// Write start message `n` into `buf`; `None` once all are written.
    pub fn encode_start(&self, n: usize, buf: &mut [u8]) -> Option<Result<usize, Error>> {
        self.start_steps().get(n).map(|s| self.encode(*s, buf))
    }

    /// Write stop message `n` into `buf`; `None` once all are written.
    pub fn encode_stop(&self, n: usize, buf: &mut [u8]) -> Option<Result<usize, Error>> {
        self.stop_steps().get(n).map(|s| self.encode(*s, buf))
    }

    fn dai_config_msg(&self, flags: u8) -> ipc3::DaiConfigDmic {
        ipc3::DaiConfigDmic { dai_index: self.dai_index, format: 0, group_id: 0, flags, dmic: self.dmic }
    }

    /// The PCM parameters this graph sends, or an error naming what is unset.
    pub fn pcm_params(&self) -> Result<ipc3::PcmParams, Error> {
        if self.stream_tag == 0 || self.stream_tag > 15 {
            return Err(Error::Invalid("pcm_params: stream tag unset"));
        }
        let r = &self.ring;
        if r.pages == 0 || r.bytes == 0 || r.bytes as u64 > r.pages as u64 * PAGE_SIZE {
            return Err(Error::Invalid("pcm_params: ring unset"));
        }
        let Ok(pt) = u32::try_from(r.page_table_addr) else {
            return Err(Error::Invalid("pcm_params: page table above 4 GiB"));
        };
        Ok(ipc3::PcmParams {
            comp_id: self.host_comp_id,
            flags: 0,
            buffer: ipc3::HostBuffer { phy_addr: pt, pages: r.pages, size: r.bytes },
            direction: ipc3::STREAM_CAPTURE,
            frame_fmt: self.frame_fmt,
            buffer_fmt: ipc3::BUFFER_INTERLEAVED,
            rate: self.rate,
            stream_tag: self.stream_tag,
            channels: self.channels,
            sample_valid_bytes: self.sample_bytes,
            sample_container_bytes: self.sample_bytes,
            host_period_bytes: r.period_bytes,
            no_stream_position: self.no_stream_position as u16,
            cont_update_posn: 0,
            chmap: [0; ipc3::MAX_CHANNELS],
        })
    }

    /// Write the message for `step` into `buf` and return its length.
    pub fn encode(&self, step: Step, buf: &mut [u8]) -> Result<usize, Error> {
        match step {
            Step::PipeNew => ipc3::encode_pipe_new(
                buf,
                &ipc3::PipeNew {
                    comp_id: self.pipeline_comp_id,
                    pipeline_id: self.pipeline_id,
                    // A capture pipeline is driven by its DAI end.
                    sched_id: self.dai_comp_id,
                    core: self.core,
                    period_us: self.period_us,
                    priority: self.priority,
                    period_mips: self.period_mips,
                    frames_per_sched: self.frames_per_sched,
                    xrun_limit_us: self.xrun_limit_us,
                    time_domain: self.time_domain,
                },
            ),
            Step::DaiComp => ipc3::encode_dai_comp(
                buf,
                &ipc3::DaiComp {
                    id: self.dai_comp_id,
                    pipeline_id: self.pipeline_id,
                    core: self.core,
                    config: self.dai_config,
                    direction: ipc3::STREAM_CAPTURE,
                    dai_index: self.dai_index,
                    dai_type: ipc3::DAI_INTEL_DMIC,
                    uuid: if self.send_uuids { Some(UUID_DAI) } else { None },
                },
            ),
            Step::DaiConfig => ipc3::encode_dai_config_dmic(buf, &self.dai_config_msg(ipc3::DAI_CONFIG_FLAGS_HW_PARAMS)),
            Step::BufferNew => ipc3::encode_buffer_new(
                buf,
                &ipc3::BufferNew {
                    id: self.buffer_comp_id,
                    pipeline_id: self.pipeline_id,
                    core: self.core,
                    size: self.buffer_size,
                    caps: self.buffer_caps,
                    flags: 0,
                },
            ),
            Step::HostComp => ipc3::encode_host_comp(
                buf,
                &ipc3::HostComp {
                    id: self.host_comp_id,
                    pipeline_id: self.pipeline_id,
                    core: self.core,
                    config: self.host_config,
                    direction: ipc3::STREAM_CAPTURE,
                    no_irq: 0,
                    dmac_config: 0,
                    uuid: if self.send_uuids { Some(UUID_HOST) } else { None },
                },
            ),
            Step::ConnectDaiToBuffer => ipc3::encode_comp_connect(buf, self.dai_comp_id, self.buffer_comp_id),
            Step::ConnectBufferToHost => ipc3::encode_comp_connect(buf, self.buffer_comp_id, self.host_comp_id),
            Step::PipeComplete => ipc3::encode_pipe_complete(buf, self.pipeline_comp_id),
            Step::PcmParams => ipc3::encode_pcm_params(buf, &self.pcm_params()?),
            Step::TrigStart => ipc3::encode_trigger_start(buf, self.host_comp_id),
            Step::TrigStop => ipc3::encode_trigger_stop(buf, self.host_comp_id),
            Step::PcmFree => ipc3::encode_pcm_free(buf, self.host_comp_id),
            Step::DaiConfigFree => ipc3::encode_dai_config_dmic(buf, &self.dai_config_msg(ipc3::DAI_CONFIG_FLAGS_HW_FREE)),
            Step::DaiFree => ipc3::encode_comp_free(buf, self.dai_comp_id),
            Step::BufferFree => ipc3::encode_buffer_free(buf, self.buffer_comp_id),
            Step::HostFree => ipc3::encode_comp_free(buf, self.host_comp_id),
            Step::PipeFree => ipc3::encode_pipe_free(buf, self.pipeline_comp_id),
        }
    }
}

impl Default for DmicCapture {
    fn default() -> Self {
        Self::TGL_DEFAULT
    }
}

/// One message of the graph's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `TPLG_PIPE_NEW`.
    PipeNew,
    /// `TPLG_COMP_NEW` for the DMIC DAI.
    DaiComp,
    /// `DAI_CONFIG` for DMIC 0 with the hw-params flag.
    DaiConfig,
    /// `TPLG_BUFFER_NEW`.
    BufferNew,
    /// `TPLG_COMP_NEW` for the host component.
    HostComp,
    /// `TPLG_COMP_CONNECT` DAI to buffer.
    ConnectDaiToBuffer,
    /// `TPLG_COMP_CONNECT` buffer to host.
    ConnectBufferToHost,
    /// `TPLG_PIPE_COMPLETE`.
    PipeComplete,
    /// `STREAM_PCM_PARAMS`; its reply is a [`ipc3::PcmParamsReply`].
    PcmParams,
    /// `STREAM_TRIG_START`.
    TrigStart,
    /// `STREAM_TRIG_STOP`.
    TrigStop,
    /// `STREAM_PCM_FREE`.
    PcmFree,
    /// `DAI_CONFIG` for DMIC 0 with the hw-free flag.
    DaiConfigFree,
    /// `TPLG_COMP_FREE` for the DAI.
    DaiFree,
    /// `TPLG_BUFFER_FREE`.
    BufferFree,
    /// `TPLG_COMP_FREE` for the host component.
    HostFree,
    /// `TPLG_PIPE_FREE`.
    PipeFree,
}

impl Step {
    /// Short name for log lines (`[bell.dsp] dmic step pcm_params failed ...`).
    pub const fn name(self) -> &'static str {
        match self {
            Step::PipeNew => "pipe_new",
            Step::DaiComp => "dai_comp_new",
            Step::DaiConfig => "dai_config",
            Step::BufferNew => "buffer_new",
            Step::HostComp => "host_comp_new",
            Step::ConnectDaiToBuffer => "connect_dai_buffer",
            Step::ConnectBufferToHost => "connect_buffer_host",
            Step::PipeComplete => "pipe_complete",
            Step::PcmParams => "pcm_params",
            Step::TrigStart => "trigger_start",
            Step::TrigStop => "trigger_stop",
            Step::PcmFree => "pcm_free",
            Step::DaiConfigFree => "dai_config_free",
            Step::DaiFree => "dai_free",
            Step::BufferFree => "buffer_free",
            Step::HostFree => "host_free",
            Step::PipeFree => "pipe_free",
        }
    }

    /// Whether the reply carries more than the generic 12 bytes (PCM_PARAMS
    /// returns the position offset).
    pub const fn has_reply_payload(self) -> bool {
        matches!(self, Step::PcmParams)
    }
}

/// Start order for ABI 3.19+ firmware, as Linux sets up a dynamic capture
/// pipeline: pipeline, then widgets from the DAI end (each DAI followed by its
/// config), then routes, then complete, then stream parameters and start.
const START_PIPELINE_FIRST: [Step; 10] = [
    Step::PipeNew,
    Step::DaiComp,
    Step::DaiConfig,
    Step::BufferNew,
    Step::HostComp,
    Step::ConnectDaiToBuffer,
    Step::ConnectBufferToHost,
    Step::PipeComplete,
    Step::PcmParams,
    Step::TrigStart,
];

/// Start order for firmware older than ABI 3.19: the pipeline is created after
/// the routes, right before it is completed.
const START_PIPELINE_LAST: [Step; 10] = [
    Step::DaiComp,
    Step::DaiConfig,
    Step::BufferNew,
    Step::HostComp,
    Step::ConnectDaiToBuffer,
    Step::ConnectBufferToHost,
    Step::PipeNew,
    Step::PipeComplete,
    Step::PcmParams,
    Step::TrigStart,
];

/// Stop order: stop, release the stream, then free from the DAI end (the DAI
/// gets a hw-free config first) and the pipeline last.
const STOP: [Step; 7] = [
    Step::TrigStop,
    Step::PcmFree,
    Step::DaiConfigFree,
    Step::DaiFree,
    Step::BufferFree,
    Step::HostFree,
    Step::PipeFree,
];
