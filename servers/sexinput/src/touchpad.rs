// SPDX-License-Identifier: GPL-3.0-only
//! The laptop touchpad: an I2C HID Precision Touchpad behind an Intel Serial IO
//! (DesignWare) I2C controller.
//!
//! BOOT leases every Serial IO I2C function to this service; the ACPI service says which
//! one carries the touchpad, at which address and HID descriptor register
//! (`sex_pdx::touchpad::Found`). Bring-up and frames come from `sex-hid-touch`, the bus
//! from `sex-i2c-dw`. Frames become ordinary pointer events (motion, buttons, wheel),
//! three-finger swipes become Alt+Tab and pinches Ctrl+= / Ctrl+-.
//!
//! When the ACPI service names the pad's I/O APIC line (`Found::irq`), the pad is read
//! when that line fires and not otherwise; a level line comes back masked by the kernel
//! and is unmasked once the read quieted the pad. Without a line, or once the line
//! proves stuck or silent (`sex_hid_touch::pace`), the pad is polled: ~125 reads a
//! second while a finger is down, ~20 when idle.
use sex_hid_touch::gesture::{Config, Event, Recognizer};
use sex_hid_touch::i2c_hid::{self, HidDescriptor};
use sex_hid_touch::pace::{Next, Pace, Read};
use sex_hid_touch::session::{Bus, ProbeError, Report, Touchpad, MAX_REPORT_DESCRIPTOR};
use sex_i2c_dw::{timing, Controller, Counts, Regs, Speed};
use sex_pdx::memory_region as m;
use sex_pdx::serial_println;
use sex_pdx::touchpad::{Found, Irq, CONTROLLER_SLOT};

struct Mmio { base: usize, length: usize }
impl Regs for Mmio {
    fn read(&mut self, offset: u32) -> u32 {
        let o = offset as usize;
        if o + 4 > self.length { return u32::MAX; }
        unsafe { core::ptr::read_volatile((self.base + o) as *const u32) }
    }
    fn write(&mut self, offset: u32, value: u32) {
        let o = offset as usize;
        if o + 4 > self.length { return; }
        unsafe { core::ptr::write_volatile((self.base + o) as *mut u32, value) }
    }
}

struct OnBus<'a> { c: &'a mut Controller<Mmio>, address: u16, last_error: &'a mut Option<sex_i2c_dw::Error> }
impl Bus for OnBus<'_> {
    fn transfer(&mut self, write: &[u8], read: &mut [u8]) -> bool {
        match self.c.transfer(self.address, write, read) {
            Ok(()) => true,
            Err(e) => { *self.last_error = Some(e); false }
        }
    }
    fn delay_ms(&mut self, ms: u32) {
        if ms > 0 { let _ = sex_pdx::sys_wait_self_ticks(ms as u64); }
    }
}

struct Pad {
    ctrl: Controller<Mmio>,
    address: u16,
    tp: Touchpad,
    rec: Recognizer,
    pace: Pace,
    /// The line's pending bit while it is in use.
    line_bit: Option<u8>,
    /// The line fired and the pad has not been read since.
    fired: bool,
    /// An edge drain wants another read; it waits for the next turn, where Input checks
    /// its outboxes have room again.
    again: bool,
    next_poll: u64,
    failures: u32,
    last_error: Option<sex_i2c_dw::Error>,
    /// Suspended to idle: the pad is in HID `SET_POWER` sleep, its line masked, not read.
    asleep: bool,
    /// Button bitmask for Mouse-collection fallback reports.
    mouse_buttons: u8,
    /// Whether `[touchpad.first_report]` has been logged.
    first_report_logged: bool,
}

static mut PAD: Option<Pad> = None;
static mut SCRATCH: [u8; MAX_REPORT_DESCRIPTOR] = [0; MAX_REPORT_DESCRIPTOR];
static mut FALLBACK_SCANNED: bool = false;

const GIVE_UP_AFTER: u32 = 64;
/// Reads that may empty a pad before its line is armed.
const DRAIN_READS: u32 = 16;
/// Common 7-bit I2C-HID touchpad addresses (Dell/Synaptics/ELAN/Goodix/Alps).
const COMMON_ADDRS: &[u16] = &[0x2c, 0x15, 0x20, 0x38, 0x10];
/// Common HID descriptor registers.
const COMMON_REGS: &[u16] = &[0x0020, 0x0001];

fn leased_serial_io() -> impl Iterator<Item = (u64, sex_pdx::device::Lease)> {
    let base = sex_pdx::device::EXTRA_SLOT_BASE;
    core::iter::once(CONTROLLER_SLOT).chain(base..base + sex_pdx::device::EXTRA_SLOTS)
        .filter_map(|s| sex_pdx::device::at_slot(s).map(|l| (s, l)))
}

fn slot_of(f: &Found) -> Option<(u64, sex_pdx::device::Lease)> {
    leased_serial_io().find(|(_, l)| (l.bus, l.dev, l.func) == (f.bus, f.device, f.function))
}

fn init_controller(slot: u64, lease: sex_pdx::device::Lease, khz: u32, fw_counts: Option<(u16, u16, u16)>) -> Result<Controller<Mmio>, &'static str> {
    let view = sex_pdx::memory_region_full_view(slot, m::QUERY_VIEW | m::READ | m::WRITE).map_err(|_| "no_view")?;
    let reply = sex_pdx::device_command(slot, sex_pdx::pci_command::MEMORY as u64, 0).map_err(|_| "command")?;
    if reply.after & sex_pdx::pci_command::MEMORY == 0 { return Err("memory_off"); }
    // D3hot → D0 recovery, then LPSS reset release.
    let _ = sex_pdx::sys_wait_self_ticks(10);
    let speed = if khz >= 400 { Speed::Fast } else { Speed::Standard };
    let clock = timing::intel_clock(lease.device);
    let counts = fw_counts
        .filter(|&(h, l, _)| h > 0 && l > 0)
        .map(|(high, low, sda_hold)| Counts { high, low, sda_hold })
        .unwrap_or_else(|| timing::counts(clock, speed, 300));
    let mmio = Mmio { base: view.virtual_address as usize, length: view.length as usize };
    let ctrl = match Controller::new(mmio, speed, counts, true) {
        Ok(c) => c,
        Err(e) => {
            serial_println!("[touchpad.i2c.fail] slot={} pci={:02x}:{:02x}.{} id={:04x}:{:04x} err={:?}",
                slot, lease.bus, lease.dev, lease.func, lease.vendor, lease.device, e);
            return Err("not_designware");
        }
    };
    let (tx, rx) = ctrl.fifo_depths();
    serial_println!("[sexinput.i2c.ready] slot={} pci={:02x}:{:02x}.{} product={:#06x} speed={} high={} low={} hold={} from_firmware={} fifo={}/{} version={:#x}",
        slot, lease.bus, lease.dev, lease.func, lease.device, khz.min(400), counts.high, counts.low, counts.sda_hold, fw_counts.is_some() as u8, tx, rx, ctrl.version);
    serial_println!("[touchpad.i2c] pci={:02x}:{:02x}.{} id={:04x}:{:04x} speed={} lpss_reset=1 version={:#x}",
        lease.bus, lease.dev, lease.func, lease.vendor, lease.device, khz.min(400), ctrl.version);
    Ok(ctrl)
}

/// Safe read-only probe for a valid HID-over-I2C 1.00 descriptor on `ctrl`.
fn scan_bus_readonly(ctrl: &mut Controller<Mmio>, preferred_reg: Option<u16>) -> Option<(u16, u16)> {
    let mut raw = [0u8; i2c_hid::DESCRIPTOR_LEN];
    for &addr in COMMON_ADDRS {
        for reg in preferred_reg.into_iter().chain(COMMON_REGS.iter().copied()) {
            if ctrl.transfer(addr, &reg.to_le_bytes(), &mut raw).is_ok() && HidDescriptor::parse(&raw).is_some() {
                return Some((addr, reg));
            }
        }
    }
    None
}

fn finish_pad(ctrl: Controller<Mmio>, lease: sex_pdx::device::Lease, address: u16, hid_reg: u16, tp: Touchpad, irq: Option<Irq>) -> Pad {
    let mode_str = if tp.touch_mode { "ptp" } else if tp.layout.touch_report.is_some() { "touch" } else { "mouse" };
    serial_println!("[sexinput.touchpad.ready] vendor={:#06x} product={:#06x} extent={}x{} fingers={} touch_mode={} report={:?} input_len={}",
        tp.hid.vendor_id, tp.hid.product_id, tp.extent.0, tp.extent.1, tp.layout.fingers, tp.touch_mode as u8, tp.layout.touch_report, tp.input_len());
    serial_println!("[touchpad.ready] pci={:02x}:{:02x}.{} addr={:#x} reg={:#x} vid={:#06x} pid={:#06x} rdesc={} mode={} fingers={}",
        lease.bus, lease.dev, lease.func, address, hid_reg, tp.hid.vendor_id, tp.hid.product_id, tp.hid.report_desc_length, mode_str, tp.layout.fingers);
    let rec = Recognizer::new(Config::for_extent(tp.extent.0));
    let mut pad = Pad {
        ctrl, address, tp, rec, pace: Pace::polling(), line_bit: None, fired: false, again: false,
        next_poll: 0, failures: 0, last_error: None, asleep: false, mouse_buttons: 0, first_report_logged: false,
    };
    if let Some(irq) = irq { arm(&mut pad, irq); } else {
        serial_println!("[sexinput.touchpad.mode] mode=poll reason=no_line");
        serial_println!("[touchpad.mode] mode=poll reason=no_line");
    }
    pad
}

fn open(f: &Found) -> Result<Pad, &'static str> {
    let (slot, lease) = slot_of(f).ok_or("controller_not_leased")?;
    let mut ctrl = init_controller(slot, lease, f.khz, f.counts)?;
    let mut address = f.address;
    let mut hid_reg = f.hid_register;
    let mut last_error = None;
    let mut tries = 0;
    let tp = loop {
        let mut bus = OnBus { c: &mut ctrl, address, last_error: &mut last_error };
        match Touchpad::probe(&mut bus, hid_reg, unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) }) {
            Ok(tp) => break tp,
            Err(ProbeError::NoAnswer) if tries == 0 => {
                // Safe read-only check for alternate address/register on the same controller.
                if let Some((alt_addr, alt_reg)) = scan_bus_readonly(&mut ctrl, Some(hid_reg)) {
                    serial_println!("[touchpad.probe.alt] pci={:02x}:{:02x}.{} addr={:#x}->{:#x} reg={:#x}->{:#x}",
                        lease.bus, lease.dev, lease.func, address, alt_addr, hid_reg, alt_reg);
                    address = alt_addr;
                    hid_reg = alt_reg;
                }
                tries += 1;
                let _ = sex_pdx::sys_wait_self_ticks(50);
            }
            Err(e) if tries < 4 => {
                serial_println!("[sexinput.touchpad.retry] try={} error={:?} bus={:?}", tries, e, last_error);
                tries += 1;
                let _ = sex_pdx::sys_wait_self_ticks(50);
            }
            Err(e) => { serial_println!("[sexinput.touchpad.fail] error={:?} bus={:?}", e, last_error); return Err("probe"); }
        }
    };
    Ok(finish_pad(ctrl, lease, address, hid_reg, tp, f.irq))
}

/// Safe one-time fallback scan across leased Serial IO I2C controllers if ACPI did not
/// deliver a working `Found` message within the boot window.
pub fn fallback_scan(now: u64) {
    if now < 5000 { return; }
    unsafe {
        if (*core::ptr::addr_of!(PAD)).is_some() || *core::ptr::addr_of!(FALLBACK_SCANNED) { return; }
        *core::ptr::addr_of_mut!(FALLBACK_SCANNED) = true;
    }
    for (slot, lease) in leased_serial_io() {
        let Ok(mut ctrl) = init_controller(slot, lease, 400, None) else { continue };
        let Some((addr, reg)) = scan_bus_readonly(&mut ctrl, None) else { continue };
        serial_println!("[touchpad.fallback.hit] pci={:02x}:{:02x}.{} addr={:#x} reg={:#x}", lease.bus, lease.dev, lease.func, addr, reg);
        let mut last_error = None;
        let mut bus = OnBus { c: &mut ctrl, address: addr, last_error: &mut last_error };
        if let Ok(tp) = Touchpad::probe(&mut bus, reg, unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) }) {
            let pad = finish_pad(ctrl, lease, addr, reg, tp, None);
            unsafe { *core::ptr::addr_of_mut!(PAD) = Some(pad); }
            return;
        }
    }
}

/// Empties the pad (its reset answer and anything since), then binds and unmasks its line.
fn arm(p: &mut Pad, irq: sex_pdx::touchpad::Irq) {
    use sex_pdx::device as d;
    let mut bus = OnBus { c: &mut p.ctrl, address: p.address, last_error: &mut p.last_error };
    let mut emptied = false;
    for _ in 0..DRAIN_READS { if p.tp.poll_report(&mut bus).is_ok() && p.tp.last_empty { emptied = true; break; } }
    let bound = d::line_bind(0, irq.gsi, irq.edge, irq.active_low).and_then(|bit| d::line_mask(0, false).map(|()| bit));
    match bound {
        Ok(bit) => {
            p.line_bit = Some(bit);
            p.pace = Pace::line(irq.edge, sex_pdx::get_ticks());
            serial_println!("[sexinput.touchpad.mode] mode=interrupt gsi={} edge={} active_low={} bit={} emptied={}", irq.gsi, irq.edge as u8, irq.active_low as u8, bit, emptied as u8);
            serial_println!("[touchpad.mode] mode=irq gsi={} edge={} active_low={}", irq.gsi, irq.edge as u8, irq.active_low as u8);
        }
        Err(e) => {
            serial_println!("[sexinput.touchpad.mode] mode=poll reason=line_refused gsi={} status={:#x}", irq.gsi, e);
            serial_println!("[touchpad.mode] mode=poll reason=line_refused gsi={}", irq.gsi);
        }
    }
}

/// Takes the line's pending bit, so an interrupt that arrives while Input cannot read
/// (its outboxes are full) does not keep waking it; the pad is read on the next `poll`.
pub fn take_interrupt() {
    let Some(p) = (unsafe { (*core::ptr::addr_of_mut!(PAD)).as_mut() }) else { return };
    let Some(bit) = p.line_bit else { return };
    if sex_pdx::device::irq_take() & 1 << bit != 0 { p.fired = true; }
}

/// The ACPI service's word: bring that touchpad up (once).
pub fn found(words: [u64; 3]) {
    let Some(f) = Found::decode(words) else { serial_println!("[sexinput.touchpad.found] valid=0"); return };
    if unsafe { (*core::ptr::addr_of!(PAD)).is_some() } { return; }
    serial_println!("[sexinput.touchpad.found] pci={:02x}:{:02x}.{} address={:#x} khz={} hid_register={:#x}", f.bus, f.device, f.function, f.address, f.khz, f.hid_register);
    match open(&f) {
        Ok(p) => unsafe { *core::ptr::addr_of_mut!(PAD) = Some(p) },
        Err(why) => serial_println!("[sexinput.touchpad.absent] reason={}", why),
    }
}

/// What a gesture asks for, in the words the rest of Input already speaks.
pub enum Out {
    /// (arg0, arg1, class) pointer words.
    Pointer([(u64, u64, u64); 1]),
    /// A USB HID keyboard snapshot (usages, modifiers): a chord, then its release.
    Chord(u8, u8),
}

fn translate(e: Event) -> Option<Out> {
    use sex_pdx::{EV_BTN, EV_REL, EV_WHEEL};
    Some(match e {
        Event::Move(dx, dy) => Out::Pointer([(dx as i64 as u64, dy as i64 as u64, EV_REL)]),
        Event::Button(n, down) => Out::Pointer([(n as u64, down as u64, EV_BTN)]),
        Event::Scroll(steps) => Out::Pointer([(sex_pdx::app_scroll::Scroll::new(steps)?.wire(), 0, EV_WHEEL)]),
        // Window switching: Alt+Tab.
        Event::SwipeLeft | Event::SwipeRight => Out::Chord(0x2B, 0x04),
        // Zoom the focused app: Ctrl+= and Ctrl+-.
        Event::ZoomIn => Out::Chord(0x2E, 0x01),
        Event::ZoomOut => Out::Chord(0x2D, 0x01),
        Event::SwipeUp | Event::SwipeDown => return None,
    })
}

/// Suspend to idle (the kernel's `OP_DEVICE_SLEEP`): the pad sleeps, its line is masked
/// and it is not read until the machine wakes; waking costs one `SET_POWER` and a read.
pub fn device_sleep(asleep: bool, now: u64) {
    let Some(p) = (unsafe { (*core::ptr::addr_of_mut!(PAD)).as_mut() }) else { return };
    if p.asleep == asleep { return; }
    if p.line_bit.is_some() { let _ = sex_pdx::device::line_mask(0, asleep); }
    let mut bus = OnBus { c: &mut p.ctrl, address: p.address, last_error: &mut p.last_error };
    let ok = if asleep { p.tp.sleep(&mut bus) } else { p.tp.wake(&mut bus) };
    p.asleep = asleep;
    // Asleep: no read is due, so Input waits its whole idle bound. Awake: read once at once.
    p.next_poll = if asleep { u64::MAX } else { now };
    p.again = !asleep;
    serial_println!("[sexinput.touchpad.power] asleep={} ok={}", asleep as u8, ok as u8);
}

/// Reads the pad when its line fired or a timed read is due. Calls `out` for each
/// resulting event.
pub fn poll(now: u64, mut out: impl FnMut(Out)) {
    take_interrupt();
    let Some(p) = (unsafe { (*core::ptr::addr_of_mut!(PAD)).as_mut() }) else { return };
    if p.asleep { p.fired = false; return; }
    let fired = core::mem::take(&mut p.fired);
    p.next_poll = p.pace.next_read();
    if !core::mem::take(&mut p.again) && !p.pace.due(now, fired) { return; }
    {
        let repeats = p.tp.repeats;
        let mut bus = OnBus { c: &mut p.ctrl, address: p.address, last_error: &mut p.last_error };
        let read = match p.tp.poll_report(&mut bus) {
            Ok(Some(Report::Touch(frame))) => {
                p.failures = 0;
                if !p.first_report_logged {
                    p.first_report_logged = true;
                    serial_println!("[touchpad.first_report] kind=ptp fingers={} button={}", frame.fingers(), frame.button as u8);
                }
                p.rec.feed(&frame, now, |e| {
                    if !matches!(e, Event::Move(..)) { sex_pdx::serial_record!("[sexinput.touchpad.event] {:?}", e); }
                    if let Some(o) = translate(e) { out(o); }
                });
                Read::Report { touch: frame.fingers() > 0 || frame.button }
            }
            Ok(Some(Report::Mouse(m))) => {
                use sex_pdx::{EV_BTN, EV_REL, EV_WHEEL};
                p.failures = 0;
                if !p.first_report_logged {
                    p.first_report_logged = true;
                    serial_println!("[touchpad.first_report] kind=mouse dx={} dy={} buttons={:#x}", m.dx, m.dy, m.buttons);
                }
                if m.dx != 0 || m.dy != 0 {
                    out(Out::Pointer([(m.dx as i64 as u64, m.dy as i64 as u64, EV_REL)]));
                }
                let diff = m.buttons ^ p.mouse_buttons;
                for b in 0..3u8 {
                    if diff & (1 << b) != 0 {
                        out(Out::Pointer([((b + 1) as u64, ((m.buttons >> b) & 1) as u64, EV_BTN)]));
                    }
                }
                p.mouse_buttons = m.buttons;
                if m.wheel != 0 {
                    if let Some(s) = sex_pdx::app_scroll::Scroll::new(m.wheel.clamp(-127, 127) as i8) {
                        out(Out::Pointer([(s.wire(), 0, EV_WHEEL)]));
                    }
                }
                Read::Report { touch: m.dx != 0 || m.dy != 0 || m.wheel != 0 || m.buttons != 0 }
            }
            // A pad that repeats its last report when idle has nothing new: as good as empty,
            // so idle check reads never count against the line.
            Ok(None) => { p.failures = 0; if p.tp.last_empty || p.tp.repeats != repeats { Read::Empty } else { Read::Report { touch: false } } }
            Err(()) => {
                p.failures += 1;
                if p.failures == 1 || p.failures == GIVE_UP_AFTER {
                    serial_println!("[sexinput.touchpad.read.fail] count={} bus={:?}", p.failures, p.last_error);
                }
                if p.failures >= GIVE_UP_AFTER {
                    serial_println!("[sexinput.touchpad.stopped] reports={} interrupts={}", p.tp.reports, p.pace.interrupts);
                    if p.line_bit.is_some() { let _ = sex_pdx::device::line_mask(0, true); }
                    unsafe { *core::ptr::addr_of_mut!(PAD) = None; }
                    return;
                }
                Read::Failed
            }
        };
        match p.pace.after(now, read) {
            Next::Again => { p.again = true; p.next_poll = now; return; }
            Next::Unmask => { let _ = sex_pdx::device::line_mask(0, false); }
            Next::Fallback(why) => {
                let _ = sex_pdx::device::line_mask(0, true);
                p.line_bit = None;
                serial_println!("[sexinput.touchpad.mode] mode=poll reason={} interrupts={} reports={}", why, p.pace.interrupts, p.tp.reports);
                serial_println!("[touchpad.mode] mode=poll reason={}", why);
            }
            Next::Done => {}
        }
    }
    p.next_poll = p.pace.next_read();
}

/// How long Input may sleep: `bound`, or until the touchpad's next timed read (a poll,
/// or the line's check read). The line's interrupt wakes Input by itself.
pub fn wait_bound(now: u64, bound: u64) -> u64 {
    match unsafe { (*core::ptr::addr_of!(PAD)).as_ref() } {
        Some(p) => p.next_poll.saturating_sub(now).clamp(1, bound.max(1)),
        None => bound,
    }
}
