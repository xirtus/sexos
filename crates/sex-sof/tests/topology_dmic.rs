// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The fixed DMIC capture graph: tplg defaults, message order, the bytes of each
//! step, and the compressed page table.

use sex_sof::ipc3::{self, Error};
use sex_sof::topology::*;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// The default graph with a ring and stream tag, as Bell would fill them.
fn ready() -> DmicCapture {
    DmicCapture {
        stream_tag: 2,
        ring: HostRing { page_table_addr: 0x7f00_0000, pages: 4, bytes: 16384, period_bytes: 384 },
        ..DmicCapture::TGL_DEFAULT
    }
}

#[test]
fn defaults_match_the_tplg() {
    let d = DmicCapture::default();
    assert_eq!(d.pipeline_id, 10);
    assert_eq!((d.period_us, d.priority, d.period_mips, d.frames_per_sched), (1000, 0, 5000, 0));
    assert_eq!(d.time_domain, ipc3::TIME_DOMAIN_TIMER);
    assert_eq!(d.buffer_size, 768);
    assert_eq!(d.buffer_caps, 113);
    assert_eq!(d.dmic.pdmclk_min, 2_400_000);
    assert_eq!(d.dmic.pdmclk_max, 4_800_000);
    assert_eq!(d.dmic.fifo_fs, 48_000);
    assert_eq!(d.dmic.fifo_bits, 32);
    assert_eq!((d.dmic.duty_min, d.dmic.duty_max), (40, 60));
    assert_eq!(d.dmic.num_pdm_active, 1);
    assert_eq!(d.dmic.unmute_ramp_time_ms, 200);
    assert_eq!((d.dmic.pdm[0].enable_mic_a, d.dmic.pdm[0].enable_mic_b), (1, 1));
    assert_eq!((d.rate, d.channels, d.sample_bytes), (48_000, 2, 4));
    // Two 1 ms periods of 2 x 32-bit samples at 48 kHz.
    assert_eq!(d.buffer_size, 2 * 48 * 2 * 4);
    // The ids must all differ: the firmware keeps one id space.
    let ids = [d.pipeline_comp_id, d.host_comp_id, d.buffer_comp_id, d.dai_comp_id];
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn start_order_new_firmware() {
    let d = ready();
    assert_eq!(
        d.start_steps(),
        &[
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
        ]
    );
    assert_eq!(
        d.stop_steps(),
        &[
            Step::TrigStop,
            Step::PcmFree,
            Step::DaiConfigFree,
            Step::DaiFree,
            Step::BufferFree,
            Step::HostFree,
            Step::PipeFree,
        ]
    );
}

#[test]
fn start_order_old_firmware() {
    let d = ready().for_firmware_abi(ipc3::abi_version(3, 18, 0));
    let s = d.start_steps();
    let pos = |x: Step| s.iter().position(|&y| y == x).unwrap();
    assert!(pos(Step::ConnectBufferToHost) < pos(Step::PipeNew));
    assert_eq!(pos(Step::PipeNew) + 1, pos(Step::PipeComplete));
    assert!(ready().for_firmware_abi(ipc3::abi_version(3, 19, 0)).pipeline_first);
}

/// Encode every start message and check its command word and the ids in it.
#[test]
fn full_start_sequence_bytes() {
    let d = ready();
    let mut buf = [0u8; ipc3::MSG_MAX_SIZE];
    let mut cmds = Vec::new();
    let mut n = 0;
    while let Some(r) = d.encode_start(n, &mut buf) {
        let len = r.unwrap_or_else(|e| panic!("step {n} ({}): {e:?}", d.start_steps()[n].name()));
        assert_eq!(u32_at(&buf, 0) as usize, len, "hdr.size of step {n}");
        assert!(len <= ipc3::MSG_MAX_SIZE);
        cmds.push((u32_at(&buf, 4), len, buf));
        n += 1;
    }
    assert_eq!(n, 10);
    let want = [
        (0x3010_0000, 48),  // pipe new
        (0x3001_0000, 96),  // DAI comp + UUID
        (0x8001_0000, 216), // DAI config
        (0x3020_0000, 44),  // buffer
        (0x3001_0000, 92),  // host comp + UUID
        (0x3003_0000, 16),  // connect DAI -> buffer
        (0x3003_0000, 16),  // connect buffer -> host
        (0x3013_0000, 12),  // complete
        (0x6001_0000, 108), // PCM params
        (0x6004_0000, 12),  // start
    ];
    for (i, (c, l)) in want.iter().enumerate() {
        assert_eq!((cmds[i].0, cmds[i].1), (*c, *l), "step {i}");
    }

    let pipe = &cmds[0].2;
    assert_eq!(u32_at(pipe, 8), d.pipeline_comp_id);
    assert_eq!(u32_at(pipe, 12), 10);
    assert_eq!(u32_at(pipe, 16), d.dai_comp_id, "capture is scheduled by its DAI");
    assert_eq!(u32_at(pipe, 24), 1000);
    assert_eq!(u32_at(pipe, 44), ipc3::TIME_DOMAIN_TIMER);

    let dai = &cmds[1].2;
    assert_eq!(u32_at(dai, 8), d.dai_comp_id);
    assert_eq!(u32_at(dai, 12), ipc3::COMP_DAI);
    assert_eq!(u32_at(dai, 16), 10);
    assert_eq!(u32_at(dai, 36), 2); // periods_sink
    assert_eq!(u32_at(dai, 48), ipc3::FRAME_S32_LE);
    assert_eq!(u32_at(dai, 64), ipc3::STREAM_CAPTURE);
    assert_eq!(u32_at(dai, 72), ipc3::DAI_INTEL_DMIC);
    assert_eq!(&dai[80..96], &UUID_DAI);

    let cfg = &cmds[2].2;
    assert_eq!(u32_at(cfg, 8), ipc3::DAI_INTEL_DMIC);
    assert_eq!(cfg[19], ipc3::DAI_CONFIG_FLAGS_HW_PARAMS);
    assert_eq!(u32_at(cfg, 52 + 16), 48_000);
    assert_eq!(u16_at(cfg, 120 + 6), 1);
    assert_eq!(u16_at(cfg, 120 + 8), 1);

    let b = &cmds[3].2;
    assert_eq!(u32_at(b, 8), d.buffer_comp_id);
    assert_eq!(u32_at(b, 28), 768);
    assert_eq!(u32_at(b, 32), 113);

    let host = &cmds[4].2;
    assert_eq!(u32_at(host, 8), d.host_comp_id);
    assert_eq!(u32_at(host, 12), ipc3::COMP_HOST);
    assert_eq!(u32_at(host, 40), 2); // periods_source
    assert_eq!(u32_at(host, 64), ipc3::STREAM_CAPTURE);
    assert_eq!(&host[76..92], &UUID_HOST);

    assert_eq!((u32_at(&cmds[5].2, 8), u32_at(&cmds[5].2, 12)), (d.dai_comp_id, d.buffer_comp_id));
    assert_eq!((u32_at(&cmds[6].2, 8), u32_at(&cmds[6].2, 12)), (d.buffer_comp_id, d.host_comp_id));
    assert_eq!(u32_at(&cmds[7].2, 8), d.pipeline_comp_id);

    let pcm = &cmds[8].2;
    assert_eq!(u32_at(pcm, 8), d.host_comp_id);
    assert_eq!(u32_at(pcm, 24 + 8), 0x7f00_0000);
    assert_eq!(u32_at(pcm, 24 + 12), 4);
    assert_eq!(u32_at(pcm, 24 + 16), 16384);
    assert_eq!(u32_at(pcm, 24 + 32), ipc3::STREAM_CAPTURE);
    assert_eq!(u32_at(pcm, 24 + 36), ipc3::FRAME_S32_LE);
    assert_eq!(u32_at(pcm, 24 + 44), 48_000);
    assert_eq!(u16_at(pcm, 24 + 48), 2); // stream tag
    assert_eq!(u16_at(pcm, 24 + 50), 2); // channels
    assert_eq!(u32_at(pcm, 24 + 56), 384);
    assert_eq!(u16_at(pcm, 24 + 60), 1); // no_stream_position

    assert_eq!(u32_at(&cmds[9].2, 8), d.host_comp_id);
    assert!(Step::PcmParams.has_reply_payload());
    assert!(!Step::TrigStart.has_reply_payload());
}

#[test]
fn full_stop_sequence_bytes() {
    let d = ready();
    let mut buf = [0u8; ipc3::MSG_MAX_SIZE];
    let want = [
        (0x6005_0000, 12, d.host_comp_id),
        (0x6003_0000, 12, d.host_comp_id),
        (0x8001_0000, 216, ipc3::DAI_INTEL_DMIC),
        (0x3002_0000, 12, d.dai_comp_id),
        (0x3021_0000, 12, d.buffer_comp_id),
        (0x3002_0000, 12, d.host_comp_id),
        (0x3011_0000, 12, d.pipeline_comp_id),
    ];
    let mut n = 0;
    while let Some(r) = d.encode_stop(n, &mut buf) {
        let len = r.unwrap();
        assert_eq!((u32_at(&buf, 4), len, u32_at(&buf, 8)), want[n], "stop step {n}");
        if n == 2 {
            assert_eq!(buf[19], ipc3::DAI_CONFIG_FLAGS_HW_FREE);
        }
        n += 1;
    }
    assert_eq!(n, want.len());
}

#[test]
fn custom_ids_and_no_uuids() {
    let d = DmicCapture {
        pipeline_id: 3,
        pipeline_comp_id: 40,
        host_comp_id: 41,
        buffer_comp_id: 42,
        dai_comp_id: 43,
        send_uuids: false,
        ..ready()
    };
    let mut buf = [0u8; ipc3::MSG_MAX_SIZE];
    assert_eq!(d.encode(Step::HostComp, &mut buf).unwrap(), 76);
    assert_eq!((u32_at(&buf, 8), u32_at(&buf, 16), u32_at(&buf, 24)), (41, 3, 0));
    assert_eq!(d.encode(Step::DaiComp, &mut buf).unwrap(), 80);
    assert_eq!(d.encode(Step::PipeNew, &mut buf).unwrap(), 48);
    assert_eq!((u32_at(&buf, 8), u32_at(&buf, 12), u32_at(&buf, 16)), (40, 3, 43));
    d.encode(Step::TrigStart, &mut buf).unwrap();
    assert_eq!(u32_at(&buf, 8), 41);
}

#[test]
fn pcm_params_needs_ring_and_tag() {
    let mut buf = [0u8; ipc3::MSG_MAX_SIZE];
    let d = DmicCapture::TGL_DEFAULT;
    assert_eq!(d.encode(Step::PcmParams, &mut buf), Err(Error::Invalid("pcm_params: stream tag unset")));
    let d = DmicCapture { stream_tag: 1, ..DmicCapture::TGL_DEFAULT };
    assert_eq!(d.encode(Step::PcmParams, &mut buf), Err(Error::Invalid("pcm_params: ring unset")));
    let d = DmicCapture { stream_tag: 16, ..ready() };
    assert!(d.pcm_params().is_err());
    let d = DmicCapture { ring: HostRing { bytes: 5 * 4096, ..ready().ring }, ..ready() };
    assert_eq!(d.pcm_params(), Err(Error::Invalid("pcm_params: ring unset")), "ring larger than its pages");
    let d = DmicCapture { ring: HostRing { page_table_addr: 1 << 32, ..ready().ring }, ..ready() };
    assert!(d.pcm_params().is_err());
    // Steps before PCM_PARAMS do not need the ring.
    assert!(DmicCapture::TGL_DEFAULT.encode(Step::PipeComplete, &mut buf).is_ok());
}

#[test]
fn step_names_are_unique() {
    let d = ready();
    let all: Vec<Step> = d.start_steps().iter().chain(d.stop_steps()).copied().collect();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a.name(), b.name());
        }
    }
}

/// The reference packing of Linux `snd_sof_create_page_table`, written the
/// way the C does it: a little-endian 32-bit store at byte (5 * i) / 2.
fn reference_table(pages: &[u64]) -> Vec<u8> {
    let mut t = vec![0u8; pages.len() * 5 / 2 + 4];
    for (i, &a) in pages.iter().enumerate() {
        let idx = (5 * i) >> 1;
        let pfn = (a >> 12) as u32;
        let old = u32::from_le_bytes(t[idx..idx + 4].try_into().unwrap());
        let v = if i & 1 == 1 { (old & 0xf) | (pfn << 4) } else { pfn };
        t[idx..idx + 4].copy_from_slice(&v.to_le_bytes());
    }
    t.truncate((pages.len() * 5).div_ceil(2));
    t
}

#[test]
fn page_table_matches_linux_packing() {
    let pages: Vec<u64> = [0x12345u64, 0xabcde, 0x00001, 0xfffff, 0x80000].iter().map(|p| p << 12).collect();
    let mut t = [0xAAu8; 64];
    assert_eq!(encode_page_table(&pages, &mut t).unwrap(), 5);
    let r = reference_table(&pages);
    assert_eq!(&t[..r.len()], &r[..]);
    assert!(t[r.len()..].iter().all(|&x| x == 0));
    // Two frames per five bytes: 0x12345 then 0xabcde.
    assert_eq!(&t[..5], &[0x45, 0x23, 0xe1, 0xcd, 0xab]);
}

#[test]
fn page_table_errors() {
    let mut t = [0u8; 5];
    assert_eq!(page_table_capacity(5), 2);
    assert!(encode_page_table(&[0x1000, 0x2000], &mut t).is_ok());
    assert_eq!(
        encode_page_table(&[0x1000, 0x2000, 0x3000], &mut t),
        Err(Error::BufferTooSmall { need: 8, have: 5 })
    );
    assert!(encode_page_table(&[0x1001], &mut t).is_err());
    assert!(encode_page_table(&[1u64 << 32], &mut t).is_err());
    assert!(encode_page_table(&[], &mut t).is_err());
}
