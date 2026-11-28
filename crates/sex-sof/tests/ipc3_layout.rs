// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! IPC3 encoders and decoders checked byte by byte against the packed C layouts
//! of SOF's `include/ipc/*.h` (sizes and offsets measured by compiling Linux's
//! `include/sound/sof/*.h` copies of those headers).

use sex_sof::ipc3::*;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// A buffer full of 0xAA, so a reserved field the encoder forgot to zero shows.
fn dirty() -> [u8; MSG_MAX_SIZE] {
    [0xAA; MSG_MAX_SIZE]
}

#[test]
fn abi_version_packing() {
    assert_eq!(ABI_VERSION, 0x0301_7001);
    assert_eq!(abi_major(ABI_VERSION), 3);
    assert_eq!(abi_minor(ABI_VERSION), 23);
    assert_eq!(abi_patch(ABI_VERSION), 1);
    assert!(abi_compatible(abi_version(3, 29, 0)));
    assert!(!abi_compatible(abi_version(4, 0, 0)));
    assert_eq!(&ABI_MAGIC.to_le_bytes(), b"SOF\0");
}

#[test]
fn command_words() {
    assert_eq!(GLB_REPLY, 0x1000_0000);
    assert_eq!(GLB_COMPOUND, 0x2000_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_COMP_NEW, 0x3001_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_COMP_FREE, 0x3002_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_COMP_CONNECT, 0x3003_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_PIPE_NEW, 0x3010_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_PIPE_FREE, 0x3011_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_PIPE_COMPLETE, 0x3013_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_BUFFER_NEW, 0x3020_0000);
    assert_eq!(GLB_TPLG_MSG | TPLG_BUFFER_FREE, 0x3021_0000);
    assert_eq!(GLB_PM_MSG | PM_CTX_SAVE, 0x4001_0000);
    assert_eq!(GLB_PM_MSG | PM_CORE_ENABLE, 0x4007_0000);
    assert_eq!(GLB_COMP_MSG | COMP_SET_DATA, 0x5003_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_PCM_PARAMS, 0x6001_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_PCM_PARAMS_REPLY, 0x6002_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_PCM_FREE, 0x6003_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_TRIG_START, 0x6004_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_TRIG_STOP, 0x6005_0000);
    assert_eq!(GLB_STREAM_MSG | STREAM_POSITION, 0x600a_0000);
    assert_eq!(FW_READY, 0x7000_0000);
    assert_eq!(GLB_DAI_MSG | DAI_CONFIG, 0x8001_0000);
    assert_eq!(GLB_TRACE_MSG | TRACE_DMA_PARAMS_EXT, 0x9003_0000);
    assert_eq!(glb_type(0x6004_0000), GLB_STREAM_MSG);
    assert_eq!(cmd_type(0x6004_0000), STREAM_TRIG_START);
}

#[test]
fn buffer_new_layout() {
    let mut b = dirty();
    let n = encode_buffer_new(
        &mut b,
        &BufferNew { id: 2, pipeline_id: 10, core: 0, size: 768, caps: 113, flags: 0 },
    )
    .unwrap();
    assert_eq!(n, BUFFER_SIZE);
    assert_eq!(n, 44);
    let want: [u32; 11] = [44, 0x3020_0000, 2, COMP_BUFFER, 10, 0, 0, 768, 113, 0, 0];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(u32_at(&b, i * 4), *w, "word {i}");
    }
    assert_eq!(b[n], 0xAA, "nothing written past the message");
}

#[test]
fn host_comp_layout() {
    let mut b = dirty();
    let uuid = [0x11u8; UUID_SIZE];
    let h = HostComp {
        id: 1,
        pipeline_id: 10,
        core: 0,
        config: CompConfig { periods_sink: 0, periods_source: 2, frame_fmt: 0, xrun_action: 0 },
        direction: STREAM_CAPTURE,
        no_irq: 0,
        dmac_config: 0,
        uuid: Some(uuid),
    };
    let n = encode_host_comp(&mut b, &h).unwrap();
    assert_eq!(n, 92);
    assert_eq!(u32_at(&b, 0), 92);
    assert_eq!(u32_at(&b, 4), 0x3001_0000);
    assert_eq!(u32_at(&b, 8), 1); // id
    assert_eq!(u32_at(&b, 12), COMP_HOST);
    assert_eq!(u32_at(&b, 16), 10); // pipeline_id
    assert_eq!(u32_at(&b, 20), 0); // core
    assert_eq!(u32_at(&b, 24), 16); // ext_data_length
    assert_eq!(u32_at(&b, 28), 36); // config.hdr.size
    assert_eq!(u32_at(&b, 32), 0); // config.hdr.cmd
    assert_eq!(u32_at(&b, 36), 0); // periods_sink
    assert_eq!(u32_at(&b, 40), 2); // periods_source
    assert_eq!(u32_at(&b, 64), STREAM_CAPTURE);
    assert_eq!(&b[76..92], &uuid);

    let h = HostComp { uuid: None, ..h };
    let n = encode_host_comp(&mut b, &h).unwrap();
    assert_eq!(n, COMP_HOST_SIZE);
    assert_eq!(u32_at(&b, 0), 76);
    assert_eq!(u32_at(&b, 24), 0);
}

#[test]
fn dai_comp_layout() {
    let mut b = dirty();
    let d = DaiComp {
        id: 3,
        pipeline_id: 10,
        core: 0,
        config: CompConfig { periods_sink: 2, periods_source: 0, frame_fmt: FRAME_S32_LE, xrun_action: 0 },
        direction: STREAM_CAPTURE,
        dai_index: 0,
        dai_type: DAI_INTEL_DMIC,
        uuid: None,
    };
    let n = encode_dai_comp(&mut b, &d).unwrap();
    assert_eq!(n, 80);
    assert_eq!(u32_at(&b, 0), 80);
    assert_eq!(u32_at(&b, 12), COMP_DAI);
    assert_eq!(u32_at(&b, 36), 2); // periods_sink
    assert_eq!(u32_at(&b, 48), FRAME_S32_LE); // config.frame_fmt
    assert_eq!(u32_at(&b, 64), STREAM_CAPTURE);
    assert_eq!(u32_at(&b, 68), 0); // dai_index
    assert_eq!(u32_at(&b, 72), DAI_INTEL_DMIC);
    assert_eq!(u32_at(&b, 76), 0); // reserved
    let n = encode_dai_comp(&mut b, &DaiComp { uuid: Some([7; 16]), ..d }).unwrap();
    assert_eq!(n, 96);
    assert_eq!(u32_at(&b, 24), 16);
    assert_eq!(&b[80..96], &[7; 16]);
}

#[test]
fn pipe_new_layout() {
    let mut b = dirty();
    let p = PipeNew {
        comp_id: 4,
        pipeline_id: 10,
        sched_id: 3,
        core: 0,
        period_us: 1000,
        priority: 0,
        period_mips: 5000,
        frames_per_sched: 0,
        xrun_limit_us: 0,
        time_domain: TIME_DOMAIN_TIMER,
    };
    let n = encode_pipe_new(&mut b, &p).unwrap();
    assert_eq!(n, 48);
    let want: [u32; 12] = [48, 0x3010_0000, 4, 10, 3, 0, 1000, 0, 5000, 0, 0, 1];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(u32_at(&b, i * 4), *w, "word {i}");
    }
}

#[test]
fn small_topology_messages() {
    let mut b = dirty();
    assert_eq!(encode_comp_connect(&mut b, 3, 2).unwrap(), 16);
    assert_eq!([u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8), u32_at(&b, 12)], [16, 0x3003_0000, 3, 2]);
    assert_eq!(encode_pipe_complete(&mut b, 4).unwrap(), 12);
    assert_eq!([u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8)], [12, 0x3013_0000, 4]);
    assert_eq!(encode_comp_free(&mut b, 1).unwrap(), 12);
    assert_eq!([u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8)], [12, 0x3002_0000, 1]);
    assert_eq!(encode_buffer_free(&mut b, 2).unwrap(), 12);
    assert_eq!(u32_at(&b, 4), 0x3021_0000);
    assert_eq!(encode_pipe_free(&mut b, 4).unwrap(), 12);
    assert_eq!(u32_at(&b, 4), 0x3011_0000);
}

fn dmic_config() -> DaiConfigDmic {
    let mut pdm = [DmicPdm::default(); DMIC_NUM_CTRL];
    pdm[0] = DmicPdm { id: 0, enable_mic_a: 1, enable_mic_b: 1, ..DmicPdm::default() };
    pdm[1] = DmicPdm { id: 1, enable_mic_a: 1, enable_mic_b: 0, skew: 5, ..DmicPdm::default() };
    DaiConfigDmic {
        dai_index: 0,
        format: 0,
        group_id: 0,
        flags: DAI_CONFIG_FLAGS_HW_PARAMS,
        dmic: DmicParams {
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
            pdm,
        },
    }
}

#[test]
fn dai_config_dmic_layout() {
    let mut b = dirty();
    let c = dmic_config();
    let n = encode_dai_config_dmic(&mut b, &c).unwrap();
    assert_eq!(n, DAI_CONFIG_SIZE);
    assert_eq!(n, 216);
    assert_eq!(u32_at(&b, 0), 216);
    assert_eq!(u32_at(&b, 4), 0x8001_0000);
    assert_eq!(u32_at(&b, 8), DAI_INTEL_DMIC);
    assert_eq!(u32_at(&b, 12), 0); // dai_index
    assert_eq!(u16_at(&b, 16), 0); // format
    assert_eq!(b[18], 0); // group_id
    assert_eq!(b[19], DAI_CONFIG_FLAGS_HW_PARAMS);
    assert!(b[20..52].iter().all(|&x| x == 0), "reserved before the union");
    let d = 52; // sof_ipc_dai_dmic_params
    assert_eq!(u32_at(&b, d), 0); // its own hdr.size, left 0 as Linux does
    assert_eq!(u32_at(&b, d + 4), 1); // driver_ipc_version
    assert_eq!(u32_at(&b, d + 8), 2_400_000);
    assert_eq!(u32_at(&b, d + 12), 4_800_000);
    assert_eq!(u32_at(&b, d + 16), 48_000); // fifo_fs
    assert_eq!(u32_at(&b, d + 20), 0); // reserved_1
    assert_eq!(u16_at(&b, d + 24), 32); // fifo_bits
    assert_eq!(u16_at(&b, d + 26), 0); // fifo_bits_b
    assert_eq!(u16_at(&b, d + 28), 40);
    assert_eq!(u16_at(&b, d + 30), 60);
    assert_eq!(u32_at(&b, d + 32), 1); // num_pdm_active
    assert_eq!(u32_at(&b, d + 44), 200); // unmute_ramp_time
    let p = d + 68; // pdm[0]
    assert_eq!(p, 120);
    assert_eq!(u16_at(&b, p + 4), 0); // id
    assert_eq!(u16_at(&b, p + 6), 1); // mic a
    assert_eq!(u16_at(&b, p + 8), 1); // mic b
    assert_eq!(u16_at(&b, p + 16), 0); // skew
    // pdm[1] is past num_pdm_active and stays zero.
    assert!(b[p + 24..216].iter().all(|&x| x == 0));

    let mut two = c;
    two.dmic.num_pdm_active = 2;
    encode_dai_config_dmic(&mut b, &two).unwrap();
    let q = p + DMIC_PDM_CTRL_SIZE;
    assert_eq!(u16_at(&b, q + 4), 1);
    assert_eq!(u16_at(&b, q + 6), 1);
    assert_eq!(u16_at(&b, q + 8), 0);
    assert_eq!(u16_at(&b, q + 16), 5);

    let mut bad = c;
    bad.dmic.num_pdm_active = 5;
    assert_eq!(encode_dai_config_dmic(&mut b, &bad), Err(Error::Invalid("dmic num_pdm_active")));
}

#[test]
fn pcm_params_layout() {
    let mut b = dirty();
    let p = PcmParams {
        comp_id: 1,
        flags: 0,
        buffer: HostBuffer { phy_addr: 0x1234_5000, pages: 4, size: 16384 },
        direction: STREAM_CAPTURE,
        frame_fmt: FRAME_S32_LE,
        buffer_fmt: BUFFER_INTERLEAVED,
        rate: 48_000,
        stream_tag: 5,
        channels: 2,
        sample_valid_bytes: 4,
        sample_container_bytes: 4,
        host_period_bytes: 4096,
        no_stream_position: 1,
        cont_update_posn: 0,
        chmap: [0; MAX_CHANNELS],
    };
    let n = encode_pcm_params(&mut b, &p).unwrap();
    assert_eq!(n, PCM_PARAMS_SIZE);
    assert_eq!(n, 108);
    assert_eq!(u32_at(&b, 0), 108);
    assert_eq!(u32_at(&b, 4), 0x6001_0000);
    assert_eq!(u32_at(&b, 8), 1); // comp_id
    assert_eq!(u32_at(&b, 12), 0); // flags
    let s = 24; // sof_ipc_stream_params
    assert_eq!(u32_at(&b, s), 84); // params.hdr.size
    assert_eq!(u32_at(&b, s + 4), 0); // buffer.hdr.size
    assert_eq!(u32_at(&b, s + 8), 0x1234_5000);
    assert_eq!(u32_at(&b, s + 12), 4);
    assert_eq!(u32_at(&b, s + 16), 16384);
    assert_eq!(u32_at(&b, s + 32), STREAM_CAPTURE);
    assert_eq!(u32_at(&b, s + 36), FRAME_S32_LE);
    assert_eq!(u32_at(&b, s + 40), BUFFER_INTERLEAVED);
    assert_eq!(u32_at(&b, s + 44), 48_000);
    assert_eq!(u16_at(&b, s + 48), 5); // stream_tag
    assert_eq!(u16_at(&b, s + 50), 2); // channels
    assert_eq!(u16_at(&b, s + 52), 4);
    assert_eq!(u16_at(&b, s + 54), 4);
    assert_eq!(u32_at(&b, s + 56), 4096);
    assert_eq!(u16_at(&b, s + 60), 1); // no_stream_position
    assert_eq!(b[s + 62], 0);
    assert_eq!(u32_at(&b, s + 64), 0); // ext_data_length
    assert!(b[s + 68..108].iter().all(|&x| x == 0), "chmap");

    assert_eq!(encode_pcm_params(&mut b, &PcmParams { channels: 9, ..p }), Err(Error::Invalid("pcm channels")));
}

#[test]
fn stream_and_pm_messages() {
    let mut b = dirty();
    assert_eq!(encode_trigger_start(&mut b, 1).unwrap(), 12);
    assert_eq!([u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8)], [12, 0x6004_0000, 1]);
    encode_trigger_stop(&mut b, 1).unwrap();
    assert_eq!(u32_at(&b, 4), 0x6005_0000);
    encode_pcm_free(&mut b, 1).unwrap();
    assert_eq!(u32_at(&b, 4), 0x6003_0000);
    assert_eq!(encode_pm_core_enable(&mut b, 1).unwrap(), 12);
    assert_eq!([u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8)], [12, 0x4007_0000, 1]);
    assert_eq!(encode_pm_gate(&mut b, 3).unwrap(), 32);
    assert_eq!(u32_at(&b, 4), 0x4008_0000);
    assert!(b[12..32].iter().all(|&x| x == 0));
    assert_eq!(encode_pm_ctx_save(&mut b).unwrap(), 76);
    assert_eq!(u32_at(&b, 4), 0x4001_0000);
}

#[test]
fn encoders_refuse_short_buffers() {
    let mut small = [0xAAu8; 100];
    let p = PipeNew {
        comp_id: 0,
        pipeline_id: 0,
        sched_id: 0,
        core: 0,
        period_us: 0,
        priority: 0,
        period_mips: 0,
        frames_per_sched: 0,
        xrun_limit_us: 0,
        time_domain: 0,
    };
    assert_eq!(encode_pipe_new(&mut small[..47], &p), Err(Error::BufferTooSmall { need: 48, have: 47 }));
    assert_eq!(
        encode_dai_config_dmic(&mut small, &dmic_config()),
        Err(Error::BufferTooSmall { need: 216, have: 100 })
    );
    assert!(small.iter().all(|&x| x == 0xAA), "nothing written on failure");
}

#[test]
fn reply_decode() {
    let mut r = [0u8; 12];
    r[0..4].copy_from_slice(&12u32.to_le_bytes());
    r[4..8].copy_from_slice(&(GLB_REPLY | TPLG_COMP_NEW).to_le_bytes());
    let ok = Reply::parse(&r).unwrap();
    assert_eq!(ok.hdr.glb_type(), GLB_REPLY);
    assert_eq!(ok.error, 0);
    assert_eq!(ok.result(), Ok(()));
    r[8..12].copy_from_slice(&(-22i32).to_le_bytes());
    assert_eq!(Reply::parse(&r).unwrap().result(), Err(Error::Firmware(-22)));
    assert_eq!(Reply::parse(&r[..8]), Err(Error::Truncated { need: 12, have: 8 }));
    r[0..4].copy_from_slice(&40u32.to_le_bytes());
    assert_eq!(Reply::parse(&r), Err(Error::BadSize(40)));
}

#[test]
fn pcm_params_reply_decode() {
    let mut r = [0u8; 20];
    r[0..4].copy_from_slice(&20u32.to_le_bytes());
    r[4..8].copy_from_slice(&(GLB_REPLY | STREAM_PCM_PARAMS_REPLY).to_le_bytes());
    r[12..16].copy_from_slice(&1u32.to_le_bytes());
    r[16..20].copy_from_slice(&0x180u32.to_le_bytes());
    let p = PcmParamsReply::parse(&r).unwrap();
    assert_eq!(p.comp_id, 1);
    assert_eq!(p.posn_offset, 0x180);
    // A firmware error comes back as a bare 12-byte reply.
    let mut e = [0u8; 12];
    e[0..4].copy_from_slice(&12u32.to_le_bytes());
    e[8..12].copy_from_slice(&(-5i32).to_le_bytes());
    assert_eq!(PcmParamsReply::parse(&e), Err(Error::Firmware(-5)));
}

#[test]
fn stream_posn_decode() {
    let mut r = [0u8; STREAM_POSN_SIZE];
    r[0..4].copy_from_slice(&76u32.to_le_bytes());
    r[4..8].copy_from_slice(&(GLB_STREAM_MSG | STREAM_POSITION).to_le_bytes());
    r[12..16].copy_from_slice(&1u32.to_le_bytes());
    r[20..24].copy_from_slice(&38_400_000u32.to_le_bytes());
    r[28..36].copy_from_slice(&0x1_0000_0040u64.to_le_bytes());
    r[36..44].copy_from_slice(&0x80u64.to_le_bytes());
    r[52..60].copy_from_slice(&12345u64.to_le_bytes());
    r[68..72].copy_from_slice(&3u32.to_le_bytes());
    r[72..76].copy_from_slice(&(-64i32).to_le_bytes());
    let p = StreamPosn::parse(&r).unwrap();
    assert_eq!(p.reply.hdr.cmd_type(), STREAM_POSITION);
    assert_eq!(p.comp_id, 1);
    assert_eq!(p.wallclock_hz, 38_400_000);
    assert_eq!(p.host_posn, 0x1_0000_0040);
    assert_eq!(p.dai_posn, 0x80);
    assert_eq!(p.wallclock, 12345);
    assert_eq!(p.xrun_comp_id, 3);
    assert_eq!(p.xrun_size, -64);
    assert!(StreamPosn::parse(&r[..75]).is_err());
}
