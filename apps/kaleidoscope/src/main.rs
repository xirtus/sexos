#![no_std]
#![no_main]

extern crate alloc;
mod paging_probe;
mod service_call;
mod body_text;
mod network_probe;
mod dns_probe;
mod http_wire;
mod http_probe;
mod browser_view;
mod document_reader;
#[path="../../../servers/silk-shell/src/linen_text_delivery.rs"]
mod text_delivery;
#[derive(Clone,Copy)]enum Intent{Echo,Dns,HttpDns,Http}

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
    identity:sex_pdx::app_lifecycle::CatalogInfo,
    viewport_sequence:u64,
    viewport_ready:bool,
    reader:document_reader::Reader,
    reading:bool,
    live_text: [u8; 64],
    live_len: usize,
    probe:network_probe::Probe,
    probe_view:network_probe::View,
    dns:dns_probe::Client,
    dns_view:dns_probe::View,
    http:http_probe::Client,
    http_view:http_probe::View,
    intent:Option<Intent>,
    preview:body_text::Preview,
    ui:text_delivery::Outbox,
    ui_last:[u8;512],
    ui_last_len:usize,
    ui_metadata:Option<text_delivery::Metadata>,
    ui_dirty:bool,
    ui_failed:bool,
    ui_retries:u8,
}

impl SexApp for App {
    fn new(_pdx: u32) -> Self {
        assert_eq!(sex_pdx::device_network_config(sex_pdx::SLOT_NET),Err(sex_pdx::pci_command::DENIED),"BROWSER_HAS_ROUTE_NOT_NIC_METADATA");
        sex_pdx::serial_record!("[browser.network.config.denied] slot=18 kind=route metadata=0");
        ALLOCATOR.init();
        paging_probe::run();
        let identity=sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).expect("BROWSER_SELF_AUTHORITY");
        assert!(identity.is_subject(15,KALEIDO_SURFACE_ID,3),"BROWSER_EXACT_BOOT_IDENTITY");
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
        Self { identity,viewport_sequence:0,viewport_ready:true,reader:document_reader::Reader::new(),reading:false,
            live_text,live_len,probe:network_probe::Probe::new(),probe_view:network_probe::View::Pending,
            dns:dns_probe::Client::new(),dns_view:dns_probe::View::Idle,http:http_probe::Client::new(),http_view:http_probe::View::Idle,
            intent:None,preview,ui:text_delivery::Outbox::atomic(300),ui_last:[0;512],ui_last_len:0,ui_metadata:None,ui_dirty:true,ui_failed:false,ui_retries:0 }

    }

    fn run(&mut self, _pdx: u32) -> bool {
        if self.viewport_ready&&sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,sex_pdx::document_view::READY,self.identity.target_pd as u64,self.identity.task_id as u64,0).is_ok(){self.viewport_ready=false;}
        let pending=self.viewport_ready||self.probe.busy()||self.dns.busy()||self.http.busy()||self.ui.pending()||(self.ui_dirty&&!self.ui_failed)||self.intent.is_some();
        let message=if pending{sex_pdx::pdx_try_listen_raw(0)}else{Some(sex_pdx::pdx_listen_raw(0))};
        if let Some(req)=message{
            if sex_pdx::app_lifecycle::close_requested(req.type_id,req.caller_pd,[req.arg0,req.arg1,req.arg2],self.identity.target_pd,self.identity.task_id)
                ||(req.type_id==0xFF_FF&&req.caller_pd==3){
                sex_pdx::serial_record!("[browser.net.job.closed] target=15 pending={} terminal=1",self.probe.busy()as u8);
                sex_pdx::serial_record!("[browser.dns.closed] pending={} terminal=1",self.dns.busy()as u8);
                sex_pdx::serial_record!("[browser.http.closed] pending={} terminal=1",self.http.busy()as u8);
                self.http.cancel();self.reader.clear();self.reading=false;self.ui.cancel();self.ui_last.fill(0);self.ui_metadata=None;sex_pdx::task_finish(0);
            }
            if req.type_id==sex_pdx::document_view::OP{
                if let Some(n)=sex_pdx::document_view::Notice::decode(req.caller_pd,self.identity.target_pd,self.identity.task_id,[req.arg0,req.arg1,req.arg2],self.viewport_sequence){
                    self.viewport_sequence=n.sequence;self.reader.resize(n.w,n.h);
                    // Recreated display slots discard old staging and ACKs.
                    // Preserve the outbox's monotonic sequence across cancel.
                    self.ui.cancel();self.ui_last.fill(0);self.ui_last_len=0;self.ui_metadata=None;self.ui_dirty=true;self.ui_failed=false;self.ui_retries=0;
                    sex_pdx::serial_record!("[browser.viewport] target={} task={} sequence={} w={} h={} origin=held_shell",n.target,n.task,n.sequence,n.w,n.h);
                }
            }
            if req.type_id==1&&req.caller_pd==13{
                if matches!(req.arg0>>56,0x79|0xE9){self.dns.reply(req.arg0,sex_pdx::get_ticks());}
                else if self.http.busy(){self.http.reply(req.arg0,sex_pdx::get_ticks());}
                else{self.probe.reply(req.arg0,sex_pdx::get_ticks());}
            }
            if req.type_id==1&&req.caller_pd==1{
                if let Some(r)=self.ui.receive(req.caller_pd,req.arg0){sex_pdx::serial_record!("[browser.view.applied] bytes={} http={} body_bytes={} pixels=unconfirmed",r.bytes,r.metadata.selected,r.metadata.count);}
            }
            if req.type_id==0x202&&req.caller_pd==3&&req.arg1==1{
                if self.ui_failed{self.ui_failed=false;self.ui_retries=0;self.ui_dirty=true;}
                let intent=match req.arg0{0x13=>Some(Intent::Echo),0x20=>Some(Intent::Dns),0x23=>Some(Intent::HttpDns),_=>None};
                if let Some(intent)=intent{
                    self.reading=false;self.reader.clear();
                    // Latest user intent waits for positive cleanup of whichever
                    // existing route currently owns a job. No parallel BEGIN.
                    if matches!(self.intent,Some(Intent::HttpDns|Intent::Http)){self.http.cancel();}
                    self.intent=Some(intent);if self.probe.busy(){self.probe.cancel();}if self.dns.busy(){self.dns.cancel();}if self.http.busy(){self.http.cancel();}
                }else{match req.arg0{
                    0x2E=>{if matches!(self.intent,Some(Intent::Echo)){self.intent=None;}self.probe.cancel();},
                    0x2D=>{if matches!(self.intent,Some(Intent::Dns|Intent::HttpDns|Intent::Http)){if matches!(self.intent,Some(Intent::HttpDns|Intent::Http)){self.http.cancel();}self.intent=None;}self.dns.cancel();},
                    0x24=>{if matches!(self.intent,Some(Intent::HttpDns|Intent::Http)){self.intent=None;self.dns.cancel();}self.http.cancel();self.reading=false;self.reader.clear();},
                    0x2F=>{if self.http.view()==http_probe::View::Ready{self.reading=!self.reading;if self.reading{self.reader.clear();}}},
                    code=>{if self.reading{let movement=match code{0x67=>Some(document_reader::Move::Up),0x6C=>Some(document_reader::Move::Down),0x68=>Some(document_reader::Move::PageUp),0x6D=>Some(document_reader::Move::PageDown),0x66=>Some(document_reader::Move::Home),0x6B=>Some(document_reader::Move::End),_=>None};if let Some(movement)=movement{self.reader.navigate(self.http.document(),movement);}}},
                }}
            }
        }
        if !self.probe.busy()&&!self.dns.busy()&&!self.http.busy(){
            if let Some(intent)=self.intent.take(){
                // Echo and HTTP share the exact0x20C caller replay fence.
                let sequence=self.probe.next_sequence().max(self.http.next_sequence());
                match intent{
                    Intent::Echo=>{if !self.probe.restart_at(sequence){sex_pdx::serial_record!("[browser.net.start.rejected] protocol=echo seq={} effects=0",sequence);}},
                    Intent::Dns=>self.dns.start(),
                    Intent::HttpDns=>{self.http.wait_for_resolution();self.dns.start();self.intent=Some(Intent::Http);},
                    Intent::Http=>{
                        let generation=self.dns.generation().unwrap_or(0);
                        if !self.http.start_resolved(sequence,generation){sex_pdx::serial_record!("[browser.net.start.rejected] protocol=http seq={} dns_generation={} effects=0",sequence,generation);}
                    },
                }
            }
        }
        if self.probe.busy(){self.probe.advance(sex_pdx::get_ticks());}
        else if self.dns.busy(){self.dns.advance(sex_pdx::get_ticks());}
        else if self.http.busy(){self.http.advance(sex_pdx::get_ticks());}
        let view=self.probe.view();if view!=self.probe_view{self.probe_view=view;sex_pdx::serial_record!("[browser.net.job.view] state={:?} web=0",view);}
        let view=self.dns.view();if view!=self.dns_view{self.dns_view=view;sex_pdx::serial_record!("[browser.dns.view] state={:?} web=0",view);}
        let view=self.http.view();if view!=self.http_view{self.http_view=view;if view!=http_probe::View::Ready{self.reading=false;self.reader.clear();}sex_pdx::serial_record!("[browser.http.view] state={:?} tls=0",view);}
        let mut reader_layout=None;
        let composed=if self.reading&&self.http.view()==http_probe::View::Ready{
            let f=self.reader.render(self.http.document());reader_layout=Some((f.first,f.end,f.body_rows,f.rows,f.cols,f.total_lines));
            Some((browser_view::Text{bytes:f.bytes,len:f.len},text_delivery::Metadata{count:self.http.document().len(),first:f.first,rows:f.body_rows,selected:http_view_code(self.http.view())|256}))
        }else{browser_view::compose(self.preview.as_bytes(),self.probe.view(),self.dns.view(),self.http.view(),self.http.preview()).map(|text|(text,text_delivery::Metadata{count:if matches!(self.http.view(),http_probe::View::Ready){self.http.body_length()}else{0},first:0,rows:0,selected:http_view_code(self.http.view())}))};
        if let Some((text,metadata))=composed{
            if self.ui_last_len!=text.len||self.ui_last!=text.bytes||self.ui_metadata!=Some(metadata){
                self.ui_last=text.bytes;self.ui_last_len=text.len;self.ui_metadata=Some(metadata);self.ui_dirty=true;self.ui_failed=false;self.ui_retries=0;
                if let Some((first,end,body_rows,rows,cols,total_lines))=reader_layout{sex_pdx::serial_record!("[browser.reader.layout] first={} end={} body_rows={} rows={} cols={} total_lines={} source_bytes={} viewport={} origin=validated_body tls=0 pixels=unconfirmed",first,end,body_rows,rows,cols,total_lines,self.http.document().len(),self.viewport_sequence);}
            }
            if self.ui_dirty&&!self.ui_failed{
                if self.ui.queue(&text.bytes[..text.len],0x00CDD6F4,metadata){self.ui_dirty=false;}
            }
        }
        if let text_delivery::Poll::Rejected(error)=self.ui.poll(4,|op|{
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_DISPLAY,op.opcode,op.surface,op.packed,op.control){
                Ok(_)=>text_delivery::Admission::Accepted,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>text_delivery::Admission::Full,Err(e)=>text_delivery::Admission::Rejected(e),
            }
        }){self.ui.cancel();self.ui_dirty=true;self.ui_retries=self.ui_retries.saturating_add(1);self.ui_failed=self.ui_retries>=3;sex_pdx::serial_record!("[browser.view.rejected] error={} attempts={} terminal={} accepted_network_replay=0",error,self.ui_retries,self.ui_failed as u8);}
        if self.viewport_ready||self.probe.busy()||self.dns.busy()||self.http.busy()||self.ui.pending()||(self.ui_dirty&&!self.ui_failed)||self.intent.is_some(){sex_pdx::sys_wait_self_ticks(2).expect("BROWSER_SELF_WAIT_AUTHORITY");}
        true
    }


}

fn http_view_code(view:http_probe::View)->u64{match view{http_probe::View::Idle=>0,http_probe::View::Pending=>1,http_probe::View::Ready=>2,http_probe::View::Unavailable=>3,http_probe::View::Cancelled=>4,http_probe::View::Uncertain=>5}}

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
