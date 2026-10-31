#![no_std]
#![no_main]

use sex_pdx::*;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop { sys_yield(); }
}

// ── Pointer state for HID report normalizer ──
static mut LAST_BUTTONS: u8 = 0;
// Tracks the USB HID keycode from the previous keyboard report for press/release detection.
static mut LAST_USB_KEY: u8 = 0;
static mut LAST_USB_MODIFIERS: u8 = 0;
const OP_HID_EVENT: u64 = 0x202;
const OP_USB_MOUSE_REPORT: u64 = 0x260;
const OP_USB_KEYBOARD_REPORT: u64 = 0x261;
static CAP_READY_SHELL: AtomicBool = AtomicBool::new(false);
static DEFER_EMITTED_SHELL: AtomicBool = AtomicBool::new(false);
static EDGE_SEND_EMITTED_SHELL: AtomicBool = AtomicBool::new(false);
// Static storage avoids placing a second message ring on the 64 KiB task stack.
// Only the startup/main loop owns this FIFO of pre-ready USB input.
static mut STARTUP_INBOX: sex_pdx::shell_ready::StartupInbox<
    { sex_pdx::shell_ready::STARTUP_INBOX_CAPACITY }
> = sex_pdx::shell_ready::StartupInbox::new();

fn pop_startup_message() -> Option<PdxMessage> {
    unsafe { (*core::ptr::addr_of_mut!(STARTUP_INBOX)).pop() }
}

static mut REAL_USB_POINTER_SEEN: bool = false;
/// Master switch for all synthetic input proofs (drag, click-focus, silkbar clicks).
///
/// Set env var `SEXOS_PROOFS_DISABLED=1` at build time to disable all proofs
/// for interactive visual use. Default (unset): proofs enabled for CI/nographic
/// verification. Build once per mode; the env var is embedded at compile time.
///
/// Equivalent to the previous three individual `USB_PROOF_DISABLE_*` constants,
/// merged into one gate to simplify toggling.
///
/// Normal desktop images never inject input. Synthetic input is available only
/// when the sealed build_sexinput stage explicitly declares
/// `rustflags = "--cfg sexos_synthetic_input_proof"` in the build specification.
/// The legacy disable variable can further suppress a declared proof profile.
const SYNTHETIC_INPUT_PROOFS_DISABLED: bool = !cfg!(sexos_synthetic_input_proof)
    || option_env!("SEXOS_PROOFS_DISABLED").is_some();
/// Enables synthetic SilkBar click proof (launcher/status/clock/bell click script).
/// Default (unset): disabled to avoid recurring panel toggle behavior during
/// interactive use.
/// Set env var `SEXOS_SILKBAR_CLICK_PROOF=1` at build time to re-enable.
const SILKBAR_CLICK_PROOF_ENABLED: bool = option_env!("SEXOS_SILKBAR_CLICK_PROOF").is_some();
/// Dev-only keyboard cursor fallback. When enabled, arrow keys and WASD
/// emit EV_REL events to move the cursor, bypassing broken QEMU USB HID input.
/// Set env var `SEXOS_KEYBOARD_CURSOR=1` at build time to enable.
/// Gate unset = no behavior change.
const KEYBOARD_CURSOR_ENABLED: bool = option_env!("SEXOS_KEYBOARD_CURSOR").is_some();
/// Enables one-shot synthetic keyboard proof for F5/F6 HID event path.
/// Set env var `SEXOS_KEYBOARD_PROOF=1` at build time to enable.
/// Default (unset): no behavior change.
/// Only affects sexinput; no kernel changes.
const KEYBOARD_PROOF_ENABLED: bool = option_env!("SEXOS_KEYBOARD_PROOF").is_some();
/// Autonomous cursor demo gate. Drives silk-shell cursor demo via OP_AUTODEMO_TICK.
/// Gate: SEXOS_CURSOR_AUTODEMO=1 at build time.
/// Decoupled from SEXOS_KEYBOARD_CURSOR: keyboard cursor fallback no longer implies autodemo.
const CURSOR_AUTODEMO_ENABLED: bool = option_env!("SEXOS_CURSOR_AUTODEMO").is_some();
const OP_AUTODEMO_TICK: u64 = 0x2A0;
// One-shot gate: set true after synthetic drag proof stages 0→1→2 complete.
// Prevents the drag proof from wrapping and replaying endlessly every 120 ticks.
// Also set true when a real USB mouse input arrives, cancelling remaining proofs.
static mut SYNTHETIC_DRAG_PROOF_DONE: bool = false;
static mut SYNTHETIC_SILKBAR_CLICK_PROOF_DONE: bool = false;

#[derive(Copy, Clone)]
struct HidPointerRawReport {
    dx: i16,
    dy: i16,
    buttons: u8,
    wheel: i8,
    is_abs: bool,
}

// BOOT fixed spawn order gives SexUSB its sole main task (PD5/task5).
// No successor or runtime task creation is sanctioned for this input producer.
fn usb_report_sender(req: &PdxMessage) -> bool {
    req.caller_pd == 5 && req.caller_task == 5
}

fn decode_pointer_payload(buttons: u64, packed: u64) -> Option<HidPointerRawReport> {
    if buttons > 7 { return None; }
    let is_abs = packed & (1 << 32) != 0;
    let allowed = if is_abs { 0x0000_ff01_ffff_ffff } else { 0x00ff_ffff };
    if packed & !allowed != 0 { return None; }
    let dx = if is_abs { (packed & 0xffff) as u16 as i16 }
        else { packed as u8 as i8 as i16 };
    let dy = if is_abs { ((packed >> 16) & 0xffff) as u16 as i16 }
        else { (packed >> 8) as u8 as i8 as i16 };
    if is_abs && (dx < 0 || dy < 0) { return None; }
    let wheel = (packed >> if is_abs { 40 } else { 16 }) as u8 as i8;
    Some(HidPointerRawReport { dx, dy, buttons: buttons as u8, wheel, is_abs })
}

/// Parse a boot-mouse-style 3-byte report, detect button edge transitions,
/// and emit normalized position/button/wheel events via the callback.
///
/// Transport-agnostic: callable from synthetic producer or future USB HID source.
/// Returns at most 5 events: one position, three button edges, then one wheel.
fn normalize_pointer_report_v1(
    report: HidPointerRawReport,
    last_buttons: &mut u8,
    mut emit: impl FnMut(u64, u64, u64),
) -> usize {
    let mut count = 0;
    let buttons = report.buttons & 0x07;       // mask to valid button bits
    let changed = buttons ^ *last_buttons; // XOR edge detection

    // Keep wire encoding byte-for-byte: shell decodes as msg.argN as i32.
    let dx = report.dx as i32;
    let dy = report.dy as i32;

    let mut reason = "none";

    // Emit ABS/REL movement FIRST so cursor position is current
    // before any button click is processed at silk-shell.
    if report.is_abs {
        // Preserve absolute tablet coordinates; shell owns current BOOT geometry.
        let x=dx.clamp(0,32767)as u64;let y=dy.clamp(0,32767)as u64;
        emit(x,y,EV_ABS);count+=1;
        sex_pdx::serial_record!("[sexinput.tablet.absolute] x={} y={} normalized_in_shell=1",x,y);
    } else {
        if dx != 0 || dy != 0 {
            if reason == "none" { reason = "motion"; }
        }
        if reason != "none" {
            unsafe {
                static mut POINTER_FORWARD_BUDGET: u32 = 2048;
                if POINTER_FORWARD_BUDGET > 0 {
                    POINTER_FORWARD_BUDGET -= 1;
                    serial_println!("[sexinput.pointer.forward.reason={}]", reason);
                }
            }
            if dx != 0 || dy != 0 {
                emit(dx as u64, dy as u64, EV_REL);
                count += 1;
            }
        }
    }

    // Emit BTN events AFTER ABS/REL so cursor position is current
    // before silk-shell processes the button click.
    if changed != 0 {
        serial_println!(
            "[usb.hid.pointer.button] buttons={} left={} ok=1",
            buttons,
            (buttons & 0x01 != 0) as u8
        );
    }
    let mut bit = 1u8;
    let mut btn_id = 1u8;
    while bit <= 0x04 {
        if changed & bit != 0 {
            let pressed = buttons & bit != 0;
            emit(btn_id as u64, if pressed { 1 } else { 0 }, EV_BTN);
            count += 1;
        }
        bit <<= 1;
        btn_id += 1;
    }
    *last_buttons = buttons;

    // Wheel follows position and all button edges; positive device steps mean up.
    if let Some(scroll) = sex_pdx::app_scroll::Scroll::new(report.wheel) {
        emit(scroll.wire(), 0, EV_WHEEL);
        count += 1;
    }

    count
}

// One main-loop owner; pointer batches and reserved keys share arrival order.
// Each cursor advances only on admission; Full retains the exact unsent suffix.
#[derive(Clone,Copy)]
struct PointerBatch { words:[(u64,u64,u64);5], len:usize, next:usize, dx:i32, dy:i32, order:u64 }
impl PointerBatch {
    const fn new()->Self {Self{words:[(0,0,0);5],len:0,next:0,dx:0,dy:0,order:0}}
}
struct PointerOutbox {
    batches:[PointerBatch;32], head:usize, len:usize, stalled:u32, sent_buttons:u8,
}
impl PointerOutbox {
    const STALL_LIMIT:u32=4096;
    const fn new()->Self {Self{batches:[PointerBatch::new();32],head:0,len:0,stalled:0,sent_buttons:0}}
    fn available(&self)->usize {self.batches.len()-self.len}
    fn push(&mut self, words:&[(u64,u64,u64)])->bool {
        if words.len()>5 || self.available()==0 {return false;}
        if words.is_empty() {return true;}
        let slot=(self.head+self.len)%self.batches.len();
        self.batches[slot].words[..words.len()].copy_from_slice(words);
        self.batches[slot].len=words.len();
        if words[0].2==EV_REL {
            self.batches[slot].dx=words[0].0 as i32;self.batches[slot].dy=words[0].1 as i32;
        }
        self.len+=1; true
    }
    fn reject_all(&mut self,error:u64) {
        sex_pdx::serial_record!("[sexinput.pointer.uncertain] reports={} error={} last_admitted_buttons={} button_recovery=unconfirmed pending=erased",
            self.len,error,self.sent_buttons);
        self.batches.fill(PointerBatch::new());self.head=0;self.len=0;self.stalled=0;
    }
    // True asks the caller to restore edge tracking to the last admitted state.
    // Expiry/rejection erases all suffixes: no wheel can outlive its own position.
    fn poll(&mut self,send:impl FnMut(u64,u64,u64)->Result<bool,u64>)->bool {
        self.poll_budget(4,true,send)
    }
    fn poll_budget(&mut self,budget:usize,allow_expiry:bool,mut send:impl FnMut(u64,u64,u64)->Result<bool,u64>)->bool {
        for _ in 0..budget {
            if self.len==0 {break;}
            let batch=&mut self.batches[self.head];
            let (a,b,c)=batch.words[batch.next];
            match send(a,b,c) {
                Ok(true)=>{
                    if c==EV_WHEEL {
                        sex_pdx::serial_record!("[sexinput.wheel.normalized] delta={} word={} reserved={} class={} admitted=1 up_positive=1",a as i64,a,b,c);
                    }
                    if c==EV_BTN && (1..=3).contains(&a) {
                        let bit=1u8<<((a-1) as u32);
                        if b==1 {self.sent_buttons|=bit;}else{self.sent_buttons&=!bit;}
                    }
                    batch.words[batch.next]=(0,0,0);batch.next+=1;self.stalled=0;
                    if batch.next==batch.len {
                        // Existing liveness receipts now follow the whole actual admission.
                        sex_pdx::serial_record!("[usb.hid.pointer.emit] op=OP_HID_EVENT buttons={} dx={} dy={} ok=1",self.sent_buttons,batch.dx,batch.dy);
                        sex_pdx::serial_record!("[usb.hid.pointer.pass] ok=1");
                        sex_pdx::serial_record!("[sexinput.usb_mouse.shell_send.ok]");
                        *batch=PointerBatch::new();self.head=(self.head+1)%self.batches.len();self.len-=1;
                    }
                }
                Ok(false)|Err(ERR_SERVICE_NOT_READY)=>{
                    self.stalled=self.stalled.saturating_add(1);
                    if allow_expiry && self.stalled>=Self::STALL_LIMIT {self.reject_all(ERR_SERVICE_NOT_READY);return true;}
                    break;
                }
                Err(error)=>{self.reject_all(error);return true;}
            }
        }
        false
    }
}

// Key custody is separate from pointer capacity. A retained key edge never
// expires on Full: dropping a modifier suffix could leave Shell holding Control.
#[derive(Clone,Copy)]
struct KeyEdge { code:u8, down:u8, source:u8, modifiers:u8 }
impl KeyEdge {
    const fn empty()->Self {Self{code:0,down:0,source:0,modifiers:0}}
}
struct KeyboardOutbox { edges:[KeyEdge;128], orders:[u64;128], head:usize, len:usize }
impl KeyboardOutbox {
    const fn new()->Self {Self{edges:[KeyEdge::empty();128],orders:[0;128],head:0,len:0}}
    fn available(&self)->usize {self.edges.len()-self.len}
    fn push(&mut self,edges:&[KeyEdge])->bool {
        if edges.len()>self.available() {return false;}
        for edge in edges {
            self.edges[(self.head+self.len)%self.edges.len()]=*edge;self.len+=1;
        }
        true
    }
    fn ps2(&mut self,raw:u64)->bool {
        if raw>255 {return false;}
        self.push(&[KeyEdge{code:(raw&0x7f)as u8,down:(raw&0x80==0)as u8,source:1,modifiers:0}])
    }
    fn usb(&mut self,key:u8,modifiers:u8,last_key:&mut u8,last_modifiers:&mut u8)->bool {
        let mut edges=[KeyEdge::empty();10];let mut count=0;
        let mut add=|code:u8,down:u8,mods:u8|{
            edges[count]=KeyEdge{code,down,source:2,modifiers:mods};count+=1;
        };
        // Release the previous key under its old modifier state, then change
        // modifier edges, then press the new key under the received snapshot.
        if key!=*last_key && *last_key!=0 {
            if let Some(code)=hid_to_ps2(*last_key) {add(code,0,*last_modifiers);}
        }
        let codes=[0x1d,0x2a,0x38,0x5b,0x61,0x36,0x64,0x5c];
        let changed=modifiers^*last_modifiers;
        for bit in 0..8 {if changed&(1<<bit)!=0 && modifiers&(1<<bit)==0 {add(codes[bit],0,modifiers);}}
        for bit in 0..8 {if changed&(1<<bit)!=0 && modifiers&(1<<bit)!=0 {add(codes[bit],1,modifiers);}}
        if key!=*last_key && key!=0 {
            if let Some(code)=hid_to_ps2(key) {add(code,1,modifiers);}
        }
        if !self.push(&edges[..count]) {return false;}
        *last_key=key;*last_modifiers=modifiers;true
    }
    fn reject_all(&mut self,error:u64) {
        sex_pdx::serial_record!("[sexinput.key.uncertain] pending={} error={} modifier_recovery=unconfirmed",self.len,error);
        self.edges.fill(KeyEdge::empty());self.orders.fill(0);self.head=0;self.len=0;
    }
    // True means no pointer work may use the remaining admission budget.
    fn poll(&mut self,budget:&mut usize,mut send:impl FnMut(u64,u64,u64)->Result<bool,u64>)->bool {
        while self.len>0 && *budget>0 {
            let edge=self.edges[self.head];*budget-=1;
            match send(edge.code as u64,edge.down as u64,EV_KEY) {
                Ok(true)=>{
                    sex_pdx::serial_record!("[sexinput.key.send] code={} down={} mod={} source={} dst=6 ok=1 err=0",edge.code,edge.down,edge.modifiers,edge.source);
                    if edge.source==1 {
                        if edge.down==1 {sex_pdx::serial_record!("[input.keyboard.keydown.ok] code={} source=ps2",edge.code);}
                        else {sex_pdx::serial_record!("[input.keyboard.keyup.ok] code={} source=ps2",edge.code);}
                    }else{
                        if edge.down==1 {sex_pdx::serial_record!("[input.keyboard.keydown.ok] code={} source=usb",edge.code);}
                        else {sex_pdx::serial_record!("[input.keyboard.keyup.ok] code={} source=usb",edge.code);}
                    }
                    self.edges[self.head]=KeyEdge::empty();self.orders[self.head]=0;self.head=(self.head+1)%self.edges.len();self.len-=1;
                }
                Ok(false)|Err(ERR_SERVICE_NOT_READY)=>return true,
                Err(error)=>{
                    self.reject_all(error);return true;
                }
            }
        }
        self.len>0
    }
}

// This counter is ordering data, not authority. No wrap or reuse is permitted.
struct InputArrival { last:u64 }
impl InputArrival {
    const fn new()->Self {Self{last:0}}
    fn pointer(&mut self,outbox:&mut PointerOutbox,words:&[(u64,u64,u64)])->bool {
        let Some(order)=self.last.checked_add(1) else{return false;};
        let slot=(outbox.head+outbox.len)%outbox.batches.len();
        if !outbox.push(words) {return false;}
        if !words.is_empty() {outbox.batches[slot].order=order;self.last=order;}
        true
    }
    fn ps2(&mut self,outbox:&mut KeyboardOutbox,raw:u64)->bool {
        let Some(order)=self.last.checked_add(1) else{return false;};
        let slot=(outbox.head+outbox.len)%outbox.edges.len();
        if !outbox.ps2(raw) {return false;}
        outbox.orders[slot]=order;self.last=order;true
    }
    fn usb(&mut self,outbox:&mut KeyboardOutbox,key:u8,modifiers:u8,last_key:&mut u8,last_modifiers:&mut u8)->bool {
        let Some(order)=self.last.checked_add(1) else{return false;};
        let before=outbox.len;
        if !outbox.usb(key,modifiers,last_key,last_modifiers) {return false;}
        for i in before..outbox.len {outbox.orders[(outbox.head+i)%outbox.edges.len()]=order;}
        self.last=order;true
    }
}

// Separate reserved capacities, one causal arrival order. Later keys cannot
// overtake an older click, and later pointer traffic cannot starve retained keys.
fn poll_input_delivery(keys:&mut KeyboardOutbox,pointer:&mut PointerOutbox,
    mut send:impl FnMut(u64,u64,u64)->Result<bool,u64>)->bool {
    let mut reset_buttons=false;
    for _ in 0..4 {
        if keys.len==0 && pointer.len==0 {break;}
        let key_first=keys.len>0 && (pointer.len==0 || keys.orders[keys.head]<pointer.batches[pointer.head].order);
        let mut blocked=false;
        if key_first {
            keys.poll(&mut 1,|a,b,c|{let result=send(a,b,c);blocked=!matches!(result,Ok(true));result});
        }else{
            // A queued key depends on this earlier position/click. Keep its
            // unsent suffix on Full instead of changing the key's destination.
            let mut failure=ERR_SERVICE_NOT_READY;
            let reset=pointer.poll_budget(1,keys.len==0,|a,b,c|{
                let result=send(a,b,c);blocked=!matches!(result,Ok(true));
                if let Err(error)=result {failure=error;}result
            });
            if reset && keys.len>0 {keys.reject_all(failure);}
            reset_buttons|=reset;
        }
        if blocked {break;}
    }
    reset_buttons
}

fn send_shell_hid_event(arg0: u64, arg1: u64, arg2: u64) -> Result<bool, u64> {
    if CAP_READY_SHELL.load(Ordering::Relaxed) {
        return pdx_call_checked(SLOT_SHELL, OP_HID_EVENT, arg0, arg1, arg2).map(|_| {
            if !EDGE_SEND_EMITTED_SHELL.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.send from=sexinput to=silk-shell slot=6 op=OP_HID_EVENT first=1]");
            }
            true
        });
    }

    match pdx_call_checked(SLOT_SHELL, OP_HID_EVENT, arg0, arg1, arg2) {
        Ok(_) => {
            CAP_READY_SHELL.store(true, Ordering::Relaxed);
            if !EDGE_SEND_EMITTED_SHELL.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.send from=sexinput to=silk-shell slot=6 op=OP_HID_EVENT first=1]");
            }
            Ok(true)
        }
        Err(err) if err == ERR_CAP_INVALID => {
            if !DEFER_EMITTED_SHELL.swap(true, Ordering::Relaxed) {
                serial_println!("[bootgraph.edge.defer from=sexinput to=silk-shell slot=6 reason=missing_cap]");
            }
            sys_yield();
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

/// Translate USB HID keyboard usage to the shell key code.
/// Ordinary keys use set1 values; navigation uses the existing normalized codes.
/// Returns None for unmapped keys.
fn hid_to_ps2(hid: u8) -> Option<u8> {
    match hid {
        0x47 => Some(0x46), // Scroll Lock
        0x4A => Some(0x66), // Home
        0x4B => Some(0x68), // Page Up
        0x4D => Some(0x6B), // End
        0x4E => Some(0x6D), // Page Down
        0x51 => Some(0x6C), // Down
        0x52 => Some(0x67), // Up
        0x28 => Some(0x1C), // Enter
        0x44 => Some(0x57), // F11
        0x45 => Some(0x58), // F12
        0x2C => Some(0x39), // Space
        0x2A => Some(0x0E), // Backspace
        0x29 => Some(0x01), // Esc
        0x04 => Some(0x1E), // a
        0x05 => Some(0x30), // b
        0x06 => Some(0x2E), // c
        0x07 => Some(0x20), // d
        0x08 => Some(0x12), // e
        0x09 => Some(0x21), // f
        0x0A => Some(0x22), // g
        0x0B => Some(0x23), // h
        0x0C => Some(0x17), // i
        0x0D => Some(0x24), // j
        0x0E => Some(0x25), // k
        0x0F => Some(0x26), // l
        0x10 => Some(0x32), // m
        0x11 => Some(0x31), // n
        0x12 => Some(0x18), // o
        0x13 => Some(0x19), // p
        0x14 => Some(0x10), // q
        0x15 => Some(0x13), // r
        0x16 => Some(0x1F), // s
        0x17 => Some(0x14), // t
        0x18 => Some(0x16), // u
        0x19 => Some(0x2F), // v
        0x1A => Some(0x11), // w
        0x1B => Some(0x2D), // x
        0x1C => Some(0x15), // y
        0x1D => Some(0x2C), // z
        _ => None,
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[sexinput.init.start]");
    sex_rt::heap_init();
    serial_println!("[sexinput] Normalizer Starting...");
    // USB_HID_POINTER_PRODUCER_V1 contract: 0x260 raw path expects abs coords
    // in 0..=32767 (bit 32 of arg2 = is_abs) and button bits within 0x07.
    serial_println!("[sexinput.usb.contract] op=0x260 raw_max=32767 buttons_mask=0x7 ok=1");

    // Budgeted proof gate state diagnostic.
    // Fires once at boot to confirm whether synthetic input proofs are enabled.
    unsafe {
        static mut PROOF_GATE_BUDGET: u32 = 1;
        let remaining = &mut PROOF_GATE_BUDGET;
        if *remaining > 0 {
            *remaining -= 1;
            let state = if SYNTHETIC_INPUT_PROOFS_DISABLED { 0 } else { 1 };
            let source = if SYNTHETIC_INPUT_PROOFS_DISABLED { "desktop_default" } else { "declared_cfg" };
            serial_println!("[proof.gate.state] enabled={} source={}", state, source);
        }
    }

    // Budgeted keyboard cursor gate diagnostic.
    // Fires once at boot to confirm whether keyboard cursor fallback is enabled.
    unsafe {
        static mut KBD_CURSOR_GATE_BUDGET: u32 = 1;
        let remaining = &mut KBD_CURSOR_GATE_BUDGET;
        if *remaining > 0 {
            *remaining -= 1;
            serial_println!("[keyboard_cursor.gate] enabled={} source={}",
                if KEYBOARD_CURSOR_ENABLED { 1 } else { 0 },
                if KEYBOARD_CURSOR_ENABLED { "env" } else { "default" });
        }
    }
    // Cursor autodemo gate diagnostic. Fires once at boot.
    serial_println!("[cursor.autodemo.gate] enabled={}", CURSOR_AUTODEMO_ENABLED as u8);
    // Autodemo decoupling proof: keyboard_cursor and autodemo are now independent.
    if KEYBOARD_CURSOR_ENABLED {
        serial_println!(
            "[cursor.input.autodemo.decoupled] keyboard_cursor=1 autodemo={} ok=1",
            CURSOR_AUTODEMO_ENABLED as u8
        );
    }
    unsafe {
        sys_set_state(SVC_STATE_LISTENING);
    }

    let mut pointer_outbox = PointerOutbox::new();
    let mut keyboard_outbox = KeyboardOutbox::new();
    let mut input_arrival = InputArrival::new();
    let mut tick: u64 = 0;
    let mut drag_proof_stage: u8 = 0;
    let mut synth_click_stage: u8 = 0;
    let mut silkbar_click_stage: u8 = 0;
    let mut kbd_proof_stage: u8 = 0;
    let mut ev_key_edge_stage: u8 = 0;

    serial_println!("[sexinput.ready]");
    // No synthetic, PS/2, or USB producer may send shell work until the
    // authenticated shell has completed initialization. Retain received USB
    // payloads unchanged and drain this FIFO before new ordinary ring messages.
    if unsafe { (*core::ptr::addr_of_mut!(STARTUP_INBOX)).wait_for_shell() }.is_err() {
        serial_println!("[shell.ready.wait.failed] reason=inbox_full");
        panic!("sexinput shell-ready inbox full");
    }
    serial_println!("[sexinput.shell.ready.accept] caller=3");
    loop {
        unsafe {
            static mut SYNTHETIC_CLICK_PROOF_GATED_MARKER_EMITTED: bool = false;
            if !SYNTHETIC_CLICK_PROOF_GATED_MARKER_EMITTED {
                SYNTHETIC_CLICK_PROOF_GATED_MARKER_EMITTED = true;
                serial_println!("[sexinput.synthetic.click.proof.gated] ok=1");
            }
        }

        // 0a. One-shot abs translation proof: fires at tick 0 before REAL_USB_POINTER_SEEN.
        //     Sends center-of-range raw coords (16000, 12000) via EV_ABS to shell to prove:
        //     normalize_abs_coord(0..32767) → screen coord, abs.init, abs.delta, abs.pass.
        //     Uses coords that survive the zero_init and edge_before_ready poison filters.
        //     Gated by !SYNTHETIC_INPUT_PROOFS_DISABLED only — independent of USB state.
        unsafe {
            static mut ABS_TRANSLATION_PROOF_FIRED: bool = false;
            if !ABS_TRANSLATION_PROOF_FIRED && !SYNTHETIC_INPUT_PROOFS_DISABLED {
                ABS_TRANSLATION_PROOF_FIRED = true;
                serial_println!("[sexinput.abs.translation.proof] x_raw=16000 y_raw=12000 ok=1");
                pdx_call(SLOT_SHELL, OP_HID_EVENT, 16000u64, 12000u64, EV_ABS);
            }
        }

        // 0. Local USB->sexinput PDX proof path (no shell routing in this phase).
        // Reserve enough key slots before consuming a possible USB snapshot.
        // A final modifier-up must remain in the actual input ring on backpressure.
        if let Some(req) = (if keyboard_outbox.available()>=10 {
            pop_startup_message().or_else(|| pdx_try_listen_raw(0))
        }else{None}) {
            'input_message: {
            serial_println!("[sexinput.usb_mouse.recv] type={:#x}", req.type_id);
            if (req.type_id == OP_USB_MOUSE_REPORT || req.type_id == OP_USB_KEYBOARD_REPORT)
                && !usb_report_sender(&req) {
                sex_pdx::serial_record!("[sexinput.usb.reject] caller={} task={} reason=sender",
                    req.caller_pd, req.caller_task);
                break 'input_message;
            }
            if req.type_id == OP_USB_MOUSE_REPORT && pointer_outbox.available() == 0 {
                sex_pdx::serial_record!("[sexinput.pointer.rejected] packed={} reason=outbox_full_or_sequence admitted=0", req.arg2);
            } else if req.type_id == OP_USB_MOUSE_REPORT {
                'pointer_report: {
                unsafe { REAL_USB_POINTER_SEEN = true; }
                // Raw payload diagnostic: log exact arg1/arg2 before budgeted decode.
                // Budget 64 covers init zero-report + first ~63 real reports.
                // Use hex to avoid sign-extension ambiguity in the log.
                unsafe {
                    static mut RAW_PAYLOAD_BUDGET: u32 = 64;
                    let r = &mut RAW_PAYLOAD_BUDGET;
                    if *r > 0 {
                        *r -= 1;
                        serial_println!(
                            "[sexinput.pointer.raw] a0={:#x} a1={:#x} a2={:#x} caller={}",
                            req.arg0, req.arg1, req.arg2, req.caller_pd
                        );
                    }
                }
                let cls = req.arg0;
                unsafe {
                    static mut SEXINPUT_POINTER_RECV_BUDGET: u32 = 16;
                    let rem = &mut SEXINPUT_POINTER_RECV_BUDGET;
                    if *rem > 0 {
                        *rem -= 1;
                        serial_println!("[sexinput.pointer.recv] class={} a0={} a1={}", cls, req.arg1 as i32, req.arg2 as i32);
                    }
                }

                // Trusted legacy normalized packets join the same causal FIFO.
                if cls == EV_REL || cls == EV_ABS || cls == EV_BTN {
                    if !input_arrival.pointer(&mut pointer_outbox,&[(req.arg1,req.arg2,cls)]) {
                        sex_pdx::serial_record!("[sexinput.pointer.rejected] class={} reason=outbox_full_or_sequence admitted=0",cls);
                    }
                    break 'pointer_report;
                }

                let Some(report) = (if cls == 0 { decode_pointer_payload(req.arg1, req.arg2) } else { None }) else {
                    serial_println!("[sexinput.usb.report.reject] reason=noncanonical");
                    break 'pointer_report;
                };
                let HidPointerRawReport { dx, dy, buttons, wheel, is_abs } = report;
                if wheel != 0 {
                    sex_pdx::serial_record!("[sexinput.wheel.received] caller={} task={} absolute={} delta={} packed={} up_positive=1",
                        req.caller_pd, req.caller_task, is_abs as u8, wheel, req.arg2);
                }

                // TOUCHPAD_ABS_CONTACT_V1: single-contact model, marker-only.
                // Driven by the abs stream's button bit 0; emits no events to
                // the shell — OP_HID_EVENT output is unchanged by this block.
                if is_abs {
                    unsafe {
                        static mut CONTACT_DOWN: bool = false;
                        static mut CONTACT_X: u16 = 0;
                        static mut CONTACT_Y: u16 = 0;
                        static mut CONTACT_START_X: u16 = 0;
                        static mut CONTACT_START_Y: u16 = 0;
                        static mut CONTACT_DRAG_BUDGET: u32 = 16;
                        static mut CONTACT_SPURIOUS_UP_EMITTED: bool = false;
                        static mut CONTACT_LAST_BTN0: bool = false;
                        let btn0 = (buttons & 1) != 0;
                        let (cx, cy) = (dx as u16, dy as u16);
                        if btn0 && !CONTACT_DOWN {
                            CONTACT_DOWN = true;
                            CONTACT_X = cx;
                            CONTACT_Y = cy;
                            CONTACT_START_X = cx;
                            CONTACT_START_Y = cy;
                            serial_println!("[sexinput.contact.down] x={} y={}", cx, cy);
                        } else if btn0 && CONTACT_DOWN {
                            let ddx = cx as i32 - CONTACT_X as i32;
                            let ddy = cy as i32 - CONTACT_Y as i32;
                            CONTACT_X = cx;
                            CONTACT_Y = cy;
                            if (ddx != 0 || ddy != 0) && CONTACT_DRAG_BUDGET > 0 {
                                CONTACT_DRAG_BUDGET -= 1;
                                serial_println!(
                                    "[sexinput.contact.drag] x={} y={} dx={} dy={}",
                                    cx, cy, ddx, ddy
                                );
                            }
                        } else if !btn0 && CONTACT_DOWN {
                            let tdx = cx as i32 - CONTACT_START_X as i32;
                            let tdy = cy as i32 - CONTACT_START_Y as i32;
                            CONTACT_DOWN = false;
                            serial_println!(
                                "[sexinput.contact.up] x={} y={} total_dx={} total_dy={}",
                                cx, cy, tdx, tdy
                            );
                        } else if !btn0 && !CONTACT_DOWN && CONTACT_LAST_BTN0
                            && !CONTACT_SPURIOUS_UP_EMITTED
                        {
                            // Release edge observed while model is idle —
                            // state desync guard, never expected in practice.
                            CONTACT_SPURIOUS_UP_EMITTED = true;
                            serial_println!("[sexinput.contact.spurious_up.ignored]");
                        }
                        CONTACT_LAST_BTN0 = btn0;
                    }
                }
                let kind = if is_abs {
                    "absolute"
                } else if buttons == 0 && dx == 0 && dy == 0 && wheel == 0 {
                    "relative_or_zero"
                } else {
                    "relative"
                };
                serial_println!(
                    "[sexinput.usb_mouse.decode.ok] buttons={:#x} dx={} dy={} wheel={} is_abs={}",
                    buttons,
                    dx,
                    dy,
                    wheel,
                    is_abs
                );
                serial_println!(
                    "[usb.hid.pointer.normalized] buttons={} dx={} dy={} kind={} ok=1",
                    buttons,
                    dx,
                    dy,
                    kind
                );
                serial_println!(
                    "[input.hid.pointer.recv] source=usb buttons={} dx={} dy={} ok=1",
                    buttons,
                    dx,
                    dy
                );
                unsafe {
                    static mut POINTER_MODE_BUDGET_PACKED: u32 = 64;
                    if POINTER_MODE_BUDGET_PACKED > 0 {
                        POINTER_MODE_BUDGET_PACKED -= 1;
                        let mode = if is_abs { "abs" } else { "rel" };
                        serial_println!(
                            "[sexinput.pointer.mode] mode={} buttons={:#x} dx={} dy={} wheel={}",
                            mode, buttons, dx, dy, wheel
                        );
                    }
                }

                // AP13: proof marker before normalizer call
                // Only fires for nonzero content — the bootgraph zero-report
                // (buttons=0 dx=0 dy=0 is_abs=0) does not trigger this marker.
                if buttons != 0 || dx != 0 || dy != 0 || is_abs {
                    unsafe {
                        static mut MOUSE_TO_NORMALIZER_PROOF_EMITTED: bool = false;
                        if !MOUSE_TO_NORMALIZER_PROOF_EMITTED {
                            MOUSE_TO_NORMALIZER_PROOF_EMITTED = true;
                            serial_println!(
                                "[usb.mouse.to_normalizer] dx={} dy={} buttons={} wheel={} is_abs={} ok=1",
                                dx, dy, buttons, wheel, if is_abs { 1 } else { 0 }
                            );
                        }
                    }
                }
                serial_println!("[sexinput.usb_mouse.normalize.start]");
                let report = HidPointerRawReport {
                    dx,
                    dy,
                    buttons,
                    wheel,
                    is_abs,
                };
                let mut normalized_events: [(u64, u64, u64); 5] = [(0, 0, 0); 5];
                let mut norm_count: usize = 0;
                let previous_buttons=unsafe { LAST_BUTTONS };
                normalize_pointer_report_v1(
                    report,
                    unsafe { &mut *core::ptr::addr_of_mut!(LAST_BUTTONS) },
                    |arg0, arg1, arg2| {
                        if norm_count < normalized_events.len() {
                            normalized_events[norm_count] = (arg0, arg1, arg2);
                            norm_count += 1;
                        }
                    },
                );
                serial_println!("[sexinput.usb_mouse.normalize.ok]");

                // AP13: one-shot normalizer output proof markers
                unsafe {
                    static mut NORMALIZER_OUT_PROOF_EMITTED: bool = false;
                    if !NORMALIZER_OUT_PROOF_EMITTED && norm_count > 0 {
                        NORMALIZER_OUT_PROOF_EMITTED = true;
                        for i in 0..norm_count {
                            let (arg0, arg1, arg2) = normalized_events[i];
                            let cls_str = if arg2 == EV_REL { "EV_REL" }
                                else if arg2 == EV_ABS { "EV_ABS" }
                                else if arg2 == EV_WHEEL { "EV_WHEEL" }
                                else { "EV_BTN" };
                            serial_println!(
                                "[usb.mouse.normalizer.out] class={} code={} value={} ok=1",
                                cls_str, arg0 as i32, arg1 as i32
                            );
                            // AP14: pointer producer normalized marker
                            serial_println!(
                                "[usb.pointer.producer.normalized] class={} ok=1",
                                arg2
                            );
                        }
                    }
                }

                // Budgeted diagnostic for real USB mouse deltas arriving from sexusb.
                // Not triggered by synthetic proof paths (those send HID_EVENT directly).
                unsafe {
                    static mut MOUSE_REAL_DELTA_BUDGET: u32 = 16;
                    let remaining = &mut MOUSE_REAL_DELTA_BUDGET;
                    if *remaining > 0 {
                        *remaining -= 1;
                        serial_println!("[sexinput.mouse.real.delta] dx={} dy={} buttons={:#x}", dx, dy, buttons);
                    }
                }
                // Budgeted liveness: sexinput received a mouse report from sexusb (tablet path).
                unsafe {
                    static mut MOUSE_LIVE_BUDGET: u32 = 16;
                    let rem = &mut MOUSE_LIVE_BUDGET;
                    if *rem > 0 {
                        *rem -= 1;
                        serial_println!("[sexinput.mouse.live] n=0 dx={} dy={} buttons=0x{:x}", dx, dy, buttons);
                    }
                }

                // Budgeted diagnostic for real USB button transitions.
                // Fires only when buttons field is nonzero (actual button event).
                // Synthetic click-focus proof also sends OP_USB_MOUSE_REPORT but
                // with buttons=0x01/0x00 at known early ticks; real clicks happen
                // later and this marker distinguishes button events from pure moves.
                unsafe {
                    static mut MOUSE_REAL_BUTTON_BUDGET: u32 = 16;
                    let remaining = &mut MOUSE_REAL_BUTTON_BUDGET;
                    if *remaining > 0 && buttons != 0 {
                        *remaining -= 1;
                        serial_println!("[sexinput.mouse.real.button] buttons={:#x} dx={} dy={}", buttons, dx, dy);
                    }
                }

                // Batch admission is all-or-nothing; LAST_BUTTONS follows retained work.
                if !input_arrival.pointer(&mut pointer_outbox,&normalized_events[..norm_count]) {
                    unsafe { LAST_BUTTONS=previous_buttons; }
                    sex_pdx::serial_record!("[sexinput.pointer.rejected] packed={} reason=outbox_full_or_sequence admitted=0",req.arg2);
                }
                }
            } else if req.type_id == OP_USB_KEYBOARD_REPORT {
                // USB HID boot keyboard report via sexusb.
                // arg0 = reserved, arg1 = modifiers, arg2 = first keycode (USB HID usage ID)
                let modifiers = req.arg1 as u8;
                let keycode = req.arg2 as u8;
                unsafe {
                    static mut KBD_RECV_BUDGET: u32 = 16;
                    let rem = &mut KBD_RECV_BUDGET;
                    if *rem > 0 {
                        *rem -= 1;
                        serial_println!("[sexinput.kbd.recv] key=0x{:x} mod=0x{:x}", keycode, modifiers);
                    }
                }
                // Dev-only keyboard cursor fallback (SEXOS_KEYBOARD_CURSOR=1).
                // Map USB HID usage IDs to EV_REL cursor movement.
                // Only on key-down (nonzero keycode, no rate limiting).
                if KEYBOARD_CURSOR_ENABLED && keycode != 0 {
                    // USB HID keyboard usage IDs for cursor keys + WASD
                    let (dx, dy): (i32, i32) = match keycode {
                        0x1a | 0x52 => (0, -8),   // W / Up
                        0x16 | 0x51 => (0, 8),    // S / Down
                        0x04 | 0x50 => (-8, 0),   // A / Left
                        0x07 | 0x4f => (8, 0),    // D / Right
                        _ => (0, 0),
                    };
                    if dx != 0 || dy != 0 {
                        unsafe {
                            static mut KBD_CURSOR_KEY_BUDGET_USB: u32 = 16;
                            let rem = &mut KBD_CURSOR_KEY_BUDGET_USB;
                            if *rem > 0 {
                                *rem -= 1;
                                serial_println!("[keyboard_cursor.key] code=0x{:x} dx={} dy={}", keycode, dx, dy);
                            }
                        }
                        pdx_call(SLOT_SHELL, OP_HID_EVENT, dx as u64, dy as u64, EV_REL);
                        unsafe {
                            static mut KBD_CURSOR_REL_BUDGET_USB: u32 = 16;
                            let rem = &mut KBD_CURSOR_REL_BUDGET_USB;
                            if *rem > 0 {
                                *rem -= 1;
                                serial_println!("[keyboard_cursor.emit.rel] dx={} dy={}", dx, dy);
                            }
                        }
                    }
                }
                // Retain a whole physical report before advancing key/modifier state.
                let retained=unsafe {input_arrival.usb(&mut keyboard_outbox,keycode,modifiers,
                    &mut *core::ptr::addr_of_mut!(LAST_USB_KEY),
                    &mut *core::ptr::addr_of_mut!(LAST_USB_MODIFIERS))};
                if !retained {
                    sex_pdx::serial_record!("[sexinput.key.rejected] source=usb key={} modifiers={} reason=outbox_full_or_sequence admitted=0",keycode,modifiers);
                }
            }
            }
        }

        // 1. Non-blocking poll for raw scancodes from kernel (SLOT_INPUT = 3)
        //    Non-blocking allows synthetic producer to run on idle ticks.
        if let Some(req) = (if keyboard_outbox.available()>0 {pdx_try_listen_raw(SLOT_INPUT)}else{None}) {
            // Kernel RawInput is type 0x201, arg0 = scancode
            if req.type_id == 0x201 {
                let scancode = req.arg0;
                sex_pdx::serial_record!("[sexinput.ps2.scancode] raw={:#x}", scancode);

                // 2. Normalize and forward to silk-shell (SLOT_SHELL = 6)
                // Typed event via 0x202: arg0=code(break-bit stripped), arg1=1(press)/0(release), arg2=EV_KEY
                let value = if scancode & 0x80 == 0 { 1 } else { 0 };
                let code = (scancode & 0x7F) as u64;
                if !input_arrival.ps2(&mut keyboard_outbox,scancode) {
                    sex_pdx::serial_record!("[sexinput.key.rejected] source=ps2 raw={} reason=noncanonical_or_full admitted=0",scancode);
                }

                // 2b. Dev-only keyboard cursor fallback (SEXOS_KEYBOARD_CURSOR=1).
                //     On key press, emit EV_REL to move cursor in 8px steps.
                //     Arrow keys (0x48/0x50/0x4B/0x4D) and WASD (0x11/0x1F/0x1E/0x20).
                //     Gate unset = zero overhead, no behavior change.
                if KEYBOARD_CURSOR_ENABLED && value == 1 {
                    let (dx, dy): (i32, i32) = match code {
                        0x11 | 0x48 => (0, -8),   // W / Up
                        0x1F | 0x50 => (0, 8),    // S / Down
                        0x1E | 0x4B => (-8, 0),   // A / Left
                        0x20 | 0x4D => (8, 0),    // D / Right
                        _ => (0, 0),
                    };
                    if dx != 0 || dy != 0 {
                        unsafe {
                            static mut KBD_CURSOR_KEY_BUDGET: u32 = 16;
                            let rem = &mut KBD_CURSOR_KEY_BUDGET;
                            if *rem > 0 {
                                *rem -= 1;
                                serial_println!("[keyboard_cursor.key] code=0x{:x} dx={} dy={}", code, dx, dy);
                            }
                        }
                        pdx_call(SLOT_SHELL, OP_HID_EVENT, dx as u64, dy as u64, EV_REL);
                        unsafe {
                            static mut KBD_CURSOR_REL_BUDGET: u32 = 16;
                            let rem = &mut KBD_CURSOR_REL_BUDGET;
                            if *rem > 0 {
                                *rem -= 1;
                                serial_println!("[keyboard_cursor.emit.rel] dx={} dy={}", dx, dy);
                            }
                        }
                    }
                }
            }
        }

        // Intake both physical key paths before draining the shared arrival order.
        // Reserved key capacity and four total attempts prevent pointer starvation.
        if poll_input_delivery(&mut keyboard_outbox,&mut pointer_outbox,send_shell_hid_event) {
            unsafe {LAST_BUTTONS=pointer_outbox.sent_buttons;}
        }

        // 2c. Bounded one-shot keyboard cursor self-test.
        //     When KEYBOARD_CURSOR_ENABLED is set, fires exactly one EV_REL(0, -8)
        //     on the first poll cycle after boot to prove the pipeline works without
        //     requiring QEMU host input routing (which is broken on QEMU 11.0.0).
        //     Uses the same pdx_call path as real keyboard cursor movement.
        //     One-shot: KBD_SELF_TEST_DONE prevents replay.
        if KEYBOARD_CURSOR_ENABLED {
            unsafe {
                static mut KBD_SELF_TEST_DONE: bool = false;
                if !KBD_SELF_TEST_DONE {
                    KBD_SELF_TEST_DONE = true;
                    serial_println!("[keyboard_cursor.self_test] dx=0 dy=-8");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 0u64, (-8i64) as u64, EV_REL);
                    serial_println!("[keyboard_cursor.self_test.ok]");
                }
            }
        }

        // 3. Deterministic synthetic drag proof (bounded):
        //    EV_ABS anchor, then BTN down -> REL move -> BTN up via normalizer.
        //    One-shot: after stages 0→1→2 complete, SYNTHETIC_DRAG_PROOF_DONE
        //    prevents further replay. Without this gate, drag_proof_stage wraps
        //    with % 3 forever, causing endless drag.start/move/end cycles every
        //    120 ticks that flood the shell and starve visual updates.
        tick = tick.wrapping_add(1);
        // Ticks 5, 6, 7: one stage per tick, no overlap with EV_KEY (3/4) or click-focus (10/14/15).
        // Original threshold was tick%120 (~171s to complete); reduced for CI 25s probe window.
        const DRAG_TICKS: [u64; 3] = [5, 6, 7];
        if !SYNTHETIC_INPUT_PROOFS_DISABLED
            && !unsafe { REAL_USB_POINTER_SEEN }
            && !unsafe { SYNTHETIC_DRAG_PROOF_DONE }
            && drag_proof_stage < 3 && tick == DRAG_TICKS[drag_proof_stage as usize]
        {
            let report = match drag_proof_stage {
                0 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 200, 200, EV_ABS);
                    serial_println!("[sexinput.drag_proof.start]");
                    HidPointerRawReport { dx: 0, dy: 0, buttons: 0x01, wheel: 0, is_abs: false } // left down edge
                }
                1 => HidPointerRawReport { dx: 6, dy: 4, buttons: 0x01, wheel: 0, is_abs: false }, // drag move
                _ => {
                    // Stage 2 (final): button up. Set DONE to prevent wrap/replay.
                    unsafe { SYNTHETIC_DRAG_PROOF_DONE = true; }
                    serial_println!("[sexinput.drag_proof.done]");
                    HidPointerRawReport { dx: 0, dy: 0, buttons: 0x00, wheel: 0, is_abs: false } // left up edge
                }
            };

            normalize_pointer_report_v1(report, unsafe { &mut *core::ptr::addr_of_mut!(LAST_BUTTONS) }, |arg0, arg1, arg2| {
                pdx_call(SLOT_SHELL, OP_HID_EVENT, arg0, arg1, arg2);
                if arg2 == EV_BTN && arg0 == 1 && arg1 == 1 {
                    serial_println!("[sexinput.drag_proof.down]");
                } else if arg2 == EV_REL {
                    serial_println!("[sexinput.drag_proof.move]");
                } else if arg2 == EV_BTN && arg0 == 1 && arg1 == 0 {
                    serial_println!("[sexinput.drag_proof.up]");
                }
            });

            drag_proof_stage = (drag_proof_stage + 1) % 3;
        }

        // 4. Synthetic SilkBar click proof via HID_EVENT path.
        //    Fires clicks on launcher, workspace, status chip, clock.
        //    Resets CLICK_ACTIVE before each click to avoid drag-proof interference.
        if !SYNTHETIC_INPUT_PROOFS_DISABLED && SILKBAR_CLICK_PROOF_ENABLED
            && !unsafe { SYNTHETIC_SILKBAR_CLICK_PROOF_DONE } {
            match silkbar_click_stage {
                // Reset CLICK_ACTIVE from drag proof (stage 0 at tick 0 sets left held)
                0 if tick == 2 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 1;
                }
                // Click launcher at (100, 25)
                1 if tick == 3 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 100, 25, EV_ABS);
                    silkbar_click_stage = 2;
                }
                2 if tick == 4 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=launcher");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 3;
                }
                3 if tick == 5 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 4;
                }
                // Click workspace 2 (index=2) at (635, 25) — SwitchWorkspace(3), ws_idx=2
                4 if tick == 7 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 635, 25, EV_ABS);
                    silkbar_click_stage = 5;
                }
                5 if tick == 8 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=workspace index=3");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 6;
                }
                6 if tick == 9 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 7;
                }
                // Click status chip at (940, 25)
                7 if tick == 11 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 940, 25, EV_ABS);
                    silkbar_click_stage = 8;
                }
                8 if tick == 12 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=status");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 9;
                }
                9 if tick == 13 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 10;
                }
                // Click clock at (1100, 25)
                10 if tick == 15 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1100, 25, EV_ABS);
                    silkbar_click_stage = 11;
                }
                11 if tick == 16 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=clock");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 12;
                }
                12 if tick == 17 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 13;
                }
                // Close launcher panel (click launcher button again to dismiss)
                13 if tick == 19 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 100, 25, EV_ABS);
                    silkbar_click_stage = 14;
                }
                14 if tick == 20 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=launcher_close");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 15;
                }
                15 if tick == 21 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 16;
                }
                // Close status panel (second click on status chip at 940, 25)
                16 if tick == 23 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 940, 25, EV_ABS);
                    silkbar_click_stage = 17;
                }
                17 if tick == 24 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=status_close");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 18;
                }
                18 if tick == 25 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 19;
                }
                // Close clock panel (second click on clock at 1100, 25)
                19 if tick == 27 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1100, 25, EV_ABS);
                    silkbar_click_stage = 20;
                }
                20 if tick == 28 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=clock_close");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 21;
                }
                21 if tick == 29 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 22;
                }
                // Click bell at (1025, 25) — centered in CHIP_X_BELL=1020..1038
                22 if tick == 31 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1025, 25, EV_ABS);
                    silkbar_click_stage = 23;
                }
                23 if tick == 32 => {
                    serial_println!("[sexinput.synthetic.silkbar_click] target=bell");
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 1, EV_BTN);
                    silkbar_click_stage = 24;
                }
                24 if tick == 33 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 1, 0, EV_BTN);
                    silkbar_click_stage = 25;
                    unsafe { SYNTHETIC_SILKBAR_CLICK_PROOF_DONE = true; }
                }
                _ => {}
            }
        } else {
            unsafe {
                static mut SILKBAR_CLICK_DISABLED_MARKER_EMITTED: bool = false;
                if !SILKBAR_CLICK_DISABLED_MARKER_EMITTED {
                    SILKBAR_CLICK_DISABLED_MARKER_EMITTED = true;
                    serial_println!("[sexinput.synthetic.silkbar_click.disabled]");
                }
            }
        }

        // 5. One-shot synthetic click proof via USB mouse path.
        //    Sets cursor to (940,560) ∈ SURFACE_ID_LINEN, then clicks.
        //    Proves sexinput→shell click_focus chain.
        //    Uses EV_ABS for positioning (not delta accumulation) to avoid
        //    coordinate corruption from concurrent silkbar proof EV_ABS events.
        if !SYNTHETIC_INPUT_PROOFS_DISABLED && !unsafe { REAL_USB_POINTER_SEEN } {
            match synth_click_stage {
                0 if tick == 10 => {
                    // Init cursor (POINTER_USB_STATE_INIT → 640,400)
                    serial_println!("[sexinput.synthetic.click_focus.start]");
                    let _ = pdx_call_checked(SLOT_SHELL, OP_USB_MOUSE_REPORT, 0, 0u64, 0u64);
                    synth_click_stage = 1;
                }
                1 if tick == 14 => {
                    // EV_ABS anchors cursor at (940,560) ∈ LINEN [900,1200)×[500,650)
                    // This explicit positioning is immune to EV_ABS interference from
                    // the synthetic silkbar click proof (ticks 2-33).
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 940, 560, EV_ABS);
                    // Button down — triggers click_focus hit on SURFACE_ID_LINEN
                    serial_println!("[sexinput.synthetic.click_focus.down]");
                    let _ = pdx_call_checked(SLOT_SHELL, OP_USB_MOUSE_REPORT, 0, 0x01u64, 0u64);
                    synth_click_stage = 2;
                }
                2 if tick == 15 => {
                    // Button up — resets CLICK_ACTIVE
                    serial_println!("[sexinput.synthetic.click_focus.up]");
                    let _ = pdx_call_checked(SLOT_SHELL, OP_USB_MOUSE_REPORT, 0, 0u64, 0u64);
                    synth_click_stage = 3;
                }
                _ => {}
            }
        }

        // 6a. KEYBOARD_EDGE_PROOF_V1: one-shot EV_KEY down+up for Enter (scancode 0x1C).
        //     Available only in the declared synthetic-input proof profile.
        if !SYNTHETIC_INPUT_PROOFS_DISABLED {
            match ev_key_edge_stage {
                0 if tick == 3 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 0x1Cu64, 1, EV_KEY);
                    serial_println!("[sexinput.key.ev_key.down code=0x1c]");
                    ev_key_edge_stage = 1;
                }
                1 if tick == 4 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 0x1Cu64, 0, EV_KEY);
                    serial_println!("[sexinput.key.ev_key.up code=0x1c]");
                    ev_key_edge_stage = 2;
                }
                _ => {}
            }
        }

        // 6. One-shot synthetic keyboard proof for F5/F6 scene settings.
        //    Sends F5 press+release, then second F5 press+release, then F6 press+release
        //    via OP_HID_EVENT EV_KEY.
        //    Bounded: kbd_proof_stage prevents replay after stage 5.
        //    Gate: SEXOS_KEYBOARD_PROOF=1 env var at build time.
        if KEYBOARD_PROOF_ENABLED {
            match kbd_proof_stage {
                // F5 press: scancode 0x3F = 63, EV_KEY, value=1 (pressed)
                0 if tick == 50 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 63, 1, EV_KEY);
                    serial_println!("[sexinput.kbd_proof.f5.down]");
                    kbd_proof_stage = 1;
                }
                // F5 release
                1 if tick == 55 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 63, 0, EV_KEY);
                    serial_println!("[sexinput.kbd_proof.f5.up]");
                    kbd_proof_stage = 2;
                }
                // Second F5 (proves cycle wraps + persist fires again)
                2 if tick == 100 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 63, 1, EV_KEY);
                    kbd_proof_stage = 3;
                }
                3 if tick == 105 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 63, 0, EV_KEY);
                    kbd_proof_stage = 4;
                }
                // F6: scancode 0x40 = 64, EV_KEY, value=1 (pressed)
                4 if tick == 150 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 64, 1, EV_KEY);
                    serial_println!("[sexinput.kbd_proof.f6.down]");
                    kbd_proof_stage = 5;
                }
                5 if tick == 155 => {
                    pdx_call(SLOT_SHELL, OP_HID_EVENT, 64, 0, EV_KEY);
                    serial_println!("[sexinput.kbd_proof.f6.up]");
                    kbd_proof_stage = 6;
                }
                _ => {}
            }
        }

        // Autonomous cursor demo: send OP_AUTODEMO_TICK to shell every 30 ticks.
        // Wakes shell main loop so maybe_run_cursor_autodemo() advances the demo.
        // Gate: CURSOR_AUTODEMO_ENABLED. Bounded: ticks 30..=750 only (24 pings max).
        if CURSOR_AUTODEMO_ENABLED && tick >= 30 && tick <= 750 && (tick - 30) % 30 == 0 {
            pdx_call(SLOT_SHELL, OP_AUTODEMO_TICK, 0, 0, 0);
        }

        sys_yield();
    }
}
