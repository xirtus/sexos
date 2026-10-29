#![no_std]

pub const SILK_MAGIC: u32 = 0x53454C4B;

use sex_pdx::*;

#[repr(C)]
pub struct SilkWindow {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub virt_addr: u64,
    pub pfn_base: u64,
    pub tag_mask: u64,
}

#[repr(C)]
pub struct SexWindowCreateParams {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pfn_base: u64,
}

pub use sex_pdx::{pdx_call, pdx_listen};

pub trait SexApp {
    fn new(pdx: u32) -> Self;
    fn run(&mut self, pdx: u32) -> bool;
}

#[macro_export]
macro_rules! app_main {
    ($app:ty) => {
        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            let mut app = <$app as $crate::SexApp>::new(0);
            loop {
                if !app.run(0) { break; }
            }
            // Exit call (slot 0, opcode 0xFF)
            unsafe { $crate::pdx_call(0, 0xFF, 0, 0, 0); }
            loop {}
        }
    };
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let s1 = core::slice::from_raw_parts(s1, n);
    let s2 = core::slice::from_raw_parts(s2, n);
    for i in 0..n {
        if s1[i] != s2[i] { return s1[i] as i32 - s2[i] as i32; }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // Do not use a copy intrinsic here: LLVM may lower it back to this symbol.
    // Volatile byte accesses keep the freestanding implementation self-contained.
    let mut i = 0;
    while i < n {
        dest.add(i).write_volatile(src.add(i).read_volatile());
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    // Prevent loop recognition from introducing a recursive memset intrinsic.
    let mut i = 0;
    while i < n {
        s.add(i).write_volatile(c as u8);
        i += 1;
    }
    s
}

impl SilkWindow {
    pub fn create(_title: &str, w: u32, h: u32) -> Result<Self, ()> {
        // In SASOS, we don't necessarily need to allocate memory for the buffer here
        // if we use zero-copy, but for now we follow the existing pattern.
        let create_params = SexWindowCreateParams { x: 0, y: 0, width: w, height: h, pfn_base: 0 };

        let (status, window_id) = unsafe {
            pdx_call(SLOT_DISPLAY, OP_WINDOW_CREATE, &create_params as *const _ as u64, 0, 0)
        };

        if status != 0 || window_id == 0 {
            Err(())
        } else {
            Ok(SilkWindow { id: window_id, width: w, height: h, virt_addr: 0, pfn_base: 0, tag_mask: 0 })
        }
    }

    pub fn commit(&self, pfns: &[u64]) -> Result<(), ()> {
        let (status, _) = unsafe {
            pdx_call(SLOT_DISPLAY, OP_WINDOW_SUBMIT, self.id, pfns.as_ptr() as u64, pfns.len() as u64)
        };
        if status == 0 { Ok(()) } else { Err(()) }
    }

    pub fn paint(&self) -> Result<(), ()> {
        // Compatibility shim: TODO: wire to real sexdisplay ABI when PAINT/DESTROY exist
        Ok(())
    }

    pub fn close(&self) -> Result<(), ()> {
        // Compatibility shim: TODO: wire to real sexdisplay ABI when PAINT/DESTROY exist
        Ok(())
    }
}
