// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The DSP boot against the register model: the good path (write order, what the
//! ROM receives, FW_READY bytes, transport), one timeout per step, and the
//! clean-up that leaves Bell's codec streams alone.

#[path = "boot_model.rs"]
mod boot_model;
#[path = "fw_builder.rs"]
mod fw_builder;

use boot_model::{Faults, Model, W, PP_AT, SPIB_AT};
use sex_sof::boot::{self, BootError, Cause, Config, IpcError, Step, Transport};
use sex_sof::regs::{dsp, hda, timeout};

fn run(faults: Faults) -> (Model, Result<boot::Booted, BootError>, [u8; 1024]) {
    let img = fw_builder::image();
    let mut m = Model::new(&fw_builder::payload(), faults);
    let mut ready = [0u8; 1024];
    let r = boot::boot(&mut m, &img, &Config::default(), &mut ready);
    (m, r, ready)
}

fn is_d(off: u32, f: impl Fn(u32) -> bool) -> impl Fn(&W) -> bool {
    move |w| matches!(*w, W::D32(o, v) if o == off && f(v))
}

fn is_h32(off: u32, f: impl Fn(u32) -> bool) -> impl Fn(&W) -> bool {
    move |w| matches!(*w, W::H32(o, v) if o == off && f(v))
}

fn fail(faults: Faults) -> (Model, BootError) {
    let (m, r, _) = run(faults);
    (m, r.expect_err("boot must fail"))
}

/// After any failure that touched the DSP: core 0 off, PP off, streams coupled,
/// vendor registers restored.
fn assert_cleaned_up(m: &Model) {
    let cs = m.d32(dsp::ADSPCS);
    assert_eq!(cs & dsp::adspcs_spa(dsp::CORE0), 0, "SPA cleared");
    assert_eq!(cs & dsp::adspcs_cpa(dsp::CORE0), 0, "core 0 unpowered");
    assert_eq!(m.h32(PP_AT + hda::PP_PPCTL), 0, "GPROCEN, PIE and PROCEN all clear");
    assert_eq!(m.h32(hda::VS_EM2) & hda::VS_EM2_L1SEN, hda::VS_EM2_L1SEN, "L1SEN restored");
    assert_eq!(m.h8(hda::VS_LTRP) & hda::VS_LTRP_GB_MASK, 0x20, "guardband restored");
    assert_eq!(m.h8(hda::VS_LTRP) & !hda::VS_LTRP_GB_MASK, 0xc0, "other LTRP bits kept");
    assert_eq!(m.d32(dsp::ADSPIC) & dsp::ADSPIC_IPC, 0, "IPC interrupt off");
}

#[test]
fn good_boot_follows_the_linux_order() {
    let (m, r, ready) = run(Faults::default());
    let booted = r.expect("boot");
    let loader = Model::loader_index();
    let sd = hda::sd(loader);
    let ppctl = PP_AT + hda::PP_PPCTL;
    let c = dsp::CORE0;

    // What the ROM saw.
    let purge = m.purge.expect("purge IPC");
    assert_eq!(
        purge,
        dsp::HIPCIDR_BUSY | dsp::ROM_IPC_CONTROL | dsp::ROM_IPC_PURGE_FW | (Model::loader_tag() - 1) << 9
    );
    assert_eq!(m.dma_bytes, fw_builder::payload(), "ROM received exactly the payload");
    assert!(m.syncs >= 1, "scratch synced before DMA");

    // Order of the important writes.
    let gprocen = m.first(is_h32(ppctl, |v| v & hda::PPCTL_GPROCEN != 0 && v & hda::PPCTL_PIE != 0)).unwrap();
    let em2_off = m.first(is_h32(hda::VS_EM2, |v| v & hda::VS_EM2_L1SEN == 0)).unwrap();
    let iccmax_run = m
        .first(is_h32(hda::sd(Model::iccmax_index()), |v| v & hda::SD_CTL_RUN != 0))
        .unwrap();
    let procen = m.first(is_h32(ppctl, |v| v & hda::ppctl_procen(loader) != 0)).unwrap();
    let srst = m.first(is_h32(sd, |v| v & hda::SD_CTL_SRST != 0)).unwrap();
    let fmt = m.first(|w| *w == W::H16(sd + hda::SD_FMT, hda::CL_STREAM_FORMAT)).unwrap();
    let lvi = m.first(|w| *w == W::H16(sd + hda::SD_LVI, 1)).unwrap();
    let spib = m.first(|w| matches!(*w, W::H32(o, _) if o == SPIB_AT + hda::spib(loader))).unwrap();
    let spa = m.first(is_d(dsp::ADSPCS, |v| v & dsp::adspcs_spa(c) != 0)).unwrap();
    let ssp = m.first(|w| matches!(*w, W::D32(o, _) if o == dsp::SSP_BASE + dsp::SSP_SSC1)).unwrap();
    let ipc = m.first(is_d(dsp::HIPCIDR, |v| v == purge)).unwrap();
    let unreset = m.first(is_d(dsp::ADSPCS, |v| v & dsp::adspcs_spa(c) != 0 && v & dsp::adspcs_crst(c) == 0)).unwrap();
    let unstall = m.first(is_d(dsp::ADSPCS, |v| v & dsp::adspcs_spa(c) != 0 && v & dsp::adspcs_cstall(c) == 0)).unwrap();
    let ack = m.first(is_d(dsp::HIPCIDA, |v| v & dsp::HIPCIDA_DONE != 0)).unwrap();
    let ipc_int = m.first(is_d(dsp::ADSPIC, |v| v & dsp::ADSPIC_IPC != 0)).unwrap();
    let intctl = m.first(is_h32(hda::INTCTL, |v| v & 1 << loader != 0)).unwrap();
    let run = m.first(is_h32(sd, |v| v & hda::SD_CTL_RUN != 0)).unwrap();
    let stop = m.last(is_h32(sd, |v| v & hda::SD_CTL_RUN == 0)).unwrap();
    let recouple = m.last(is_h32(ppctl, |v| v & hda::ppctl_procen(loader) == 0)).unwrap();
    let em2_back = m.last(is_h32(hda::VS_EM2, |v| v & hda::VS_EM2_L1SEN != 0)).unwrap();
    let order = [
        ("gprocen", gprocen),
        ("em2 l1sen off", em2_off),
        ("iccmax run", iccmax_run),
        ("loader procen", procen),
        ("loader reset", srst),
        ("loader fmt", fmt),
        ("loader lvi", lvi),
        ("loader spib", spib),
        ("core 0 spa", spa),
        ("ssp consumer", ssp),
        ("rom purge ipc", ipc),
        ("core 0 unreset", unreset),
        ("core 0 unstall", unstall),
        ("rom ipc ack", ack),
        ("ipc interrupts", ipc_int),
        ("loader intctl", intctl),
        ("loader run", run),
        ("loader stop", stop),
        ("loader recoupled", recouple),
        ("em2 restored", em2_back),
    ];
    for pair in order.windows(2) {
        assert!(pair[0].1 < pair[1].1, "{} (#{}) must come before {} (#{})", pair[0].0, pair[0].1, pair[1].0, pair[1].1);
    }

    // The stream tag and format of the loader.
    let tagged = m.first(is_h32(sd, |v| (v & hda::SD_CTL_TAG_MASK) >> hda::SD_CTL_TAG_SHIFT == Model::loader_tag()));
    assert!(tagged.is_some_and(|t| t < run));

    // After the boot: DSP running, PP on with PIE, borrowed streams coupled again.
    let ppv = m.h32(ppctl);
    assert_eq!(ppv, hda::PPCTL_GPROCEN | hda::PPCTL_PIE);
    assert_eq!(m.h32(hda::INTCTL) & 1 << loader, 0);
    assert_eq!(m.h32(sd + hda::SD_BDPL), 0);
    assert_eq!(m.h8(hda::VS_LTRP) & hda::VS_LTRP_GB_MASK, 0x20);
    assert_eq!(m.d32(dsp::HIPCCTL), dsp::HIPCCTL_BUSY | dsp::HIPCCTL_DONE);

    // FW_READY bytes handed over raw, doorbell answered.
    let want = boot_model::fw_ready_bytes();
    assert_eq!(booted.fw_ready_len, dsp::DEFAULT_DSP_BOX.size.min(1024) as usize);
    assert_eq!(&ready[..want.len()], &want[..]);
    assert_eq!(m.d32(dsp::HIPCTDR) & dsp::HIPCTDR_BUSY, 0);
    assert_ne!(m.d32(dsp::HIPCTDA) & dsp::HIPCTDA_DONE, 0);

    // The transport uses the mailboxes from the image.
    assert_eq!(booted.transport, Transport::new(dsp::DEFAULT_HOST_BOX, dsp::DEFAULT_DSP_BOX));
    assert_eq!(booted.caps.pp, PP_AT);
    assert_eq!(booted.caps.spib, Some(SPIB_AT));
    assert_eq!(booted.image.payload_len, fw_builder::PAYLOAD_LEN);
}

#[test]
fn messages_before_fw_ready_are_dropped() {
    let (_, r, ready) = run(Faults { noise_before_ready: true, ..Faults::default() });
    r.expect("boot");
    assert_eq!(&ready[..8], &boot_model::fw_ready_bytes()[..8]);
}

#[test]
fn controller_not_ready() {
    let (m, e) = fail(Faults { controller_in_reset: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::ControllerReady, Cause::Timeout));
    assert!(m.elapsed_us >= timeout::CTRL_RESET_US as u64);
    assert!(m.log.is_empty(), "nothing written");
}

#[test]
fn no_processing_pipe() {
    let (m, e) = fail(Faults { no_pp: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::Capabilities, Cause::Missing));
    assert!(m.log.is_empty());
}

#[test]
fn bad_image_and_small_scratch() {
    let mut m = Model::new(&[], Faults::default());
    let mut ready = [0u8; 64];
    let e = boot::boot(&mut m, &[0u8; 0x4000], &Config::default(), &mut ready).unwrap_err();
    assert_eq!(e.step, Step::Image);
    assert!(matches!(e.cause, Cause::Image(_)));
    assert!(m.log.is_empty());

    struct Small(Model, Vec<u8>);
    impl sex_sof::hw::Hw for Small {
        fn hda_read8(&mut self, o: u32) -> u8 { self.0.hda_read8(o) }
        fn hda_read16(&mut self, o: u32) -> u16 { self.0.hda_read16(o) }
        fn hda_read32(&mut self, o: u32) -> u32 { self.0.hda_read32(o) }
        fn hda_write8(&mut self, o: u32, v: u8) { self.0.hda_write8(o, v) }
        fn hda_write16(&mut self, o: u32, v: u16) { self.0.hda_write16(o, v) }
        fn hda_write32(&mut self, o: u32, v: u32) { self.0.hda_write32(o, v) }
        fn dsp_read32(&mut self, o: u32) -> u32 { self.0.dsp_read32(o) }
        fn dsp_write32(&mut self, o: u32, v: u32) { self.0.dsp_write32(o, v) }
        fn delay_us(&mut self, us: u32) { self.0.delay_us(us) }
        fn dma(&mut self) -> (&mut [u8], u64) { (&mut self.1, boot_model::BUS) }
    }
    let mut s = Small(Model::new(&[], Faults::default()), vec![0; 0x1000]);
    let e = boot::boot(&mut s, &fw_builder::image(), &Config::default(), &mut ready).unwrap_err();
    assert_eq!((e.step, e.cause), (Step::Scratch, Cause::Scratch));
    assert!(s.0.log.is_empty());
}

#[test]
fn loader_stream_reset_times_out() {
    let (m, e) = fail(Faults { stream_reset_stuck: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::LoaderStream, Cause::Timeout));
    assert_cleaned_up(&m);
}

#[test]
fn core_power_up_times_out_after_three_attempts() {
    let (m, e) = fail(Faults { power_stuck: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::CorePowerUp, Cause::Timeout));
    // Rising edges of SPA in the write log: one per attempt.
    let spa = dsp::adspcs_spa(dsp::CORE0);
    let mut on = false;
    let mut spa_writes = 0;
    for w in &m.log {
        if let W::D32(dsp::ADSPCS, v) = *w {
            if v & spa != 0 && !on {
                spa_writes += 1;
            }
            on = v & spa != 0;
        }
    }
    assert_eq!(spa_writes, timeout::BOOT_ATTEMPTS as usize);
    assert!(m.elapsed_us >= (timeout::BOOT_ATTEMPTS * timeout::CORE_US) as u64);
    assert_cleaned_up(&m);
}

#[test]
fn core_reset_times_out() {
    let (m, e) = fail(Faults { reset_stuck: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::CoreReset, Cause::Timeout));
    assert_cleaned_up(&m);
}

#[test]
fn core_unstall_fails() {
    let (m, e) = fail(Faults { stall_stuck: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::CoreRun, Cause::Timeout));
    assert_cleaned_up(&m);
}

#[test]
fn rom_ipc_times_out() {
    let (m, e) = fail(Faults { rom_ipc_silent: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::RomIpc, Cause::Timeout));
    assert!(m.elapsed_us >= (timeout::BOOT_ATTEMPTS * timeout::ROM_IPC_US) as u64);
    assert_cleaned_up(&m);
}

#[test]
fn rom_init_times_out() {
    let (m, e) = fail(Faults { rom_init_stuck: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::RomInit, Cause::Timeout));
    assert!(m.elapsed_us >= (timeout::BOOT_ATTEMPTS * timeout::ROM_INIT_US) as u64);
    assert!(m.dma_bytes.is_empty(), "DMA never started");
    assert_cleaned_up(&m);
}

#[test]
fn fw_entered_times_out_and_dma_is_stopped() {
    let (m, e) = fail(Faults { no_fw_entered: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::FwEntered, Cause::Timeout));
    assert_eq!(e.rom_status & dsp::FSR_STATE_MASK, dsp::FSR_STATE_INIT_DONE, "ROM status reported");
    let sd = hda::sd(Model::loader_index());
    assert_eq!(m.h32(sd) & hda::SD_CTL_RUN, 0);
    assert_cleaned_up(&m);
}

#[test]
fn fw_ready_times_out() {
    let (m, e) = fail(Faults { no_fw_ready: true, ..Faults::default() });
    assert_eq!((e.step, e.cause), (Step::FwReady, Cause::Timeout));
    assert_eq!(e.rom_status & dsp::FSR_STATE_MASK, dsp::FSR_STATE_FW_ENTERED);
    assert!(m.elapsed_us >= timeout::FW_READY_US as u64);
    assert_cleaned_up(&m);
}

#[test]
fn fw_panic_is_reported() {
    let (m, e) = fail(Faults { panic: true, ..Faults::default() });
    assert_eq!(e.step, Step::FwReady);
    assert_eq!(e.cause, Cause::Panic(dsp::HIPCTDR_BUSY | dsp::IPC_PANIC_MAGIC | 0x5));
    assert_cleaned_up(&m);
}

#[test]
fn step_names_are_distinct() {
    let all = [
        Step::ControllerReady,
        Step::Capabilities,
        Step::Image,
        Step::Scratch,
        Step::IccmaxStream,
        Step::LoaderStream,
        Step::CorePowerUp,
        Step::CoreReset,
        Step::CoreRun,
        Step::RomIpc,
        Step::RomInit,
        Step::DmaStart,
        Step::FwEntered,
        Step::DmaStop,
        Step::FwReady,
        Step::CoreStall,
        Step::CorePowerDown,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a.name(), b.name());
        }
    }
}

#[test]
fn power_down_after_boot() {
    let (mut m, r, _) = run(Faults::default());
    r.expect("boot");
    boot::power_down(&mut m).expect("power down");
    let stall = m.last(is_d(dsp::ADSPCS, |v| v & dsp::adspcs_cstall(dsp::CORE0) != 0 && v & dsp::adspcs_spa(dsp::CORE0) != 0)).unwrap();
    let off = m.last(is_d(dsp::ADSPCS, |v| v & dsp::adspcs_spa(dsp::CORE0) == 0)).unwrap();
    assert!(stall < off);
    assert_cleaned_up(&m);
}

#[test]
fn power_down_times_out_when_power_sticks() {
    let (m, r, _) = run(Faults::default());
    r.expect("boot");
    // CPA never clears: a wrapper keeps reporting it.
    struct Sticky(Model);
    impl sex_sof::hw::Hw for Sticky {
        fn hda_read8(&mut self, o: u32) -> u8 { self.0.hda_read8(o) }
        fn hda_read16(&mut self, o: u32) -> u16 { self.0.hda_read16(o) }
        fn hda_read32(&mut self, o: u32) -> u32 { self.0.hda_read32(o) }
        fn hda_write8(&mut self, o: u32, v: u8) { self.0.hda_write8(o, v) }
        fn hda_write16(&mut self, o: u32, v: u16) { self.0.hda_write16(o, v) }
        fn hda_write32(&mut self, o: u32, v: u32) { self.0.hda_write32(o, v) }
        fn dsp_read32(&mut self, o: u32) -> u32 {
            let v = self.0.dsp_read32(o);
            if o == dsp::ADSPCS { v | dsp::adspcs_cpa(dsp::CORE0) } else { v }
        }
        fn dsp_write32(&mut self, o: u32, v: u32) { self.0.dsp_write32(o, v) }
        fn delay_us(&mut self, us: u32) { self.0.delay_us(us) }
        fn dma(&mut self) -> (&mut [u8], u64) { self.0.dma() }
    }
    let mut s = Sticky(m);
    let e = boot::power_down(&mut s).unwrap_err();
    assert_eq!((e.step, e.cause), (Step::CorePowerDown, Cause::Timeout));
}

#[test]
fn transport_send_and_reply() {
    let (mut m, r, _) = run(Faults::default());
    let mut t = r.expect("boot").transport;
    let mut msg = [0u8; 20];
    msg[0..4].copy_from_slice(&20u32.to_le_bytes());
    msg[4..8].copy_from_slice(&0x3001_0000u32.to_le_bytes());
    msg[8..20].copy_from_slice(b"hello, dsp!!");
    let mut reply = [0u8; 64];
    let n = t.send(&mut m, &msg, &mut reply).expect("reply");
    assert_eq!(n, 12);
    assert_eq!(&reply[4..8], &0x3001_0000u32.to_le_bytes());
    assert_eq!(m.ipc_rx.last().unwrap(), &msg.to_vec());
    assert_eq!(m.d32(dsp::HIPCIDA) & dsp::HIPCIDA_DONE, 0, "DONE cleared");
    assert_eq!(m.d32(dsp::HIPCCTL) & dsp::HIPCCTL_DONE, dsp::HIPCCTL_DONE, "DONE interrupt re-enabled");
    // Order: mailbox written before the doorbell.
    let mbox = m.last(|w| matches!(*w, W::D32(o, _) if o == dsp::DEFAULT_HOST_BOX.offset + 16)).unwrap();
    let bell = m.last(is_d(dsp::HIPCIDR, |v| v == dsp::HIPCIDR_BUSY)).unwrap();
    assert!(mbox < bell);

    // Errors.
    assert_eq!(t.send(&mut m, &[], &mut reply), Err(IpcError::TooLarge));
    assert_eq!(t.send(&mut m, &msg, &mut reply[..8]), Err(IpcError::ReplyTooLarge(12)));
    m.faults.ipc_silent = true;
    assert_eq!(t.send(&mut m, &msg, &mut reply), Err(IpcError::Timeout));
    assert_eq!(t.send(&mut m, &msg, &mut reply), Err(IpcError::Busy), "unanswered request still pending");
}

#[test]
fn transport_notifications() {
    let (mut m, r, _) = run(Faults::default());
    let mut t = r.expect("boot").transport;
    let mut buf = [0u8; 32];
    assert_eq!(t.poll(&mut m, &mut buf), Ok(None));
    let mut body = [0u8; 16];
    body[0..4].copy_from_slice(&16u32.to_le_bytes());
    body[4..8].copy_from_slice(&0x6001_0000u32.to_le_bytes());
    m.notify(0x3000_0000, 0x42, &body);
    let n = t.wait(&mut m, &mut buf, timeout::IPC_US).expect("notification");
    assert_eq!((n.primary, n.extension, n.len), (0x3000_0000, 0x42, 16));
    assert_eq!(&buf[..16], &body);
    assert_eq!(m.d32(dsp::HIPCTDR) & dsp::HIPCTDR_BUSY, 0);
    assert_ne!(m.d32(dsp::HIPCTDA) & dsp::HIPCTDA_DONE, 0);
    assert_eq!(t.wait(&mut m, &mut buf, 1_000), Err(IpcError::Timeout));
    m.set_d32(dsp::HIPCTDD, 0x80);
    m.set_d32(dsp::HIPCTDR, dsp::HIPCTDR_BUSY | dsp::IPC_PANIC_MAGIC | 0x7);
    assert_eq!(
        t.poll(&mut m, &mut buf),
        Err(IpcError::Panic { code: dsp::HIPCTDR_BUSY | dsp::IPC_PANIC_MAGIC | 0x7, extension: 0x80 })
    );
}
