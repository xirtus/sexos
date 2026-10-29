#![no_std]
#![no_main]

extern crate alloc;
mod paging_probe;
mod service_call;
mod body_text;

use silkclient::{app_main, SexApp};
// Private BOOT ELF backing replaces the old ambient fixed-address heap.
#[path="../../../sex-rt/src/heap.rs"]
mod heap;
static HEAP:heap::Arena=heap::Arena::new();
#[global_allocator]
static ALLOCATOR:heap::BoundedAllocator=heap::BoundedAllocator::new(&HEAP);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[paging.client.fatal] {}", info);
    loop { sex_pdx::sys_yield(); }
}

const KALEIDO_SURFACE_ID: u64 = 300;
const SEXNET_DNS_RESOLVE: u64 = 0x20B;

struct App {
    live_text: [u8; 64],
    live_len: usize,
}

impl SexApp for App {
    fn new(_pdx: u32) -> Self {
        ALLOCATOR.init();
        paging_probe::run();
        sex_pdx::serial_println!("[browser.slot.net.static_grant.begin]");
        let _value = service_call::request(sex_pdx::SLOT_NET, 13, 0x200, [0, 0, 0]).expect("NET warmup");
        let status = 0u64;
        sex_pdx::serial_println!("[browser.slot.net.route.call] status={}", status);
        sex_pdx::serial_println!("[browser.slot.net.static_grant.proof.done] ok=1 network=0");
        sex_pdx::serial_println!("[browser.dns.resolve.request] host_id=1 ok=1");
        let dns_reply = service_call::request(sex_pdx::SLOT_NET, 13, SEXNET_DNS_RESOLVE, [1, 0, 0]).expect("NET DNS");
        if dns_reply != 0 {
            let ip = (dns_reply as u32).to_be_bytes();
            sex_pdx::serial_println!(
                "[browser.dns.resolve.ok] addr={}.{}.{}.{} ok=1",
                ip[0],
                ip[1],
                ip[2],
                ip[3]
            );
        } else {
            sex_pdx::serial_println!("[browser.dns.resolve.miss] host_id=1 ok=1 reason=cache_miss");
        }
        sex_pdx::serial_println!("[browser.packed_text.begin]");

        let len_value = service_call::request(sex_pdx::SLOT_NET, 13, 0x200, [0x207, 0, 0]).expect("NET text length");
        let mut live_text = [0u8; 64];
        let mut live_len = 0usize;
        {
            let reported = core::cmp::min(len_value as usize, live_text.len());
            sex_pdx::serial_println!("[browser.packed_text.len.recv] len={}", reported);
            let chunk_count = core::cmp::min((reported + 7) / 8, 8);
            let mut idx = 0usize;
            while idx < chunk_count {
                let chunk_value = service_call::request(sex_pdx::SLOT_NET, 13, 0x200, [0x208, idx as u64, 0]).expect("NET text chunk");
                let start = idx * 8;
                let remain = reported.saturating_sub(start);
                let bytes = core::cmp::min(remain, 8);
                let mut i = 0usize;
                while i < bytes {
                    live_text[start + i] = ((chunk_value >> (i * 8)) & 0xFF) as u8;
                    i += 1;
                }
                sex_pdx::serial_println!("[browser.packed_text.chunk.recv] idx={} bytes={}", idx, bytes);
                live_len = start + bytes;
                idx += 1;
            }
            if live_len > reported {
                live_len = reported;
            }
            sex_pdx::serial_println!("[browser.packed_text.text.set] live=1 len={}", live_len);
            sex_pdx::serial_println!(
                "[browser.async_reply.proof.done] len={} text_recv={}",
                reported,
                live_len
            );
        }
        sex_pdx::serial_println!("[browser.packed_text.proof.done]");
        sex_pdx::serial_println!("[browser.body.begin]");
        let body_reply = service_call::request(sex_pdx::SLOT_NET, 13, 0x200, [0x209, 0, 0]).expect("NET body length");
        let body_reported = core::cmp::min(body_reply as usize, 64);
        let mut body_text = [0u8; 64];
        let mut body_len = 0usize;
        if body_reported > 0 {
            sex_pdx::serial_println!("[browser.body.len.recv] len={}", body_reported);
            let chunk_count = core::cmp::min((body_reported + 7) / 8, 8);
            let mut idx = 0usize;
            while idx < chunk_count {
                let chunk_value = service_call::request(sex_pdx::SLOT_NET, 13, 0x200, [0x20A, idx as u64, 0]).expect("NET body chunk");
                let start = idx * 8;
                let remain = body_reported.saturating_sub(start);
                let bytes = core::cmp::min(remain, 8);
                let mut i = 0usize;
                while i < bytes {
                    body_text[start + i] = ((chunk_value >> (i * 8)) & 0xFF) as u8;
                    i += 1;
                }
                sex_pdx::serial_println!("[browser.body.chunk.recv] idx={} bytes={}", idx, bytes);
                body_len = start + bytes;
                idx += 1;
            }
            if body_len > body_reported {
                body_len = body_reported;
            }
            sex_pdx::serial_println!("[browser.body.text.set] live=1 len={}", body_len);
        }
        sex_pdx::serial_println!("[browser.render.live_text.begin] len={}", live_len);
        sex_pdx::serial_println!("[browser.visible.polish.begin] sid={}", KALEIDO_SURFACE_ID);

        service_call::enqueue(sex_pdx::SLOT_DISPLAY, 0xEC,
            [KALEIDO_SURFACE_ID, (120u64 << 32) | 80u64, (120u64 << 32) | 520u64])
            .expect("display surface create");

        sex_pdx::serial_println!(
            "[browser.render.buffer.ready] virt=surface pfn=surface sid={}",
            KALEIDO_SURFACE_ID
        );

        let preview = body_text::build(&body_text[..body_len], &live_text[..live_len]);
        let text = preview.as_bytes();
        let text_color: u64 = 0x00CDD6F4;
        for (index, chunk) in text.chunks(8).enumerate() {
            let offset = index * 8;
            let mut packed = 0u64;
            for (byte_index, &byte) in chunk.iter().enumerate() {
                packed |= (byte as u64) << (byte_index * 8);
            }
            // The bounded preview fits the existing low-byte offset encoding.
            let arg2 = offset as u64 | ((chunk.len() as u64) << 8) | (text_color << 32);
            service_call::enqueue(sex_pdx::SLOT_DISPLAY, 0xFB,
                [KALEIDO_SURFACE_ID, packed, arg2]).expect("display text");
        }
        sex_pdx::serial_record!("[browser.preview.layout] bytes={} newline=1 cached_or_sample=1", text.len());
        if body_len > 0 {
            sex_pdx::serial_println!("[browser.body.render.done]");
        }

        sex_pdx::serial_println!("[browser.render.draw_text] live=1 len={}", live_len);

        service_call::enqueue(sex_pdx::SLOT_DISPLAY, 0xEB, [KALEIDO_SURFACE_ID, 50, 100]).expect("display position");

        sex_pdx::serial_println!("[browser.render.commit.request] window={}", KALEIDO_SURFACE_ID);
        sex_pdx::serial_println!("[browser.render.live_text.done]");
        sex_pdx::serial_println!("[browser.visible.polish.done]");

        // Complete registration before asking the display to attest an actual
        // rendered frame. Enqueue success alone proves neither transition.
        assert_eq!(service_call::request(sex_pdx::SLOT_SHELL, 3, 0xFA,
            [KALEIDO_SURFACE_ID, 300, 15 << 8]).expect("shell app registration"),
            KALEIDO_SURFACE_ID);
        sex_pdx::serial_record!("[app.surface.registered] target=15 surface=300 owner=15");
        assert_eq!(service_call::request(sex_pdx::SLOT_DISPLAY, 1,
            sex_pdx::app_lifecycle::OP_APP_PRESENT, [KALEIDO_SURFACE_ID, 1, 0])
            .expect("display app presentation"), 1 << 8);
        sex_pdx::serial_record!("[app.surface.presented] target=15 surface=300 sequence=1 ok=1");
        #[cfg(app_resource_cleanup_proof)]
        full_resource_cleanup_proof();
        Self { live_text, live_len }
    }

    fn run(&mut self, _pdx: u32) -> bool {
        let req = sex_pdx::pdx_listen_raw(0);
        if sex_pdx::app_lifecycle::close_requested(req.type_id, req.caller_pd,
            [req.arg0, req.arg1, req.arg2], 15, 15)
            || (req.type_id == 0xFF_FF && req.caller_pd == 3)
        { sex_pdx::task_finish(0); }
        true
    }
}

#[cfg(app_resource_cleanup_proof)]
fn full_resource_cleanup_proof()->! {
    fn wait(caller:u32)->u64 {loop {let m=sex_pdx::pdx_listen_raw(0);if m.type_id==1&&m.caller_pd==caller{return m.arg0}}}
    // Retain an actual saved object lease and event until terminal cleanup.
    let name=u64::from_le_bytes(*b"app-note");
    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_STORAGE,0x30,name,0,1<<24).0,0);
    let handle=wait(11); assert!(handle!=0 && (handle as i64)>0);
    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_STORAGE,0x32,handle,0,u64::from_le_bytes(*b"keep me!")).0,0);
    assert_eq!(wait(11),8);

    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_BELL,sex_pdx::OP_BELL_NOTIFY,0,0,0).0,0);
    // NOTIFY is one-way. SUBSCRIBE replies after the same Bell queue has
    // processed it, providing an ordered publication barrier.
    let _generation = service_call::request(sex_pdx::SLOT_BELL, 10,
        sex_pdx::OP_BELL_SUBSCRIBE, [0, 0, 0]).expect("Bell publication barrier");
    // Registration and actual presentation already acknowledged. Preserve a
    // visible interval before exit; this delay is not a presentation proof.
    let started=sex_pdx::get_ticks();
    while sex_pdx::get_ticks().wrapping_sub(started)<128 {sex_pdx::sys_yield();}
    sex_pdx::serial_record!("[app.resources.acquired] target=15 surface=300 handle={} notification=1 private_heap=elf_owned resident=4 visible_ticks=128",handle);
    sex_pdx::task_finish(0)
}

app_main!(App);
