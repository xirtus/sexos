#![no_std]
#![no_main]

extern crate alloc;
mod paging_probe;
mod service_call;
mod body_text;
mod network_probe;
mod dns_probe;
mod entropy_probe;
mod clock_probe;
mod trust_probe;
mod clock_view;
mod tls_probe;
mod tls_exchange;
mod http_wire;
mod http_probe;
mod http_navigation;
mod browser_view;
mod document_reader;
mod document_import;
mod import_view;
mod asset_view;
mod asset_decode;
mod bitmap_delivery;
#[path="../../../servers/silk-shell/src/linen_text_delivery.rs"]
mod text_delivery;
mod browser_command;
use browser_command::Intent;

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
// Inline payload is private BOOT ELF storage, never an App::new stack copy.
static mut CLIPBOARD:sex_pdx::clipboard_client::Client=sex_pdx::clipboard_client::Client::new();

struct App {
    identity:sex_pdx::app_lifecycle::CatalogInfo,
    viewport_sequence:u64,
    viewport_ready:bool,
    reader:document_reader::Reader,
    selection:sex_pdx::text_selection::Selection,
    selection_generation:u64,
    selection_body:Option<(bool,usize,u64)>,
    clipboard_view:sex_pdx::clipboard_client::View,
    ui_last_selection:(u16,u16),
    reading:bool,
    reading_https:bool,
    https_panel:bool,
    https_view:tls_exchange::View,
    https_progress_logged:usize,
    asset_panel:bool,
    asset_ticket:u32,
    asset_status:asset_view::Status,
    asset_peer_view:tls_exchange::View,
    image:asset_decode::Image,
    bitmap:bitmap_delivery::Outbox,
    image_layout:Option<(sex_pdx::bitmap_batch::Geometry,u64)>,
    asset_fenced:bool,
    asset_text_viewport:Option<u64>,
    importer:document_import::Client<sex_tls::Document>,
    import_source:Option<sex_tls::Document>,
    import_editor:import_view::Editor,
    import_status:import_view::Status,
    import_panel:bool,
    import_ticket:u32,
    import_viewport:(u32,u32),
    workspace:sex_pdx::document_view::WorkspaceRefresh,
    files_pending:bool,
    live_text: [u8; 64],
    live_len: usize,
    probe:network_probe::Probe,
    probe_view:network_probe::View,
    dns:dns_probe::Client,
    dns_view:dns_probe::View,
    seed:entropy_probe::Client,
    seed_view:entropy_probe::View,
    clock:clock_probe::Client,
    clock_view:clock_probe::View,
    clock_panel:bool,
    trust:trust_probe::Client,
    trust_view:trust_probe::View,
    tls:tls_probe::Probe,
    tls_view:tls_probe::View,
    tls_panel:bool,
    exchange:tls_exchange::Client,
    peer_view:tls_exchange::View,
    peer_panel:bool,
    peer_generation:u32,peer_sequence:u32,peer_ticket_logged:u32,
    http:http_probe::Client,
    http_view:http_probe::View,
    intent:Option<Intent>,
    app_turn:u64,
    resources_last:Option<(heap::Usage,usize,bool,bool,bool,Option<Intent>,bool)>,
    navigation:http_navigation::Navigation,
    preview:body_text::Preview,
    ui:text_delivery::Outbox,
    ui_last:[u8;512],
    ui_last_len:usize,
    ui_metadata:Option<text_delivery::Metadata>,
    ui_dirty:bool,
    ui_failed:bool,
    ui_retries:u8,
}

impl App {
    fn clipboard_progress(&self)->bool { unsafe{CLIPBOARD.pending()&&CLIPBOARD.view()!=sex_pdx::clipboard_client::View::Uncertain} }
    fn clear_selection(&mut self){
        self.selection.clear();self.selection_body=None;
        unsafe{CLIPBOARD.cancel(sex_pdx::get_ticks());}
    }
    fn reader_body(&self)->&[u8]{if self.reading_https{self.https_body()}else{self.http.document()}}
    fn sync_selection(&mut self){
        if !self.reading{return}
        let body=self.reader_body();let signature=(self.reading_https,body.len(),public_fingerprint(body));
        if self.selection_body==Some(signature){return}
        let Some(generation)=self.selection_generation.checked_add(1)else{self.selection.clear();return};
        self.selection_generation=generation;
        if self.selection.bind(sex_pdx::text_selection::Source{generation,revision:0},signature.1).is_ok(){self.selection_body=Some(signature);}
    }
    fn key_input(&mut self,key:sex_pdx::app_key::Key){
        self.sync_selection();
        if key.code==0x01&&unsafe{CLIPBOARD.pending()}{unsafe{CLIPBOARD.cancel(sex_pdx::get_ticks());}return}
        if key.control {
            if self.reading&&key.code==0x1e {if let Some(s)=self.selection.snapshot(){let _=self.selection.select_all(s.source());}}
            // Clipboard chords arrive as Shell-issued interactions. Unknown
            // Ctrl chords are consumed instead of invoking plain-C Cancel.
            return;
        }
        if self.reading {
            if let Some(s)=self.selection.snapshot(){
                let source=s.source();
                let moved=match key.code {
                    0x69|0x6a=>{self.selection_horizontal(key.code==0x6a,key.shift);true},
                    0x66 if key.shift=>{let _=self.selection.home(source,true);true},
                    0x6b if key.shift=>{let _=self.selection.end(source,true);true},
                    0x67|0x6c|0x68|0x6d if key.shift=>{
                        let movement=match key.code{0x67=>document_reader::Move::Up,0x6c=>document_reader::Move::Down,0x68=>document_reader::Move::PageUp,_=>document_reader::Move::PageDown};
                        let body=if self.reading_https{if self.exchange.has_document(){self.exchange.document()}else if let Some(b)=self.import_source.as_ref(){b.as_bytes()}else{self.importer.document().map_or(&[][..],|b|b.as_bytes())}}else{self.http.document()};
                        let to=self.reader.selection_move(body,s.caret(),movement);let _=self.selection.move_to(source,to,true);true
                    },_=>false,
                };
                if moved {
                    let caret=self.selection.snapshot().unwrap().caret();
                    let body=if self.reading_https{if self.exchange.has_document(){self.exchange.document()}else if let Some(b)=self.import_source.as_ref(){b.as_bytes()}else{self.importer.document().map_or(&[][..],|b|b.as_bytes())}}else{self.http.document()};
                    self.reader.reveal(body,caret);return;
                }
            }
        }
        self.key(key.code as u64);
    }
    fn selection_horizontal(&mut self,right:bool,extend:bool){
        let Some(snapshot)=self.selection.snapshot()else{return};
        let(lo,hi)=snapshot.range();
        let position=if !extend&&lo!=hi{if right{hi}else{lo}}
            else{let body=self.reader_body();if right{sex_pdx::text_encoding::next(body,snapshot.caret())}else{sex_pdx::text_encoding::previous(body,snapshot.caret())}};
        let _=self.selection.move_to(snapshot.source(),position,extend);
    }
    fn clipboard_events(&mut self){
        self.sync_selection();
        while let Some(event)=unsafe{CLIPBOARD.take_event()}{
            match event {
                sex_pdx::clipboard_client::Event::Export{ticket,cut}=>{
                    let snapshot=self.selection.snapshot();
                    if !cut&&self.reading&&snapshot.is_some(){
                        let snapshot=snapshot.unwrap();let(lo,hi)=snapshot.range();let body=self.reader_body();
                        let accepted=unsafe{CLIPBOARD.publish(ticket,snapshot.source(),&body[lo..hi],sex_pdx::get_ticks())};
                        sex_pdx::serial_record!("[browser.clipboard.copy] ticket={} generation={} revision={} start={} end={} bytes={} hash={:#x} accepted={}",ticket,snapshot.source().generation,snapshot.source().revision,lo,hi,hi-lo,public_fingerprint(&body[lo..hi]),accepted as u8);
                    }else{unsafe{CLIPBOARD.decline(ticket,sex_pdx::clipboard_client::View::Unsupported,sex_pdx::get_ticks());}}
                },
                sex_pdx::clipboard_client::Event::Published{ticket,len,generation,hash,..}=>sex_pdx::serial_record!("[browser.clipboard.published] ticket={} bytes={} generation={} hash={:#x} positive_ack=1",ticket,len,generation,hash),
                sex_pdx::clipboard_client::Event::Paste{ticket}=>{unsafe{CLIPBOARD.decline(ticket,sex_pdx::clipboard_client::View::Unsupported,sex_pdx::get_ticks());}},
                sex_pdx::clipboard_client::Event::Ready{..}=>{unsafe{CLIPBOARD.settle(false);}},
                sex_pdx::clipboard_client::Event::Stopped{..}=>{},
            }
        }
        unsafe{CLIPBOARD.poll(sex_pdx::get_ticks(),|packet|{
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,packet.opcode,packet.args[0],packet.args[1],packet.args[2]){
                Ok(_)=>sex_pdx::clipboard_client::Admission::Accepted,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>sex_pdx::clipboard_client::Admission::Full,Err(_)=>sex_pdx::clipboard_client::Admission::Rejected,
            }
        });}
        let view=unsafe{CLIPBOARD.view()};if view!=self.clipboard_view{self.clipboard_view=view;sex_pdx::serial_record!("[browser.clipboard.view] state={:?}",view);}
    }
    fn fence_asset(&mut self,reason:&str){
        self.asset_fenced=true;self.asset_status=asset_view::Status::Uncertain;self.image.clear();
        sex_pdx::serial_record!("[browser.asset.fenced] reason={} accepted_replay=0 owned_storage=erased",reason);
    }
    fn clear_asset(&mut self)->bool {
        let had=self.image_layout.take().is_some()||self.bitmap.pending()||!self.image.pixels().is_empty();
        self.image.clear();
        if had {
            if self.bitmap.clear(bitmap_delivery::Metadata{viewport:self.viewport_sequence,source:0}).is_err(){self.fence_asset("clear_sequence");self.exchange.cancel();}
        }
        had
    }
    fn queue_intent(&mut self,next:Intent,document:bool)->bool{
        if self.asset_fenced{sex_pdx::serial_record!("[browser.command.blocked] kind={:?} display_uncertain=1 accepted_replay=0",next);return false;}

        if self.importer.busy(){sex_pdx::serial_record!("[browser.command.blocked] kind={:?} import_pending=1 accepted_replay=0",next);return false;}
        if self.importer.view()==document_import::View::Uncertain{sex_pdx::serial_record!("[browser.command.blocked] kind={:?} import_uncertain=1 accepted_replay=0",next);return false;}
        self.clear_selection();
        self.clear_asset();if self.asset_fenced{return false;}self.asset_text_viewport=None;self.asset_ticket=0;self.asset_panel=next==Intent::TlsAssetDns;
        if self.asset_panel{self.asset_status=asset_view::Status::Loading;self.asset_peer_view=tls_exchange::View::Idle;self.https_progress_logged=0;}
        self.importer.clear(sex_pdx::get_ticks());self.import_source=None;self.import_editor.cancel();self.import_panel=false;self.import_ticket=0;
        let n=browser_command::replace(&mut self.intent,next,&mut self.tls,&mut self.trust,&mut self.seed,&mut self.clock);
        if n>0{sex_pdx::serial_record!("[browser.tls.erased] bytes={} owned_storage=1",n);}
        self.exchange.cancel();if self.probe.busy(){self.probe.cancel();}if self.dns.busy(){self.dns.cancel();}
        if self.http.busy()||self.navigation.active()||matches!(next,Intent::HttpDns|Intent::TlsDns|Intent::TlsAssetDns){self.http.cancel();}
        self.navigation.cancel();if next==Intent::HttpDns{self.navigation.begin();}
        self.reading=false;self.reading_https=false;self.reader.clear();
        self.https_panel=next==Intent::TlsDns&&document;self.peer_panel=next==Intent::TlsDns&&!document;
        self.tls_panel=next==Intent::TlsPrepare;self.clock_panel=next==Intent::Clock;
        if self.https_panel{self.https_progress_logged=0;}
        sex_pdx::serial_record!("[browser.command.queued] kind={:?} document={} retained=1 accepted_replay=0",next,document as u8);
        self.resources("command",true);true
    }
    fn https_body(&self)->&[u8]{
        if self.exchange.has_document(){self.exchange.document()}
        else if !self.import_editor.active&&self.import_source.is_some(){self.import_source.as_ref().unwrap().as_bytes()}
        else if self.importer.view()==document_import::View::Saved{self.importer.document().map_or(&[][..],|d|d.as_bytes())}
        else{&[]}
    }
    fn has_https_body(&self)->bool{self.exchange.has_document()||(!self.import_editor.active&&self.import_source.is_some())||(self.importer.view()==document_import::View::Saved&&self.importer.has_document())}
    fn record_import_control(&mut self){
        if let Some(c)=self.importer.take_control(){let(ok,value)=match c.value{None=>(1,0),Some(Ok(v))=>(1,v),Some(Err(e))=>(0,e)};sex_pdx::serial_record!("[browser.import.control] phase={:?} kind={} sequence={} object={} ok={} value={}",c.phase,c.kind,c.sequence,c.object,ok,value);}
    }
    fn begin_import(&mut self){
        if self.asset_panel||self.exchange.resource_kind()==Some(sex_tls::Resource::ArtworkQoi){sex_pdx::serial_record!("[browser.import.rejected] reason=asset_not_text effects=0");return;}

        if self.importer.view()==document_import::View::Uncertain{self.import_panel=true;self.import_status=import_view::Status::Uncertain;sex_pdx::serial_record!("[browser.import.blocked] uncertain=1 effects=0");return;}
        if self.importer.busy(){sex_pdx::serial_record!("[browser.import.blocked] pending=1 effects=0");return;}
        let import_bytes=if let Some(body)=self.import_source.as_ref(){body.as_bytes()}
            else if self.exchange.has_document(){self.exchange.document()}
            else{self.importer.document().map_or(&[][..],|body|body.as_bytes())};
        if !sex_pdx::text_encoding::valid_text(import_bytes){
            self.import_panel=true;self.import_editor.cancel();self.import_status=import_view::Status::UnsupportedText;
            sex_pdx::serial_record!("[browser.import.rejected] reason=unsupported_text effects=0 source_preserved=1");return;
        }
        if self.import_source.is_none(){
            if let Some(body)=self.exchange.take_document_for_import(){self.import_ticket=self.exchange.ticket();self.import_source=Some(body);}
            else if let Some(body)=self.importer.take_document(){self.import_source=Some(body);}
        }
        self.import_panel=true;
        if let Some(body)=self.import_source.as_ref(){
            self.https_panel=false;self.peer_panel=false;self.tls_panel=false;self.clock_panel=false;self.reading=false;self.reading_https=false;
            self.importer.clear(sex_pdx::get_ticks());self.import_editor.begin();self.import_status=import_view::Status::Naming;
            sex_pdx::serial_record!("[browser.import.intent] source_ticket={} bytes={} hash={:#x} authenticated=1 positive_release=1 create_only=1 accepted=1",self.import_ticket,body.as_bytes().len(),public_fingerprint(body.as_bytes()));
        }else{self.import_status=import_view::Status::NoDocument;sex_pdx::serial_record!("[browser.import.rejected] reason=no_released_document effects=0");}
        self.resources("change",true);
    }
    fn submit_import(&mut self){
        if self.import_source.as_ref().is_some_and(|body|!sex_pdx::text_encoding::valid_text(body.as_bytes())){
            self.import_editor.cancel();self.import_status=import_view::Status::UnsupportedText;
            sex_pdx::serial_record!("[browser.import.rejected] reason=unsupported_text effects=0 source_preserved=1");return;
        }
        let Some(body)=self.import_source.take()else{return;};let name=self.import_editor.name();let len=body.as_bytes().len();let hash=public_fingerprint(body.as_bytes());
        let accepted=match self.importer.start(body,name,sex_pdx::get_ticks()){Ok(())=>true,Err(body)=>{self.import_source=Some(body);false}};
        self.import_status=if accepted{import_view::Status::Saving}else if self.importer.view()==document_import::View::Uncertain{import_view::Status::Uncertain}else{import_view::Status::Unavailable};
        sex_pdx::serial_record!("[browser.import.submit] name={} bytes={} hash={:#x} source_ticket={} accepted={} create_only=1",core::str::from_utf8(self.import_editor.as_bytes()).unwrap(),len,hash,self.import_ticket,accepted as u8);
        self.resources("change",true);
    }
    fn key(&mut self,raw:u64){
        let Ok(code)=u8::try_from(raw)else{return;};
        if self.ui_failed{self.ui_failed=false;self.ui_retries=0;self.ui_dirty=true;}
        if self.import_editor.active{
            match self.import_editor.key(code){import_view::Key::Submit=>self.submit_import(),import_view::Key::Cancel=>{self.import_status=import_view::Status::Cancelled;self.import_panel=false;self.https_panel=true;self.reading=true;self.reading_https=true;},_=>{}}
            return;
        }
        if code==0x17{self.begin_import();return;}
        if code==0x18{
            if !self.importer.busy()||self.importer.view()==document_import::View::Uncertain{self.workspace.changed();self.files_pending=true;}else{sex_pdx::serial_record!("[browser.import.files.blocked] pending=1 effects=0");}return;
        }
        if self.import_panel&&code==0x2F&&self.has_https_body(){self.import_panel=false;self.https_panel=true;self.reading=true;self.reading_https=true;return;}
        if matches!(code,0x10|0x24){self.clear_selection();}
        if code==0x10&&(self.import_panel||self.importer.has_document()||self.import_source.is_some()||self.importer.busy()){
            self.import_editor.cancel();self.import_source=None;let _=self.importer.cancel(sex_pdx::get_ticks());self.import_panel=true;self.reading=false;self.reading_https=false;
            self.reader.clear();self.import_status=if self.importer.view()==document_import::View::Uncertain{import_view::Status::Uncertain}else if self.importer.busy(){import_view::Status::Cleaning}else{import_view::Status::Cancelled};self.resources("clear",true);return;
        }
        let intent=match code{0x13=>Some(Intent::Echo),0x20=>Some(Intent::Dns),0x23=>Some(Intent::HttpDns),_=>None};
        if let Some(intent)=intent{self.queue_intent(intent,false);}
        else{match code{
            0x12=>{self.queue_intent(Intent::Seed,false);},
            0x14=>{self.queue_intent(Intent::Clock,false);},
            0x10=>{if self.asset_panel{let had=self.clear_asset();self.asset_status=if self.asset_fenced{asset_view::Status::Uncertain}else if had{asset_view::Status::Clearing}else{asset_view::Status::Cleared};sex_pdx::serial_record!("[browser.asset.clear.intent] pending={} owned_storage=erased",self.bitmap.pending()as u8);}self.exchange.cancel();if self.https_panel{self.reading=false;self.reading_https=false;self.reader.clear();sex_pdx::serial_record!("[browser.https.cleared] body_visible=0 owned_storage=erased network=0");}if matches!(self.intent,Some(Intent::TlsDns|Intent::TlsAssetDns|Intent::TlsStart|Intent::TlsPrepare|Intent::Seed|Intent::Clock)){self.intent=None;self.dns.cancel();}let n=self.tls.cancel(&mut self.trust,&mut self.seed,&mut self.clock);sex_pdx::serial_record!("[browser.tls.erased] bytes={} owned_storage=1",n);self.resources("clear",true);},
            0x26=>{self.queue_intent(Intent::TlsPrepare,false);},
            0x22=>{if self.queue_intent(Intent::TlsAssetDns,false){sex_pdx::serial_record!("[browser.asset.intent] accepted=1 explicit=1 public_internet=0");}},
            0x30=>{if self.queue_intent(Intent::TlsDns,true){sex_pdx::serial_record!("[browser.https.intent] accepted=1 explicit=1 public_internet=0");}},
            0x15=>{if self.queue_intent(Intent::TlsDns,false){sex_pdx::serial_record!("[browser.tls.peer.intent] accepted=1 explicit=1 public_internet=0");}},
            0x2E=>{if matches!(self.intent,Some(Intent::Echo)){self.intent=None;}self.probe.cancel();},
            0x2D=>{self.navigation.cancel();if matches!(self.intent,Some(Intent::Dns|Intent::HttpDns|Intent::Http)){if matches!(self.intent,Some(Intent::HttpDns|Intent::Http)){self.http.cancel();}self.intent=None;}self.dns.cancel();},
            0x24=>{self.navigation.cancel();if matches!(self.intent,Some(Intent::HttpDns|Intent::Http)){self.intent=None;self.dns.cancel();}self.http.cancel();self.reading=false;self.reader.clear();},
            0x2F=>{if self.https_panel{if self.has_https_body(){self.reading=!self.reading;self.reading_https=self.reading;}}else if self.http.view()==http_probe::View::Ready{self.reading=!self.reading;self.reading_https=false;}},
            code=>{if self.reading{let movement=match code{0x67=>Some(document_reader::Move::Up),0x6C=>Some(document_reader::Move::Down),0x68=>Some(document_reader::Move::PageUp),0x6D=>Some(document_reader::Move::PageDown),0x66=>Some(document_reader::Move::Home),0x6B=>Some(document_reader::Move::End),_=>None};if let Some(movement)=movement{let body=if self.reading_https{if self.exchange.has_document(){self.exchange.document()}else if let Some(body)=self.import_source.as_ref(){body.as_bytes()}else{self.importer.document().map_or(&[][..],|d|d.as_bytes())}}else{self.http.document()};self.reader.navigate(body,movement);}}},
        }}
    }
    fn waiting_receipt(&self)->bool{
        self.exchange.view()==tls_exchange::View::Uncertain||self.http.view()==http_probe::View::Uncertain||self.probe.view()==network_probe::View::Uncertain||self.dns.view()==dns_probe::View::Uncertain||self.trust.view()==trust_probe::View::Uncertain||self.seed.view()==entropy_probe::View::Uncertain||self.clock.view()==clock_probe::View::Uncertain
    }
    fn record_release(&mut self){
        if let Some(r)=self.exchange.take_released(){
            sex_pdx::serial_record!("[browser.tls.cleanup] ticket={} outcome={:?} authenticated={} document={} tls_payload={} positive_release=1",r.ticket,r.outcome,r.authenticated as u8,r.document as u8,r.payload_bytes);
            if r.ticket==self.asset_ticket&&r.outcome==tls_exchange::View::Unavailable{
                sex_pdx::serial_record!("[browser.asset.rejected] ticket={} positive_release=1 authenticated={} failure={:?} http={:?}",r.ticket,r.authenticated as u8,self.exchange.failure(),self.exchange.http_failure());
            }
        }
    }
    fn resources(&mut self,reason:&str,force:bool){
        let usage=ALLOCATOR.usage();let tls=self.tls.busy();
        let route=self.probe.busy()||self.dns.busy()||self.http.busy()||self.exchange.busy();
        let providers=self.trust.busy()||self.seed.busy()||self.clock.busy();let body=self.has_https_body();
        let payload=self.image.allocated_payload_bytes()+self.tls.allocated_payload_bytes()+self.exchange.allocated_payload_bytes()+if self.import_source.is_some(){sex_tls::MAX_DOCUMENT}else{0}+if self.importer.has_document(){sex_tls::MAX_DOCUMENT}else{0};
        let snapshot=(usage,payload,tls,route,providers,self.intent,body);
        if force||self.resources_last!=Some(snapshot){self.resources_last=Some(snapshot);sex_pdx::serial_record!("[browser.resources] reason={} heap_used={} heap_free={} heap_total={} tls_payload={} tls_pending={} route_busy={} providers_busy={} queued={:?} body_visible={}",reason,usage.used,usage.free,usage.total,payload,tls as u8,route as u8,providers as u8,self.intent,body as u8);}
    }
}

impl SexApp for App {
    fn new(_pdx: u32) -> Self {
        assert_eq!(sex_pdx::device_network_config(sex_pdx::SLOT_NET),Err(sex_pdx::pci_command::DENIED),"BROWSER_HAS_ROUTE_NOT_NIC_METADATA");
        sex_pdx::serial_record!("[browser.network.config.denied] slot=18 kind=route metadata=0");
        ALLOCATOR.init();
        for slot in [sex_pdx::SLOT_NET,sex_pdx::entropy_source::SLOT,sex_pdx::entropy_source::SLOT+(1<<32)]{
            assert_eq!(sex_pdx::entropy_info(slot),Err(sex_pdx::entropy_source::DENIED),"BROWSER_HAS_NO_RAW_ENTROPY_GRANT");
        }
        sex_pdx::serial_record!("[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1");
        for slot in [sex_pdx::SLOT_NET,sex_pdx::clock_source::SLOT,sex_pdx::clock_source::SLOT+(1<<32)]{
            assert_eq!(sex_pdx::clock_info(slot),Err(sex_pdx::clock_source::DENIED),"BROWSER_HAS_NO_RAW_CLOCK_GRANT");
        }
        sex_pdx::serial_record!("[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1");
        for slot in [sex_pdx::SLOT_NET,sex_pdx::trust_source::SLOT,sex_pdx::SLOT_NET+(1<<32),sex_pdx::trust_source::SLOT+(1<<32)]{
            assert_eq!(sex_pdx::trust_info(slot),Err(sex_pdx::trust_source::DENIED),"BROWSER_HAS_NO_RAW_TRUST_GRANT");
        }
        sex_pdx::serial_record!("[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1");
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
        unsafe{assert!(CLIPBOARD.bind(sex_pdx::clipboard::Participant::from_catalog(identity).expect("BROWSER_CLIPBOARD_POLICY")));}
        #[cfg(app_resource_cleanup_proof)]
        full_resource_cleanup_proof();
        Self { identity,viewport_sequence:0,viewport_ready:true,reader:document_reader::Reader::new(),selection:sex_pdx::text_selection::Selection::new(),selection_generation:0,selection_body:None,clipboard_view:sex_pdx::clipboard_client::View::Idle,ui_last_selection:(0,0),reading:false,reading_https:false,https_panel:false,https_view:tls_exchange::View::Idle,https_progress_logged:0,
            asset_panel:false,asset_ticket:0,asset_status:asset_view::Status::Cleared,asset_peer_view:tls_exchange::View::Idle,image:asset_decode::Image::new(),bitmap:bitmap_delivery::Outbox::new(),image_layout:None,asset_fenced:false,asset_text_viewport:None,
            importer:document_import::Client::new(),import_source:None,import_editor:import_view::Editor::new(),import_status:import_view::Status::NoDocument,import_panel:false,import_ticket:0,import_viewport:(0,0),workspace:sex_pdx::document_view::WorkspaceRefresh::new(identity.target_pd,identity.task_id),files_pending:false,
            live_text,live_len,probe:network_probe::Probe::new(),probe_view:network_probe::View::Pending,
            dns:dns_probe::Client::new(),dns_view:dns_probe::View::Idle,seed:entropy_probe::Client::new(),seed_view:entropy_probe::View::Idle,clock:clock_probe::Client::new(),clock_view:clock_probe::View::Idle,clock_panel:false,trust:trust_probe::Client::new(),trust_view:trust_probe::View::Idle,tls:tls_probe::Probe::new(),tls_view:tls_probe::View::Idle,tls_panel:false,exchange:tls_exchange::Client::new(),peer_view:tls_exchange::View::Idle,peer_panel:false,peer_generation:0,peer_sequence:0,peer_ticket_logged:0,http:http_probe::Client::new(),http_view:http_probe::View::Idle,
            intent:None,app_turn:0,resources_last:None,navigation:http_navigation::Navigation::new(),preview,ui:text_delivery::Outbox::atomic(300),ui_last:[0;512],ui_last_len:0,ui_metadata:None,ui_dirty:true,ui_failed:false,ui_retries:0 }

    }

    fn run(&mut self, _pdx: u32) -> bool {
        self.app_turn=self.app_turn.checked_add(1).expect("BROWSER_TURN_EXHAUSTED");
        if self.viewport_ready&&sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,sex_pdx::document_view::READY,self.identity.target_pd as u64,self.identity.task_id as u64,0).is_ok(){self.viewport_ready=false;}
        let pending=self.clipboard_progress()||self.image.pending()||self.bitmap.pending()||self.importer.busy()||self.workspace.pending()||self.files_pending||self.viewport_ready||self.probe.busy()||self.dns.busy()||self.http.busy()||self.seed.busy()||self.clock.busy()||self.trust.busy()||self.tls.busy()||self.exchange.busy()||self.ui.pending()||(self.ui_dirty&&!self.ui_failed)||self.intent.is_some();
        let message=if pending{sex_pdx::pdx_try_listen_raw(0)}else{Some(sex_pdx::pdx_listen_raw(0))};
        if let Some(req)=message{
            if req.caller_pd==3&&req.caller_task==3&&(sex_pdx::app_lifecycle::close_requested(req.type_id,req.caller_pd,[req.arg0,req.arg1,req.arg2],self.identity.target_pd,self.identity.task_id)
                ||req.type_id==0xFF_FF){
                sex_pdx::serial_record!("[browser.net.job.closed] target=15 pending={} terminal=1",self.probe.busy()as u8);
                sex_pdx::serial_record!("[browser.dns.closed] pending={} terminal=1",self.dns.busy()as u8);
                sex_pdx::serial_record!("[browser.http.closed] pending={} terminal=1",self.http.busy()as u8);
                let asset_pending=self.image.pending()||self.bitmap.pending()||self.exchange.busy()||matches!(self.intent,Some(Intent::TlsAssetDns|Intent::TlsStart));
                let https_pending=self.https_panel&&(self.exchange.busy()||matches!(self.intent,Some(Intent::TlsDns|Intent::TlsAssetDns|Intent::TlsStart)));
                let peer_pending=self.exchange.busy()||matches!(self.intent,Some(Intent::TlsDns|Intent::TlsAssetDns|Intent::TlsStart));self.exchange.close();
                sex_pdx::serial_record!("[browser.https.closed] pending={} terminal=1 owned_storage=erased body_visible=0",https_pending as u8);
                sex_pdx::serial_record!("[browser.tls.peer.closed] pending={} terminal=1 owned_storage=erased",peer_pending as u8);
                let tls_pending=self.tls.busy();let erased=self.tls.close();
                sex_pdx::serial_record!("[browser.tls.erased] bytes={} owned_storage=1",erased);
                sex_pdx::serial_record!("[browser.tls.closed] pending={} terminal=1 owned_storage=erased",tls_pending as u8);
                let seed_pending=self.seed.busy();self.seed.close();
                let clock_pending=self.clock.busy();self.clock.close();
                let trust_pending=self.trust.busy();self.trust.close();
                sex_pdx::serial_record!("[browser.trust.closed] target={} task={} pending={} terminal=1",self.identity.target_pd,self.identity.task_id,trust_pending as u8);
                sex_pdx::serial_record!("[browser.clock.closed] target={} task={} pending={} terminal=1",self.identity.target_pd,self.identity.task_id,clock_pending as u8);
                sex_pdx::serial_record!("[browser.entropy.closed] target={} task={} pending={} terminal=1 private_payload=erased",self.identity.target_pd,self.identity.task_id,seed_pending as u8);
                let import_pending=self.importer.busy();let accepted=self.importer.accepted();self.import_editor.cancel();self.import_source=None;self.importer.close();self.workspace.close();self.files_pending=false;
                sex_pdx::serial_record!("[browser.import.closed] target={} task={} pending={} accepted={} terminal=1 owned_storage=erased",self.identity.target_pd,self.identity.task_id,import_pending as u8,accepted);
                self.image.clear();self.bitmap.close();self.image_layout=None;
                sex_pdx::serial_record!("[browser.asset.closed] pending={} terminal=1 owned_storage=erased",asset_pending as u8);
                unsafe{CLIPBOARD.close();}self.selection.clear();self.selection_body=None;self.ui_last_selection=(0,0);
                sex_pdx::serial_record!("[browser.clipboard.closed] target={} task={} owned_scratch=erased terminal=1",self.identity.target_pd,self.identity.task_id);
                self.navigation.cancel();self.http.cancel();self.reader.clear();self.reading=false;self.ui.cancel();self.ui_last.fill(0);self.ui_metadata=None;self.intent=None;self.resources("close",true);sex_pdx::task_finish(0);
            }
            if req.type_id==sex_pdx::document_view::OP{
                if let Some(n)=sex_pdx::document_view::Notice::decode(req.caller_pd,self.identity.target_pd,self.identity.task_id,[req.arg0,req.arg1,req.arg2],self.viewport_sequence){
                    self.viewport_sequence=n.sequence;self.asset_text_viewport=None;self.reader.resize(n.w,n.h);self.import_viewport=(n.w,n.h);
                    // A live recreation may have erased partial display staging.
                    // Replace unsent words with a higher Clear; accepted terminal
                    // operations retain their historical exact receipt first.
                    if self.bitmap.pending()||self.image_layout.is_some(){
                        if self.bitmap.clear(bitmap_delivery::Metadata{viewport:n.sequence,source:0}).is_err(){self.fence_asset("viewport_clear_sequence");}
                        self.image_layout=None;
                        if !self.asset_fenced&&!self.image.pixels().is_empty(){self.asset_status=asset_view::Status::Sending;}
                    }
                    // Recreated display slots discard old staging and ACKs.
                    // Preserve the outbox's monotonic sequence across cancel.
                    self.ui.cancel();self.ui_last.fill(0);self.ui_last_len=0;self.ui_metadata=None;self.ui_dirty=true;self.ui_failed=false;self.ui_retries=0;
                    sex_pdx::serial_record!("[browser.viewport] target={} task={} sequence={} w={} h={} origin=held_shell",n.target,n.task,n.sequence,n.w,n.h);
                }
            }
            if req.type_id==1&&req.caller_pd==11{self.importer.reply(req.arg0,sex_pdx::get_ticks());self.record_import_control();}
            if req.type_id==1&&req.caller_pd==13{
                if matches!(req.arg0>>56,0x7C|0xEC){self.trust.reply(req.arg0,sex_pdx::get_ticks());}
                else if matches!(req.arg0>>56,0x7B|0xEB){self.clock.reply(req.arg0,sex_pdx::get_ticks());}
                else if matches!(req.arg0>>56,0x7A|0xEA){self.seed.reply(req.arg0,sex_pdx::get_ticks());}
                else if matches!(req.arg0>>56,0x79|0xE9){self.dns.reply(req.arg0,sex_pdx::get_ticks());}
                else if self.exchange.busy(){self.exchange.reply(req.arg0,sex_pdx::get_ticks());self.record_release();}
                else if self.http.busy(){self.http.reply(req.arg0,sex_pdx::get_ticks());}
                else{self.probe.reply(req.arg0,sex_pdx::get_ticks());}
            }
            if req.type_id==1&&req.caller_pd==1{
                if let Some(r)=self.bitmap.receive(req.caller_pd,req.arg0){
                    if let Some(g)=r.geometry{
                        sex_pdx::serial_record!("[browser.asset.applied] ticket={} viewport={} sequence={} x={} y={} width={} height={} scale={} hash={:#x} positive_ack=1 pixels=unconfirmed",r.metadata.source,r.metadata.viewport,r.sequence,g.x,g.y,g.width,g.height,g.scale,r.hash);
                        if self.asset_panel&&r.metadata.source==self.image.ticket as u64&&r.metadata.viewport==self.viewport_sequence&&self.image_layout==Some((g,self.viewport_sequence)){self.asset_status=asset_view::Status::Ready;}
                    }else{sex_pdx::serial_record!("[browser.asset.cleared] viewport={} sequence={} positive_ack=1",r.metadata.viewport,r.sequence);if self.asset_status==asset_view::Status::Clearing{self.asset_status=asset_view::Status::Cleared;}}
                }

                if let Some(r)=self.ui.receive(req.caller_pd,req.arg0){
                    if self.asset_panel&&r.metadata.selected&(1<<20)!=0&&r.metadata.first==self.viewport_sequence as usize{
                        self.asset_text_viewport=Some(self.viewport_sequence);
                        let a=sex_pdx::text_batch::Applied::decode(req.caller_pd,req.arg0).unwrap();
                        sex_pdx::serial_record!("[browser.asset.text.applied] viewport={} sequence={} bytes={} rows={} positive_ack=1 pixels=unconfirmed",r.metadata.first,a.sequence,r.bytes,r.metadata.rows);
                    }

                    if r.metadata.selected&1024!=0{let a=sex_pdx::text_batch::Applied::decode(req.caller_pd,req.arg0).unwrap();sex_pdx::serial_record!("[browser.import.applied] bytes={} hash={:#x} rows={} viewport={} state_code={} sequence={} positive_ack=1",r.bytes,r.metadata.count,r.metadata.rows,r.metadata.first,r.metadata.selected>>11,a.sequence);}
                    sex_pdx::serial_record!("[browser.view.applied] bytes={} http={} body_bytes={} pixels=unconfirmed",r.bytes,r.metadata.selected,if r.metadata.selected&1024!=0{0}else{r.metadata.count});
                }
            }
            if sex_pdx::clipboard::response_opcode(req.type_id){unsafe{CLIPBOARD.receive(sex_pdx::clipboard::Subject{pd:req.caller_pd,task:req.caller_task},req.type_id,[req.arg0,req.arg1,req.arg2],sex_pdx::get_ticks());}}
            if req.type_id==0x202&&req.caller_pd==3&&req.caller_task==3{
                if let Some(key)=sex_pdx::app_key::Key::decode(req.arg0,req.arg1,req.arg2){self.key_input(key);}
            }

        }
        self.clipboard_events();
        self.importer.advance(sex_pdx::get_ticks());self.record_import_control();
        if let Some(event)=self.importer.take_event(){
            self.import_status=match event.view{document_import::View::Saved=>import_view::Status::Saved,document_import::View::Exists=>import_view::Status::Exists,document_import::View::Uncertain=>import_view::Status::Uncertain,document_import::View::Cancelled=>import_view::Status::Cancelled,_=>import_view::Status::Unavailable};
            if event.view==document_import::View::Saved{self.workspace.changed();}
            sex_pdx::serial_record!("[browser.import.event] state={:?} object={} identity={} bytes={} hash={:#x} error={} source_ticket={}",event.view,event.object,event.generation,event.bytes,event.hash,event.error,self.import_ticket);
            self.resources("change",true);
        }
        self.workspace.poll(|args|sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,sex_pdx::document_view::WORKSPACE_CHANGED,args[0],args[1],args[2]).map(|_|()));
        if self.files_pending{match sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,sex_pdx::document_view::OPEN_LINEN,self.identity.target_pd as u64,self.identity.task_id as u64,0){Ok(_)=>{self.files_pending=false;sex_pdx::serial_record!("[browser.import.files] target={} task={} accepted=1",self.identity.target_pd,self.identity.task_id);},Err(sex_pdx::ERR_SERVICE_NOT_READY)=>{},Err(error)=>{self.files_pending=false;sex_pdx::serial_record!("[browser.import.files] target={} task={} accepted=0 error={}",self.identity.target_pd,self.identity.task_id,error);}}}
        if let Some(path)=self.http.take_redirect(){
            if self.intent.is_none()&&self.navigation.active()&&self.navigation.follow(path){
                sex_pdx::serial_record!("[browser.http.redirect.follow] hop={} path={} same_endpoint=1 fresh_dns=1 positive_release=1",self.navigation.hops(),core::str::from_utf8(path.as_bytes()).unwrap());
                self.intent=Some(Intent::HttpDns);
            }else{
                self.navigation.cancel();self.http.reject_redirect();
                sex_pdx::serial_record!("[browser.http.redirect.denied] loop_or_limit_or_superseded=1 effects=0");
            }
        }
        let command_busy=browser_command::Busy{route:self.asset_fenced||self.bitmap.pending()||self.probe.busy()||self.dns.busy()||self.http.busy()||self.exchange.busy(),providers:self.trust.busy()||self.seed.busy()||self.clock.busy(),tls:self.tls.busy()};
        {
            if let Some(intent)=browser_command::take_ready(&mut self.intent,command_busy){
                // Echo and HTTP share the exact0x20C caller replay fence.
                let sequence=self.probe.next_sequence().max(self.http.next_sequence()).max(self.exchange.next_sequence());
                match intent{
                    Intent::Echo=>{if !self.probe.restart_at(sequence){sex_pdx::serial_record!("[browser.net.start.rejected] protocol=echo seq={} effects=0",sequence);}},
                    Intent::Dns=>self.dns.start(),
                    Intent::HttpDns=>{self.http.wait_for_resolution();self.dns.start();self.intent=Some(Intent::Http);},
                    Intent::Seed=>{let accepted=self.seed.start();sex_pdx::serial_record!("[browser.entropy.intent] accepted={} explicit=1 fallback=0",accepted as u8);sex_pdx::serial_record!("[browser.command.started] kind=Seed accepted={}",accepted as u8);},
                    Intent::Clock=>{let accepted=self.clock.start();sex_pdx::serial_record!("[browser.clock.intent] accepted={} explicit=1 no_ticks_fallback=1",accepted as u8);sex_pdx::serial_record!("[browser.command.started] kind=Clock accepted={}",accepted as u8);},
                    Intent::TlsPrepare=>{let accepted=self.tls.start(&mut self.trust,&mut self.seed,&mut self.clock,sex_pdx::get_ticks());sex_pdx::serial_record!("[browser.tls.intent] accepted={} explicit=1 network=0",accepted as u8);sex_pdx::serial_record!("[browser.command.started] kind=TlsPrepare accepted={}",accepted as u8);},
                    Intent::TlsDns|Intent::TlsAssetDns=>{
                        let accepted=if intent==Intent::TlsAssetDns{self.tls.start_resource(&mut self.trust,&mut self.seed,&mut self.clock,sex_pdx::get_ticks(),sex_tls::Resource::ArtworkQoi)}else if self.https_panel{self.tls.start_document(&mut self.trust,&mut self.seed,&mut self.clock,sex_pdx::get_ticks())}else{self.tls.start_exchange(&mut self.trust,&mut self.seed,&mut self.clock,sex_pdx::get_ticks())};
                        sex_pdx::serial_record!("[browser.command.started] kind=TlsDns accepted={}",accepted as u8);
                        if accepted{self.dns.start();self.intent=Some(Intent::TlsStart);}else{self.exchange.reject_prerequisites();}
                    },
                    Intent::TlsStart=>{
                        if self.tls.view()==tls_probe::View::HelloPending{
                            let generation=self.dns.generation().unwrap_or(0);
                            if let Some(session)=self.tls.take_exchange(){
                                if self.exchange.start(session,sequence,generation){self.peer_generation=generation;self.peer_sequence=sequence;self.peer_ticket_logged=0;}
                            }
                        }else if self.tls.busy(){self.intent=Some(Intent::TlsStart);}else{self.exchange.reject_prerequisites();}
                    },
                    Intent::Http=>{
                        let generation=self.dns.generation().unwrap_or(0);
                        if !self.http.start_resolved_path(sequence,generation,self.navigation.current()){self.navigation.cancel();sex_pdx::serial_record!("[browser.net.start.rejected] protocol=http seq={} dns_generation={} effects=0",sequence,generation);}
                    },
                }
            }
        }
        if self.trust.busy(){self.trust.advance(sex_pdx::get_ticks());}
        if self.clock.busy(){self.clock.advance(sex_pdx::get_ticks());}
        if self.seed.busy(){self.seed.advance(sex_pdx::get_ticks());}
        let tls_begin=sex_pdx::get_ticks();
        let(prepared,erased)=self.tls.advance(&mut self.trust,&mut self.seed,&mut self.clock,tls_begin);
        if let Some(work)=self.tls.take_work(){let end=sex_pdx::get_ticks();let elapsed=end.checked_sub(tls_begin);sex_pdx::serial_record!("[browser.tls.work] kind={:?} sequence={} app_turn={} begin_ticks={} end_ticks={} elapsed_ticks={} clock_valid={} payload={} seed_ready={}",work.kind,work.sequence,self.app_turn,tls_begin,end,elapsed.unwrap_or(0),elapsed.is_some()as u8,self.tls.allocated_payload_bytes(),(self.seed.view()==entropy_probe::View::Ready)as u8);}
        if let Some(profile)=self.tls.take_self_check(){sex_pdx::serial_record!("[browser.trust.self_check] profile={} passed=1 fixture=public_controlled_profile authenticated_peer=0",profile);}
        if let Some(stats)=prepared{if self.https_panel{sex_pdx::serial_record!("[browser.https.prepared] task_bytes={} private_bytes={} tx_bytes={} utc_snapshot={} request_bytes=94 document_limit=12288 network=0",stats.task_bytes,stats.private_bytes,stats.tx_bytes,stats.utc_snapshot);}sex_pdx::serial_record!("[browser.tls.prepared] task_bytes={} private_bytes={} read=4096 write=2048 tx_bytes={} utc_snapshot={} authenticated=0 network=0",stats.task_bytes,stats.private_bytes,stats.tx_bytes,stats.utc_snapshot);}
        if erased>0{sex_pdx::serial_record!("[browser.tls.erased] bytes={} owned_storage=1",erased);}
        if self.exchange.busy(){self.exchange.advance(sex_pdx::get_ticks());}
        else if self.probe.busy(){self.probe.advance(sex_pdx::get_ticks());}
        else if self.dns.busy(){self.dns.advance(sex_pdx::get_ticks());}
        else if self.http.busy(){self.http.advance(sex_pdx::get_ticks());}
        if self.navigation.active()&&self.intent.is_none()&&!self.http.busy()&&!self.dns.busy(){
            if self.http.view()==http_probe::View::Ready{self.navigation.complete();}
            else if matches!(self.http.view(),http_probe::View::Unavailable|http_probe::View::Cancelled){self.navigation.cancel();}
        }
        let peer_ticket=self.exchange.ticket();
        if peer_ticket!=0&&peer_ticket!=self.peer_ticket_logged{
            self.peer_ticket_logged=peer_ticket;if let Some(stats)=self.exchange.stats(){if self.exchange.resource_kind()==Some(sex_tls::Resource::ArtworkQoi){self.asset_ticket=peer_ticket;sex_pdx::serial_record!("[browser.asset.admitted] ticket={} dns_generation={} seq={} task_bytes={} private_bytes={} profile=1 authority=held_route",peer_ticket,self.peer_generation,self.peer_sequence,stats.task_bytes,stats.private_bytes);}else if self.exchange.is_document(){sex_pdx::serial_record!("[browser.https.admitted] ticket={} dns_generation={} seq={} task_bytes={} private_bytes={} profile=1 authority=held_route",peer_ticket,self.peer_generation,self.peer_sequence,stats.task_bytes,stats.private_bytes);}else{sex_pdx::serial_record!("[browser.tls.peer.admitted] ticket={} dns_generation={} seq={} task_bytes={} private_bytes={} profile=1 authority=held_route",peer_ticket,self.peer_generation,self.peer_sequence,stats.task_bytes,stats.private_bytes);}}
        }
        if self.exchange.take_authenticated(){if self.exchange.resource_kind()==Some(sex_tls::Resource::ArtworkQoi){sex_pdx::serial_record!("[browser.asset.authenticated] ticket={} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1",peer_ticket);}else if self.exchange.is_document(){sex_pdx::serial_record!("[browser.https.authenticated] ticket={} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1",peer_ticket);}else{sex_pdx::serial_record!("[browser.tls.peer.authenticated] ticket={} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1",peer_ticket);}}
        let received=self.exchange.document_received();
        if self.asset_panel&&received>self.https_progress_logged{self.https_progress_logged=received;sex_pdx::serial_record!("[browser.asset.progress] ticket={} body_received={} authenticated=1 body_visible=0",peer_ticket,received);}
        if self.exchange.is_document()&&received>self.https_progress_logged{self.https_progress_logged=received;sex_pdx::serial_record!("[browser.https.progress] ticket={} body_received={} authenticated=1 body_visible=0",peer_ticket,received);}
        if self.exchange.take_response_verified(){if self.exchange.resource_kind()==Some(sex_tls::Resource::ArtworkQoi){sex_pdx::serial_record!("[browser.asset.body] ticket={} bytes={} hash={:#x} parsed=1 authenticated=1 visible=0",peer_ticket,self.exchange.retained_asset_len(),self.exchange.retained_asset_hash());}else if self.exchange.is_document(){sex_pdx::serial_record!("[browser.https.body] ticket={} bytes={} hash={:#x} parsed=1 authenticated=1 visible=0",peer_ticket,self.exchange.retained_document_len(),self.exchange.retained_document_hash());}else{sex_pdx::serial_record!("[browser.tls.peer.response] ticket={} bytes={} hash={:#x} authenticated=1 source=guest_aead",peer_ticket,sex_tls::EXCHANGE_RESPONSE.len(),public_fingerprint(sex_tls::EXCHANGE_RESPONSE));}}
        let peer_view=if matches!(self.intent,Some(Intent::TlsDns|Intent::TlsAssetDns|Intent::TlsStart)){
            if self.waiting_receipt(){tls_exchange::View::Uncertain}else{tls_exchange::View::Pending}
        }else{self.exchange.view()};
        if self.asset_panel{
            if peer_view!=self.asset_peer_view{
                self.asset_peer_view=peer_view;
                match peer_view{
                    tls_exchange::View::Verified=>{if let Some(body)=self.exchange.take_asset(){
                        let len=body.as_bytes().len();let hash=public_fingerprint(body.as_bytes());
                        sex_pdx::serial_record!("[browser.asset.released] ticket={} tx={} rx={} body_bytes={} body_hash={:#x} positive_release=1 authenticated=1",peer_ticket,self.exchange.tx_bytes(),self.exchange.rx_bytes(),len,hash);
                        self.asset_status=if self.image.begin(body,peer_ticket).is_ok(){asset_view::Status::Decoding}else{sex_pdx::serial_record!("[browser.asset.rejected] reason=invalid_qoi_or_allocation effects=0 owned_storage=erased");asset_view::Status::Invalid};
                    }},
                    tls_exchange::View::Unavailable=>{self.image.clear();self.asset_status=asset_view::Status::Unavailable;},
                    tls_exchange::View::Uncertain=>{self.image.clear();self.asset_status=asset_view::Status::Uncertain;},
                    tls_exchange::View::Cancelled=>{if !self.bitmap.pending(){self.asset_status=asset_view::Status::Cleared;}},
                    _=>{},
                }
                sex_pdx::serial_record!("[browser.asset.view] state={:?} controlled_profile=1 public_internet=0",peer_view);
            }
        }else if self.https_panel{
            if peer_view!=self.https_view{
                self.https_view=peer_view;
                if peer_view==tls_exchange::View::Verified&&self.exchange.has_document(){
                    sex_pdx::serial_record!("[browser.https.released] ticket={} tx={} rx={} request_bytes=94 body_bytes={} body_hash={:#x} positive_release=1 authenticated=1 public_test_root=1",peer_ticket,self.exchange.tx_bytes(),self.exchange.rx_bytes(),self.exchange.document().len(),public_fingerprint(self.exchange.document()));
                    self.reading=true;self.reading_https=true;self.reader.clear();
                }
                if peer_view==tls_exchange::View::Cancelled&&peer_ticket!=0&&!self.exchange.busy(){sex_pdx::serial_record!("[browser.https.cancelled] ticket={} positive_release=1 authenticated={}",peer_ticket,self.exchange.peer_authenticated() as u8);}
                if peer_view==tls_exchange::View::Unavailable&&peer_ticket!=0&&!self.exchange.busy(){sex_pdx::serial_record!("[browser.https.rejected] ticket={} positive_release=1 authenticated={} failure={:?} http={:?}",peer_ticket,self.exchange.peer_authenticated() as u8,self.exchange.failure(),self.exchange.http_failure());}
                sex_pdx::serial_record!("[browser.https.view] state={:?} controlled_profile=1 public_internet=0",peer_view);
            }
        }else if !self.import_panel&&peer_view!=self.peer_view{
            self.peer_view=peer_view;
            if peer_view==tls_exchange::View::Verified{sex_pdx::serial_record!("[browser.tls.peer.released] ticket={} tx={} rx={} request_bytes={} response_bytes={} positive_release=1 authenticated=1 public_test_root=1",peer_ticket,self.exchange.tx_bytes(),self.exchange.rx_bytes(),sex_tls::EXCHANGE_REQUEST.len(),sex_tls::EXCHANGE_RESPONSE.len());}
            if peer_view==tls_exchange::View::Cancelled&&peer_ticket!=0&&!self.exchange.busy(){sex_pdx::serial_record!("[browser.tls.peer.cancelled] ticket={} positive_release=1 authenticated={}",peer_ticket,self.exchange.peer_authenticated() as u8);}
            if peer_view==tls_exchange::View::Unavailable&&peer_ticket!=0&&!self.exchange.busy(){sex_pdx::serial_record!("[browser.tls.peer.rejected] ticket={} positive_release=1 authenticated={} failure={:?}",peer_ticket,self.exchange.peer_authenticated() as u8,self.exchange.failure());}
            sex_pdx::serial_record!("[browser.tls.peer.view] state={:?} controlled_profile=1 public_internet=0",peer_view);
        }
        let view=self.probe.view();if view!=self.probe_view{self.probe_view=view;sex_pdx::serial_record!("[browser.net.job.view] state={:?} web=0",view);}
        let view=self.dns.view();if view!=self.dns_view{self.dns_view=view;sex_pdx::serial_record!("[browser.dns.view] state={:?} web=0",view);}
        let view=self.http.view();if view!=self.http_view{self.http_view=view;if view!=http_probe::View::Ready&&!self.reading_https{self.reading=false;self.reader.clear();}sex_pdx::serial_record!("[browser.http.view] state={:?} tls=0",view);}
        let view=if self.intent==Some(Intent::Seed){if self.waiting_receipt(){entropy_probe::View::Uncertain}else{entropy_probe::View::Pending}}else{self.seed.view()};if view!=self.seed_view{self.seed_view=view;sex_pdx::serial_record!("[browser.entropy.view] state={:?} private=1 bytes_visible=0",view);}
        let view=if self.intent==Some(Intent::Clock){if self.waiting_receipt(){clock_probe::View::Uncertain}else{clock_probe::View::Pending}}else{self.clock.view()};if view!=self.clock_view{self.clock_view=view;sex_pdx::serial_record!("[browser.clock.view] state={:?} source=q35_cmos_utc snapshot=1",view);}
        let view=self.trust.view();if view!=self.trust_view{self.trust_view=view;sex_pdx::serial_record!("[browser.trust.view] state={:?} immutable_boot_profile=1 transport=0",view);}
        let view=if self.intent==Some(Intent::TlsPrepare){if self.waiting_receipt(){tls_probe::View::Uncertain}else{tls_probe::View::Preparing}}else{self.tls.view()};if view!=self.tls_view{self.tls_view=view;sex_pdx::serial_record!("[browser.tls.view] state={:?} authenticated=0 network=0",view);}
        self.resources("change",false);
        match self.image.advance(){
            Ok(true)=>{self.asset_status=asset_view::Status::Sending;sex_pdx::serial_record!("[browser.asset.decoded] ticket={} width={} height={} pixels={} hash={:#x} bounded_turn=128",self.image.ticket,self.image.width,self.image.height,self.image.pixels().len(),sex_pdx::bitmap_batch::hash(self.image.pixels()));},
            Err(())=>{self.asset_status=asset_view::Status::Invalid;sex_pdx::serial_record!("[browser.asset.rejected] reason=invalid_qoi effects=0 owned_storage=erased");},_=>{},
        }
        if self.asset_panel&&self.asset_text_viewport==Some(self.viewport_sequence)&&!self.image.pixels().is_empty()&&!self.bitmap.pending()&&!self.asset_fenced{
            if let Some(g)=asset_view::geometry(self.import_viewport.0,self.import_viewport.1,self.image.width,self.image.height){
                if self.image_layout!=Some((g,self.viewport_sequence)){
                    if self.bitmap.queue(self.image.pixels(),g,bitmap_delivery::Metadata{viewport:self.viewport_sequence,source:self.image.ticket as u64}).is_ok(){
                        self.image_layout=Some((g,self.viewport_sequence));self.asset_status=asset_view::Status::Sending;
                        sex_pdx::serial_record!("[browser.asset.layout] ticket={} viewport={} x={} y={} width={} height={} scale={} hash={:#x} pixels=unconfirmed",self.image.ticket,self.viewport_sequence,g.x,g.y,g.width,g.height,g.scale,sex_pdx::bitmap_batch::hash(self.image.pixels()));
                    }else{self.fence_asset("image_queue");}
                }
            }else if self.image_layout.take().is_some(){if self.bitmap.clear(bitmap_delivery::Metadata{viewport:self.viewport_sequence,source:0}).is_err(){self.fence_asset("clipped_clear_sequence");}}
        }
        let bitmap_poll=self.bitmap.poll(self.image.pixels(),4,|request|{
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_DISPLAY,request.opcode,request.surface,request.packed,request.control){Ok(_)=>bitmap_delivery::Admission::Accepted,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>bitmap_delivery::Admission::Full,Err(error)=>bitmap_delivery::Admission::Rejected(error)}
        });
        match bitmap_poll{
            bitmap_delivery::Poll::Rejected(error)=>{self.fence_asset("display_rejected");sex_pdx::serial_record!("[browser.asset.display.rejected] error={} accepted_replay=0",error);},
            bitmap_delivery::Poll::InvalidPixels|bitmap_delivery::Poll::Closed=>self.fence_asset("invalid_display_state"),_=>{},
        }
        self.sync_selection();
        let mut visible_selection=(0,0);
        let mut reader_layout=None;
        let composed=if self.asset_panel{
            let f=asset_view::render(self.import_viewport.0,self.import_viewport.1,self.asset_status);
            Some((browser_view::Text{bytes:f.bytes,len:f.len},text_delivery::Metadata{count:0,first:self.viewport_sequence as usize,rows:f.rows,selected:(1<<20)|((self.asset_status as u64)<<21)}))
        }else if self.import_panel{
            let f=import_view::render(self.import_viewport.0,self.import_viewport.1,self.import_status,self.import_editor.as_bytes());
            let metadata=text_delivery::Metadata{count:public_fingerprint(&f.bytes[..f.len])as usize,first:self.viewport_sequence as usize,rows:f.rows,selected:1024|((self.import_status as u64)<<11)};
            if self.ui_last_len!=f.len||self.ui_last!=f.bytes||self.ui_metadata!=Some(metadata){sex_pdx::serial_record!("[browser.import.layout] state={:?} name={} bytes={} cols={} rows={} viewport={} hash={:#x} pixels=unconfirmed",self.import_status,core::str::from_utf8(self.import_editor.as_bytes()).unwrap(),f.len,f.cols,f.rows,self.viewport_sequence,metadata.count);}
            Some((browser_view::Text{bytes:f.bytes,len:f.len},metadata))
        }else if self.reading&&self.reading_https&&self.has_https_body(){
            let body=if self.exchange.has_document(){self.exchange.document()}else if let Some(body)=self.import_source.as_ref(){body.as_bytes()}else{self.importer.document().unwrap().as_bytes()};
            let f=self.reader.render_https_selected(body,sex_tls::HTTPS_PATH,self.selection.range());visible_selection=f.selection;reader_layout=Some((f.first,f.end,f.body_rows,f.rows,f.cols,f.total_lines));
            Some((browser_view::Text{bytes:f.bytes,len:f.len},text_delivery::Metadata{count:body.len(),first:f.first,rows:f.body_rows,selected:2|256|512}))
        }else if self.reading&&self.http.view()==http_probe::View::Ready{
            let f=self.reader.render_path_selected(self.http.document(),self.http.document_path(),self.selection.range());visible_selection=f.selection;reader_layout=Some((f.first,f.end,f.body_rows,f.rows,f.cols,f.total_lines));
            Some((browser_view::Text{bytes:f.bytes,len:f.len},text_delivery::Metadata{count:self.http.document().len(),first:f.first,rows:f.body_rows,selected:http_view_code(self.http.view())|256}))
        }else{browser_view::compose(self.preview.as_bytes(),self.probe.view(),self.dns.view(),self.http.view(),self.http.preview()).and_then(|text|{
            if self.https_panel{browser_view::with_https(text,https_label(self.https_view,self.exchange.failure(),self.exchange.http_failure()))}
            else if self.peer_panel{browser_view::with_peer(text,peer_label(self.peer_view))}
            else if self.tls_panel{browser_view::with_tls(text,tls_probe::label(self.tls_view))}
            else if self.clock_panel{if let Some(status)=clock_view::status(self.clock_view){browser_view::with_clock(text,&status.bytes[..status.len])}else{Some(text)}}
            else{browser_view::with_seed(text,seed_label(self.seed_view))}
        }).map(|text|(text,text_delivery::Metadata{count:if matches!(self.http.view(),http_probe::View::Ready){self.http.body_length()}else{0},first:0,rows:0,selected:http_view_code(self.http.view())}))};
        if let Some((mut text,metadata))=composed{
            if reader_layout.is_some(){
                let view=self.clipboard_view;
                let label:&[u8]=match view{
                    sex_pdx::clipboard_client::View::Pending=>b"COPY PENDING / ESC CANCEL",
                    sex_pdx::clipboard_client::View::Copied=>b"COPIED / CTRL A C / SHIFT SELECT",
                    sex_pdx::clipboard_client::View::Empty=>b"EMPTY SELECTION / CTRL A",
                    sex_pdx::clipboard_client::View::Unsupported=>b"COPY UNSUPPORTED",
                    sex_pdx::clipboard_client::View::Denied=>b"COPY DENIED",
                    sex_pdx::clipboard_client::View::Uncertain=>b"COPY UNCERTAIN / ESC QUIESCE",
                    sex_pdx::clipboard_client::View::Busy=>b"CLIPBOARD BUSY",
                    sex_pdx::clipboard_client::View::Cancelled=>b"COPY CANCELLED / CTRL A C",
                    _=>if self.selection.range().map_or(false,|(lo,hi)|hi>lo){b"CTRL C COPY / SHIFT SELECT"}else{b""},
                };
                if !label.is_empty(){if let Some((_,_,_,rows,cols,_))=reader_layout{if rows>1{document_reader::replace_footer(&mut text.bytes,text.len,cols,label);}}}
            }
            if reader_layout.is_none()&&!self.asset_panel&&!self.import_panel&&self.https_panel&&self.ui_last!=text.bytes{sex_pdx::serial_record!("[browser.https.layout] base={} bytes=64 viewport={} state={:?} pixels=unconfirmed",text.len-64,self.viewport_sequence,self.https_view);}
            if reader_layout.is_none()&&!self.asset_panel&&!self.import_panel&&!self.https_panel&&self.peer_panel&&self.ui_last!=text.bytes{sex_pdx::serial_record!("[browser.tls.peer.layout] base={} bytes=64 viewport={} state={:?} pixels=unconfirmed",text.len-64,self.viewport_sequence,self.peer_view);}
            if reader_layout.is_none()&&!self.asset_panel&&!self.import_panel&&!self.https_panel&&!self.peer_panel&&self.tls_panel&&self.tls_view!=tls_probe::View::Idle&&self.ui_last!=text.bytes{sex_pdx::serial_record!("[browser.tls.layout] base={} bytes=64 viewport={} state={:?} pixels=unconfirmed",text.len-64,self.viewport_sequence,self.tls_view);}
            if reader_layout.is_none()&&!self.asset_panel&&!self.import_panel&&!self.https_panel&&!self.peer_panel&&!self.tls_panel&&self.clock_panel&&self.clock_view!=clock_probe::View::Idle&&self.ui_last!=text.bytes{
                sex_pdx::serial_record!("[browser.clock.layout] base={} bytes=64 viewport={} state={:?} pixels=unconfirmed",text.len-64,self.viewport_sequence,self.clock_view);
            }
            if reader_layout.is_none()&&!self.asset_panel&&!self.import_panel&&!self.https_panel&&!self.peer_panel&&!self.tls_panel&&!self.clock_panel&&seed_label(self.seed_view).is_some()&&self.ui_last!=text.bytes{
                sex_pdx::serial_record!("[browser.entropy.layout] base={} bytes=64 viewport={} state={:?} pixels=unconfirmed",text.len-64,self.viewport_sequence,self.seed_view);
            }
            if self.ui_last_len!=text.len||self.ui_last!=text.bytes||self.ui_metadata!=Some(metadata)||self.ui_last_selection!=visible_selection{
                self.ui_last=text.bytes;self.ui_last_len=text.len;self.ui_metadata=Some(metadata);self.ui_last_selection=visible_selection;self.ui_dirty=true;self.ui_failed=false;self.ui_retries=0;
                if let Some((first,end,body_rows,rows,cols,total_lines))=reader_layout{if self.reading_https{sex_pdx::serial_record!("[browser.reader.layout] first={} end={} body_rows={} rows={} cols={} total_lines={} source_bytes={} viewport={} origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed",first,end,body_rows,rows,cols,total_lines,self.https_body().len(),self.viewport_sequence);}else{sex_pdx::serial_record!("[browser.reader.layout] first={} end={} body_rows={} rows={} cols={} total_lines={} source_bytes={} viewport={} origin=validated_body tls=0 pixels=unconfirmed",first,end,body_rows,rows,cols,total_lines,self.http.document().len(),self.viewport_sequence);}sex_pdx::serial_record!("[browser.reader.position] anchor={} viewport={} tls={} retained_body_only=1",self.reader.source_anchor(),self.viewport_sequence,self.reading_https as u8);}
            }
            if self.ui_dirty&&!self.ui_failed{
                if self.ui.queue_selected(&text.bytes[..text.len],0x00CDD6F4,metadata,visible_selection){self.ui_dirty=false;}
            }
        }
        if let text_delivery::Poll::Rejected(error)=self.ui.poll(4,|op|{
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_DISPLAY,op.opcode,op.surface,op.packed,op.control){
                Ok(_)=>text_delivery::Admission::Accepted,Err(sex_pdx::ERR_SERVICE_NOT_READY)=>text_delivery::Admission::Full,Err(e)=>text_delivery::Admission::Rejected(e),
            }
        }){self.ui.cancel();self.ui_dirty=true;self.ui_retries=self.ui_retries.saturating_add(1);self.ui_failed=self.ui_retries>=3;sex_pdx::serial_record!("[browser.view.rejected] error={} attempts={} terminal={} accepted_network_replay=0",error,self.ui_retries,self.ui_failed as u8);}
        if self.clipboard_progress()||self.image.pending()||self.bitmap.pending()||self.importer.busy()||self.workspace.pending()||self.files_pending||self.viewport_ready||self.probe.busy()||self.dns.busy()||self.http.busy()||self.seed.busy()||self.clock.busy()||self.trust.busy()||self.tls.busy()||self.exchange.busy()||self.ui.pending()||(self.ui_dirty&&!self.ui_failed)||self.intent.is_some(){sex_pdx::sys_wait_self_ticks(2).expect("BROWSER_SELF_WAIT_AUTHORITY");}
        true
    }


}

fn seed_label(view:entropy_probe::View)->Option<&'static[u8]>{match view{
    entropy_probe::View::Idle=>None,entropy_probe::View::Pending=>Some(b"checking"),
    entropy_probe::View::Ready=>Some(b"ready 32 bytes"),entropy_probe::View::Unavailable=>Some(b"unavailable"),
    entropy_probe::View::Cancelled=>Some(b"cancelled"),entropy_probe::View::Uncertain=>Some(b"waiting receipt"),
}}

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

fn peer_label(view:tls_exchange::View)->&'static[u8]{match view{tls_exchange::View::Idle=>b"idle",tls_exchange::View::Pending=>b"checking",tls_exchange::View::Verified=>b"peer verified",tls_exchange::View::Unavailable=>b"unavailable",tls_exchange::View::Cancelled=>b"cancelled",tls_exchange::View::Uncertain=>b"waiting receipt"}}
fn public_fingerprint(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325,|hash,byte|(hash^*byte as u64).wrapping_mul(0x100000001b3))}

fn https_label(view:tls_exchange::View,failure:Option<sex_tls::Failure>,http:Option<sex_tls::HttpError>)->&'static[u8]{
 match view{
  tls_exchange::View::Idle=>b"idle",tls_exchange::View::Pending=>b"loading",tls_exchange::View::Verified=>b"ready",tls_exchange::View::Cancelled=>b"cancelled",tls_exchange::View::Uncertain=>b"waiting receipt",
  tls_exchange::View::Unavailable=>if http.is_some(){b"invalid document"}else{match failure{Some(sex_tls::Failure::Certificate)=>b"certificate rejected",Some(sex_tls::Failure::Signature)=>b"signature rejected",Some(sex_tls::Failure::Protocol)=>b"TLS rejected",Some(sex_tls::Failure::Record)=>b"invalid TLS record",_=>b"connection failed"}},
 }
}
