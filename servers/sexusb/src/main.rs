#![no_std]
#![no_main]

//! SexUSB: the USB service. It drives the xHCI controller (`xhci.rs`), enumerates every
//! device on root ports and behind hubs (`usb.rs`), and turns keyboards, mice and tablets
//! into reports for Input. Report packing and the ordered outbox to Input live here.

mod hid;
mod uac;
mod usb;
mod xhci;

use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::{serial_record as serial_println, sys_yield, SLOT_USB_HOST, pdx_call_checked};


#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { sys_yield(); }
}

fn map_xhci_bar0(map_size: u64) -> u64 {
    let map_va: u64;
    unsafe {
        // syscall 43 = MAP_PCI_BAR(cap_slot, bar_index, map_size)
        core::arch::asm!(
            "syscall",
            in("rax") 43u64,
            in("rdi") SLOT_USB_HOST,
            in("rsi") 0u64,
            in("rdx") map_size,
            lateout("rax") map_va,
            out("rcx") _,
            out("r11") _,
        );
    }
    map_va
}

#[inline(always)]
fn mmio_read32(base: u64, offset: u64) -> u32 {
    let ptr = (base + offset) as *const u32;
    unsafe { core::ptr::read_volatile(ptr) }
}

#[inline(always)]
fn mmio_write32(base: u64, offset: u64, value: u32) {
    let ptr = (base + offset) as *mut u32;
    unsafe { core::ptr::write_volatile(ptr, value); }
}

#[inline(always)]
fn trb_write_volatile(base_va: u64, index: u64, d0: u32, d1: u32, d2: u32, d3: u32) {
    let p = (base_va + index * 16) as *mut u32;
    unsafe {
        core::ptr::write_volatile(p.add(0), d0);
        core::ptr::write_volatile(p.add(1), d1);
        core::ptr::write_volatile(p.add(2), d2);
        core::ptr::write_volatile(p.add(3), d3);
    }
}

#[inline(always)]
fn trb_read_dword(base_va: u64, index: u64, word: usize) -> u32 {
    let p = (base_va + index * 16) as *const u32;
    unsafe { core::ptr::read_volatile(p.add(word)) }
}

struct BootMouseReport {
    buttons: u8,
    dx: i8,
    dy: i8,
    wheel: i8,
}

#[allow(dead_code)]
fn decode_boot_mouse_report(buf: &[u8], len: usize) -> Option<BootMouseReport> {
    if len < 3 || len > buf.len() {
        return None;
    }
    let wheel = if len >= 4 { buf[3] as i8 } else { 0 };
    Some(BootMouseReport {
        buttons: buf[0],
        dx: buf[1] as i8,
        dy: buf[2] as i8,
        wheel,
    })
}

struct TabletReport {
    buttons: u8,
    abs_x: u16,
    abs_y: u16,
    wheel: i8,
}

/// Decode QEMU usb-tablet HID report (5 bytes, optional sixth wheel byte):
///   byte 0: buttons[2:0] (bit0=left, bit1=right, bit2=middle), padding[7:3]
///   byte 1-2: X absolute (little-endian u16, 0..32767)
///   byte 3-4: Y absolute (little-endian u16, 0..32767)
///   byte 5, when received: signed relative wheel (positive = up)
#[allow(dead_code)]
fn decode_tablet_report(buf: &[u8], len: usize) -> Option<TabletReport> {
    if len < 5 || len > buf.len() {
        return None;
    }
    let abs_x = (buf[1] as u16) | ((buf[2] as u16) << 8);
    let abs_y = (buf[3] as u16) | ((buf[4] as u16) << 8);
    Some(TabletReport {
        buttons: buf[0] & 0x07,
        abs_x,
        abs_y,
        wheel: if len >= 6 { buf[5] as i8 } else { 0 },
    })
}

#[allow(dead_code)]
fn pack_tablet_axes(report: &TabletReport) -> u64 {
    (report.abs_x as u64) | ((report.abs_y as u64) << 16)
        | (1u64 << 32) | ((report.wheel as u8 as u64) << 40)
}

#[allow(dead_code)]
fn pack_mouse_axes(report: &BootMouseReport) -> u64 {
    (report.dx as u8 as u64) | ((report.dy as u8 as u64) << 8)
        | ((report.wheel as u8 as u64) << 16)
}

// Nonbudgeted receipt of a decoded hardware wheel report, before IPC admission.
fn record_device_wheel(slot: u32, bytes: u32, absolute: bool, wheel: i8, packed: u64) {
    if wheel != 0 {
        sex_pdx::serial_record!("[sexusb.wheel.device] slot={} bytes={} absolute={} delta={} packed={} up_positive=1",
            slot, bytes, absolute as u8, wheel, packed);
    }
}

const SLOT_USB_SEXINPUT: u64 = 9;
const OP_USB_MOUSE_REPORT: u64 = 0x260;
const OP_USB_KEYBOARD_REPORT: u64 = 0x261;
static CAP_READY_SEXINPUT: AtomicBool = AtomicBool::new(false);
static DEFER_EMITTED_SEXINPUT: AtomicBool = AtomicBool::new(false);
static EDGE_SEND_EMITTED_SEXINPUT: AtomicBool = AtomicBool::new(false);
static ROUTE_READY_EMITTED_SEXINPUT: AtomicBool = AtomicBool::new(false);
static ROUTE_MISSING_EMITTED_SEXINPUT: AtomicBool = AtomicBool::new(false);
static BOOTGRAPH_PROOF_MARKER_EMITTED: AtomicBool = AtomicBool::new(false);
// Gate per-iteration ring-advance and idle-poll serial logs.
// One main-task owner, separate reserved raw keyboard and pointer capacities.
// Both preserve arrival order; hardware polling/rearming never waits on Full.
struct UsbPointerOutbox {
    words: [(u64,u64,u64);32], orders:[u64;32], head: usize, len: usize, stalled: u32,
}
impl UsbPointerOutbox {
    const STALL_LIMIT: u32 = 4096;
    const fn new() -> Self { Self { words:[(0,0,0);32],orders:[0;32],head:0,len:0,stalled:0 } }
    fn push(&mut self, packet:(u64,u64,u64)) -> bool {
        if self.len == self.words.len() { return false; }
        self.words[(self.head+self.len)%self.words.len()] = packet;
        self.len+=1; true
    }
    fn pop(&mut self) {
        self.words[self.head]=(0,0,0);self.orders[self.head]=0;
        self.head=(self.head+1)%self.words.len();self.len-=1;self.stalled=0;
    }
    fn poll(&mut self,send:impl FnMut(u64,u64,u64)->Result<bool,u64>) {
        self.poll_budget(OP_USB_MOUSE_REPORT,&mut 4,true,send);
    }
    fn poll_budget(&mut self,op:u64,budget:&mut usize,allow_expiry:bool,mut send:impl FnMut(u64,u64,u64)->Result<bool,u64>)->bool {
        while *budget>0 {
            if self.len==0 { break; }
            let (a,b,c)=self.words[self.head];*budget-=1;
            match send(a,b,c) {
                Ok(true) => {if op==OP_USB_MOUSE_REPORT {record_wheel_admission(c);} self.pop();}
                Ok(false) | Err(sex_pdx::ERR_SERVICE_NOT_READY) => {
                    self.stalled=self.stalled.saturating_add(1);
                    // Keyboard snapshots retain custody on Full; expiring a
                    // key-up could leave an admitted modifier held forever.
                    if op==OP_USB_KEYBOARD_REPORT || !allow_expiry || self.stalled < Self::STALL_LIMIT {return true;}
                    sex_pdx::serial_record!("[sexusb.pointer.rejected] packed={} reason=stall_limit admitted=0",c);
                    self.pop();return true;
                }
                Err(error) => {
                    sex_pdx::serial_record!("[sexusb.input.rejected] op={} arg0={} arg1={} arg2={} reason=transport error={} admitted=0",op,a,b,c,error);
                    self.pop();return true;
                }
            }
        }
        self.len>0
    }
}
static mut POINTER_OUTBOX: UsbPointerOutbox = UsbPointerOutbox::new();
static mut KEYBOARD_OUTBOX: UsbPointerOutbox = UsbPointerOutbox::new();
static mut INPUT_ARRIVAL: u64 = 0;

fn record_wheel_admission(packed:u64) {
    let absolute=packed & (1u64<<32)!=0;
    let wheel=(packed >> if absolute {40}else{16}) as u8 as i8;
    if wheel!=0 {
        sex_pdx::serial_record!("[sexusb.wheel.admitted] absolute={} delta={} packed={} admitted=1",absolute as u8,wheel,packed);
    }
}

fn poll_pointer_reports() {
    // Hardware input retains one arrival order across reserved class capacities.
    // Raw reports are indivisible IPC packets; each is admitted exactly once.
    unsafe {
        let keys=&mut *core::ptr::addr_of_mut!(KEYBOARD_OUTBOX);
        let pointer=&mut *core::ptr::addr_of_mut!(POINTER_OUTBOX);
        for _ in 0..4 {
            if keys.len==0 && pointer.len==0 {break;}
            let key_first=keys.len>0 && (pointer.len==0 || keys.orders[keys.head]<pointer.orders[pointer.head]);
            let allow_expiry=keys.len==0;
            let (outbox,op)=if key_first {( &mut *keys,OP_USB_KEYBOARD_REPORT)}else{(&mut *pointer,OP_USB_MOUSE_REPORT)};
            let mut blocked=false;
            let mut hard_failure=None;
            outbox.poll_budget(op,&mut 1,allow_expiry,|a,b,c|{
                let result=send_report_once(op,a,b,c);blocked=!matches!(result,Ok(true));
                if let Err(error)=result {if error!=sex_pdx::ERR_SERVICE_NOT_READY {hard_failure=Some(error);}}
                result
            });
            if let Some(error)=hard_failure {
                sex_pdx::serial_record!("[sexusb.input.uncertain] error={} pointer={} keys={} dependent_input=erased",error,pointer.len,keys.len);
                keys.words.fill((0,0,0));keys.orders.fill(0);keys.head=0;keys.len=0;keys.stalled=0;
                pointer.words.fill((0,0,0));pointer.orders.fill(0);pointer.head=0;pointer.len=0;pointer.stalled=0;
            }
            if blocked {break;}
        }
    }
}

fn send_report_to_sexinput(op:u64,arg0:u64,arg1:u64,arg2:u64)->Result<bool,u64> {
    if op!=OP_USB_MOUSE_REPORT && op!=OP_USB_KEYBOARD_REPORT {
        return send_report_once(op,arg0,arg1,arg2);
    }
    if unsafe{*core::ptr::addr_of!(INPUT_ARRIVAL)==u64::MAX} {
        sex_pdx::serial_record!("[sexusb.input.rejected] op={} reason=sequence_exhausted admitted=0",op);
        return Err(sex_pdx::ERR_SERVICE_NOT_READY);
    }
    let keyboard=op==OP_USB_KEYBOARD_REPORT;
    let pending=unsafe{(*core::ptr::addr_of!(KEYBOARD_OUTBOX)).len>0
        || (*core::ptr::addr_of!(POINTER_OUTBOX)).len>0};
    let outbox=unsafe { &mut *if keyboard {core::ptr::addr_of_mut!(KEYBOARD_OUTBOX)}
        else{core::ptr::addr_of_mut!(POINTER_OUTBOX)} };
    if !pending {
        match send_report_once(op,arg0,arg1,arg2) {
            Ok(true) => {if !keyboard{record_wheel_admission(arg2);}return Ok(true);}
            Ok(false)|Err(sex_pdx::ERR_SERVICE_NOT_READY)=>{}
            Err(error)=>{
                sex_pdx::serial_record!("[sexusb.input.rejected] op={} arg0={} arg1={} arg2={} reason=transport error={} admitted=0",op,arg0,arg1,arg2,error);
                return Err(error);
            }
        }
    }
    let next=unsafe{(*core::ptr::addr_of!(INPUT_ARRIVAL)).checked_add(1)};
    let slot=(outbox.head+outbox.len)%outbox.words.len();
    if let Some(order)=next {
        if outbox.push((arg0,arg1,arg2)) {
            outbox.orders[slot]=order;unsafe{INPUT_ARRIVAL=order;}return Ok(false);
        }
    }
    {
        sex_pdx::serial_record!("[sexusb.input.rejected] op={} arg0={} arg1={} arg2={} reason=outbox_full_or_sequence admitted=0",op,arg0,arg1,arg2);
        Err(sex_pdx::ERR_SERVICE_NOT_READY)
    }
}

fn send_report_once(op: u64, arg0: u64, arg1: u64, arg2: u64) -> Result<bool, u64> {
    if CAP_READY_SEXINPUT.load(Ordering::Relaxed) {
        return pdx_call_checked(SLOT_USB_SEXINPUT, op, arg0, arg1, arg2).map(|_| {
            if !ROUTE_READY_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[sexusb.route.sexinput.ready] slot={} ok=1", SLOT_USB_SEXINPUT);
            }
            if !BOOTGRAPH_PROOF_MARKER_EMITTED.swap(true, Ordering::Relaxed) {
                serial_println!("[sexusb.bootgraph.proof_report]");
            }
            if !EDGE_SEND_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.send from=sexusb to=sexinput slot=9 op=HID_REPORT first=1]");
            }
            true
        });
    }

    match pdx_call_checked(SLOT_USB_SEXINPUT, op, arg0, arg1, arg2) {
        Ok(_) => {
            CAP_READY_SEXINPUT.store(true, Ordering::Relaxed);
            if !ROUTE_READY_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[sexusb.route.sexinput.ready] slot={} ok=1", SLOT_USB_SEXINPUT);
            }
            if !BOOTGRAPH_PROOF_MARKER_EMITTED.swap(true, Ordering::Relaxed) {
                serial_println!("[sexusb.bootgraph.proof_report]");
            }
            if !EDGE_SEND_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.send from=sexusb to=sexinput slot=9 op=HID_REPORT first=1]");
            }
            Ok(true)
        }
        Err(e) if e == sex_pdx::ERR_CAP_INVALID => {
            if !ROUTE_MISSING_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[sexusb.route.sexinput.missing] slot={} ok=0", SLOT_USB_SEXINPUT);
            }
            if !DEFER_EMITTED_SEXINPUT.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.defer from=sexusb to=sexinput slot=9 reason=missing_cap]");
            }
            sys_yield();
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[sexusb.init.start]");
    let limit = sex_pdx::memory_region_info(SLOT_USB_HOST).map(|info| info.length).unwrap_or(0).min(0x10000);
    let mut cap = u64::MAX;
    for _ in 0..3 {
        cap = if limit >= 0x20 { map_xhci_bar0(limit) } else { u64::MAX };
        if cap != 0 && cap != u64::MAX { break; }
        sys_yield();
    }
    if cap == 0 || cap == u64::MAX {
        serial_println!("[sexusb.xhci.map.bad] reason=bar_zero_after_retry attempts=3 ok=0");
        serial_println!("[usb.host.controller.none] map_va={:#x}", cap);
        park();
    }
    serial_println!("[usb.host.controller.found] slot={} bar=0 map_va={:#x}", SLOT_USB_HOST, cap);
    serial_println!("[sexusb.xhci.map.ok]");
    // A zero-byte reservation is refused, but it builds the pool from the DMA region.
    let mut pool = None;
    let _ = reserve_dma(&mut pool, 0);
    let Some(pool) = pool else { serial_println!("[sexusb.memory.dma.bad]"); park(); };
    let controller = match xhci::Xhci::start(cap, limit, pool) {
        Ok(x) => x,
        Err(e) => { serial_println!("[sexusb.xhci.start.fail] reason={:?}", e); park(); }
    };
    serial_println!("[sexusb.ready]");
    // The first report to Input is the first real one: a report sent here could reach
    // Input before it is ready (the boot order check counts that as a failure).
    let mut usb = usb::Usb::new(controller);
    // USB network adapters: frames to and from SexNet through this region (sex_pdx::usb_net).
    let net_view = sex_pdx::memory_region_full_view(sex_pdx::usb_net::SLOT, sex_pdx::memory_region::QUERY_VIEW | sex_pdx::memory_region::READ | sex_pdx::memory_region::WRITE).ok()
        .filter(|v| v.length >= sex_pdx::usb_net::BYTES && v.virtual_address != 0);
    if let Some(v) = net_view {
        let shared = unsafe { sex_pdx::usb_net::Shared::at(v.virtual_address as *mut u8) };
        shared.set_link(None);
        usb.net_region = v.virtual_address;
    }
    serial_println!("[sexusb.ether.route] region={}", net_view.is_some() as u8);
    // USB sound cards: Bell's frames through this region (sex_pdx::usb_audio).
    if let Ok(v) = sex_pdx::memory_region_full_view(sex_pdx::usb_audio::SLOT, sex_pdx::memory_region::QUERY_VIEW | sex_pdx::memory_region::READ | sex_pdx::memory_region::WRITE) {
        if v.length >= sex_pdx::usb_audio::BYTES && v.virtual_address != 0 {
            unsafe { sex_pdx::usb_audio::Shared::at(v.virtual_address as *mut u8) }.set_card(None);
            usb.audio_region = v.virtual_address;
        }
    }
    serial_println!("[sexusb.audio.route] region={}", (usb.audio_region != 0) as u8);
    let ports = usb.x.ports;
    let (connected, first) = usb.scan();
    if connected == 0 {
        serial_println!("[sexusb.discovery.summary] ports={} connected=0 first=none ok=1", ports);
    } else {
        serial_println!("[sexusb.discovery.summary] ports={} connected={} first={} ok=1", ports, connected, first);
        // Devices behind hubs found at start count too.
        for _ in 0..8 {
            while let Some(e) = usb.x.event() { usb.event(e); }
            usb.service();
        }
        let s = usb.summary();
        serial_println!("[sexusb.enum.summary] ports_connected={} slots_enabled={} hid_devices={} pointer_devices={} keyboards={} hubs={} failed={}",
            connected, s.slots, s.hid, s.pointers, s.keyboards, s.hubs, s.failed);
    }
    // Files' disk requests arrive in this region and inbox (sex_pdx::usb_disk).
    use sex_pdx::memory_region::{QUERY_VIEW as Q, READ as R, WRITE as W};
    let files = sex_pdx::memory_region_info(sex_pdx::usb_disk::BUFFER_SLOT).ok().and_then(|info| info.peer_pd).unwrap_or(0);
    let region = sex_pdx::memory_region_full_view(sex_pdx::usb_disk::BUFFER_SLOT, Q | R | W).map(|v| v.virtual_address).unwrap_or(0);
    serial_println!("[sexusb.disks.route] files={} region={} ok={}", files, (region != 0) as u8, (files != 0 && region != 0) as u8);
    loop {
        let mut busy = false;
        for _ in 0..64 {
            let Some(e) = usb.x.event() else { break };
            usb.event(e);
            busy = true;
        }
        usb.service();
        busy |= usb.ether_pump();
        busy |= usb.audio_pump();
        if usb.changed && files != 0 {
            usb.changed = false;
            let (status, _) = sex_pdx::pdx_call(sex_pdx::usb_disk::SLOT, sex_pdx::usb_disk::OP_CHANGED, usb.generation, 0, 0);
            serial_println!("[sexusb.disks.changed] generation={} sent={}", usb.generation, (status == 0) as u8);
            if status != 0 { usb.changed = true; }
        }
        while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
            if m.type_id <= 1 { continue; }
            busy = true;
            if m.caller_pd != files || region == 0 {
                serial_println!("[sexusb.disks.refused] caller={} op={:#x}", m.caller_pd, m.type_id);
                let _ = sex_pdx::pdx_reply(m.caller_pd, sex_pdx::usb_disk::INVALID);
                continue;
            }
            let value = usb.disk_request(m.type_id, m.arg0, m.arg1, m.arg2, region);
            let _ = sex_pdx::pdx_reply(m.caller_pd, value);
        }
        poll_pointer_reports();
        if !busy { let _ = sex_pdx::sys_wait_self_ticks(1); }
    }
}

fn reserve_dma(pool: &mut Option<sex_pdx::dma_pool::DmaRegionPool>, bytes: u64) -> (u64, u64) {
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE, DMA_ADDRESS};
    if pool.is_none() {
        let Ok(view) = sex_pdx::memory_region_full_view(
            sex_pdx::SLOT_DMA_REGION, QUERY_VIEW | READ | WRITE | DMA_ADDRESS,
        ) else { return (u64::MAX, u64::MAX); };
        let Ok(new_pool) = sex_pdx::dma_pool::DmaRegionPool::new(view)
            else { return (u64::MAX, u64::MAX); };
        sex_pdx::serial_record!("[sexusb.memory.dma.ready] region_id={} length={}", view.region_id, view.length);
        *pool = Some(new_pool);
    }
    match pool.as_mut().unwrap().reserve(bytes) {
        Ok(buffer) => (buffer.physical_address, buffer.virtual_address),
        Err(_) => (u64::MAX, u64::MAX),
    }
}

fn park() -> ! { loop { let _ = sex_pdx::sys_wait_self_ticks(100); } }
