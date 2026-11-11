//! PRIVATE owner-bound snapshots. Never merge into the production browser.
//! Reads Rust fields and the existing JIT counters; never evaluates JS, invokes
//! JS getters, drains jobs, recomputes layout/styles or publishes pixels.
use super::{Browser, Doc};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use super::sex_html::{dom::{Dom, NodeId}, layout::Page, style::Style};

const SCAN: usize = 4096;
const CHOSEN: usize = 48;
const ANCESTORS: usize = 64;
const SLOTS: usize = super::MAX_FRAMES + 1;
static SEQ: AtomicU32 = AtomicU32::new(0);
static SHOTS: AtomicU32 = AtomicU32::new(0);
struct Cache {
    owner: u64, frame: i64, seq: u32, nodes: usize, boxes: usize,
    view: (i32, i32), lines: Vec<String>,
}
// Browser and all child documents execute on this one browser thread.
static mut CACHES: [Option<Cache>; SLOTS] = [const { None }; SLOTS];
struct Evals { owner: u64, frame: i64, total: u32, ok: u32, thrown: u32, trapped: u32, other: u32, lines: Vec<String> }
static mut EVALS: [Option<Evals>; SLOTS] = [const { None }; SLOTS];

fn document(doc: &Doc) -> &super::Document {
    match doc { Doc::Plain(d) => d, Doc::Scripted(s) => s.page() }
}
fn text(s: &str) -> String {
    s.chars().take(80).map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '?' }).collect()
}
fn connected(dom: &Dom, mut n: NodeId) -> Option<bool> {
    for _ in 0..ANCESTORS {
        if n == Dom::ROOT { return Some(true); }
        n = match dom.nodes.get(n)?.parent { Some(p) => p, None => return Some(false) };
    }
    None
}
fn chosen(dom: &Dom) -> Vec<NodeId> {
    let mut out = Vec::new();
    for n in 0..dom.nodes.len().min(SCAN) {
        if connected(dom, n) != Some(true) { continue; }
        let tag = dom.tag(n).unwrap_or("");
        let id = dom.attr(n, "id").unwrap_or("");
        // Include semantic controls and inline vectors, so a zero-size clickable
        // control can be explained using its actual cached computed style.
        // Selection remains bounded and does not read JS properties/getters.
        if !matches!(tag, "html" | "body" | "iframe" | "video" | "audio" | "button" | "svg")
            && dom.attr(n, "role") != Some("button") && !id.contains("player") { continue; }
        let mut p = Some(n);
        for _ in 0..ANCESTORS {
            let Some(at) = p else { break };
            if at >= dom.nodes.len() { break; }
            if !out.contains(&at) && out.len() < CHOSEN { out.push(at); }
            p = dom.nodes.get(at).and_then(|q| q.parent);
        }
        if out.len() == CHOSEN { break; }
    }
    out
}
fn slot(b: &Browser) -> Option<usize> {
    match b.in_frame { None => Some(0), Some(n) => b.frames.iter().position(|f| f.node == n).map(|i| i + 1) }
}
pub(super) fn context(b: &Browser) -> Option<(usize,i64)> {
    slot(b).filter(|i|*i<SLOTS).map(|i|(i,b.in_frame.map_or(-1,|n|n as i64)))
}
/// Classify the existing evaluation's returned status only. No exception reason
/// lookup and no extra evaluation; opaque/handled exceptions remain opaque.
pub(super) fn outcome(context: Option<(usize,i64)>, owner: u64, source: &str, result: &Result<(),sex_js::Error>) {
    let Some((at,frame))=context else{return};
    if at>=SLOTS{return;}
    let all=unsafe{&mut *core::ptr::addr_of_mut!(EVALS)};
    if !all[at].as_ref().is_some_and(|e|e.owner==owner&&e.frame==frame) {
        all[at]=Some(Evals{owner,frame,total:0,ok:0,thrown:0,trapped:0,other:0,lines:Vec::new()});
    }
    let e=all[at].as_mut().unwrap();e.total=e.total.saturating_add(1);
    let kind=match result {
        Ok(())=>{e.ok=e.ok.saturating_add(1);"ok"},
        Err(sex_js::Error::Thrown)=>{e.thrown=e.thrown.saturating_add(1);"thrown"},
        Err(sex_js::Error::Trapped(_))=>{e.trapped=e.trapped.saturating_add(1);"trapped"},
        _=>{e.other=e.other.saturating_add(1);"other"},
    };
    if e.lines.len()<32 {e.lines.push(format!("[web.live.eval-result] owner={owner} frame={frame} result={kind} reason=unavailable src={}",text(source)));}
}
/// Cache only computed values that the real relayout just used. No synthetic
/// final recomputation: stale/missing owner-bound caches are unavailable.
pub(super) fn record_layout(b: &Browser, styles: &[Option<Style>]) {
    let Some(at) = slot(b) else { return };
    if at >= SLOTS { return; }
    let d = document(&b.doc);
    let owner = d.diagnostic_owner();
    let frame = b.in_frame.map_or(-1, |n| n as i64);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let mut lines = Vec::new();
    for n in chosen(&d.dom) {
        let Some(s) = styles.get(n).and_then(Option::as_ref) else { continue };
        lines.push(format!("[web.live.style] owner={owner} frame={frame} layout={seq} node={n} display={:?} css_hidden={} opacity={} position={:?} width={:?} height={:?} min_w={:?} max_w={:?} min_h={:?} overflow_clip={}", s.display, s.hidden as u8, s.opacity, s.position, s.width, s.height, s.min_width, s.max_width, s.min_height, s.overflow_clip as u8));
        lines.push(format!("[web.live.offsets] owner={owner} frame={frame} layout={seq} node={n} inset={:?} margin={:?}", s.inset, s.margin));
        lines.push(format!("[web.live.edges] owner={owner} frame={frame} layout={seq} node={n} padding={:?} border={:?}", s.padding, s.border));
        lines.push(format!("[web.live.container] owner={owner} frame={frame} layout={seq} node={n} flex_column={} flex_wrap={} flex_grow={} flex_basis={:?} background={:x} cb_engine_id=unavailable", s.flex_column as u8, s.flex_wrap as u8, s.flex_grow, s.flex_basis, s.background));
    }
    unsafe { (&mut *core::ptr::addr_of_mut!(CACHES))[at] = Some(Cache {
        owner, frame, seq, nodes: d.dom.nodes.len(), boxes: b.page.boxes.len(), view: b.view, lines,
    }); }
}
pub(super) fn snapshot(b: &Browser) {
    let shot = SHOTS.fetch_add(1, Ordering::Relaxed) + 1;
    if shot > 4 { return; }
    if b.in_frame.is_some() {
        sex_pdx::serial_record!("[web.live.unavailable] shot={} reason=in-frame-swap", shot); return;
    }
    let size = b.window.size();
    sex_pdx::serial_record!("[web.live.start] shot={} target={} task={} window={}x{} view={}x{} frames={} pending_js_jobs=unavailable c_guard_reasons=unavailable pixel_generation=unavailable", shot,b.window.target,b.window.task,size.0,size.1,b.view.0,b.view.1,b.frames.len());
    #[cfg(feature="baseline-jit")]
    sex_pdx::serial_record!("[web.live.jit-mode] shot={} enabled={}",shot,b.jit_enabled as u8);
    raw(&b.doc,&b.page,&b.media,0,-1,b.view,b.scroll,b.script_scroll,b.timer,b.media_loads.len());
    for (i,f) in b.frames.iter().take(super::MAX_FRAMES).enumerate() {
        sex_pdx::serial_record!("[web.live.frame] shot={} node={} rect={:?} view={}x{} items={} height={} frames_shown={} url={}",shot,f.node,f.rect,f.view.0,f.view.1,f.page.items.len(),f.page.height,f.frames_shown,text(&f.url));
        raw(&f.doc,&f.page,&f.media,i+1,f.node as i64,f.view,f.scroll,f.script_scroll,f.timer,f.media_loads.len());
    }
    sex_pdx::serial_record!("[web.live.done] shot={} target={} task={}",shot,b.window.target,b.window.task);
}
#[allow(clippy::too_many_arguments)]
fn raw(doc: &Doc, page: &Page, media: &[super::media::Media], at: usize, frame: i64,
    view: (i32,i32), scroll: i32, script_scroll: i32, timer: Option<u64>, loads: usize) {
    let d = document(doc);
    let owner = d.diagnostic_owner();
    let mut count = 0; let mut unknown = 0;
    for n in 0..d.dom.nodes.len().min(SCAN) {
        match connected(&d.dom,n) { Some(true) => count += 1, None => unknown += 1, _ => {} }
    }
    sex_pdx::serial_record!("[web.live.dom] owner={} frame={} scripted={} nodes={} scan_connected={} scan_unknown={} scan_limit={} dirty={} viewport={}x{} view={}x{} items={} boxes={} page={}x{} bg={:x}",owner,frame,matches!(doc,Doc::Scripted(_))as u8,d.dom.nodes.len(),count,unknown,SCAN,d.dirty as u8,d.viewport.0,d.viewport.1,view.0,view.1,page.items.len(),page.boxes.len(),page.width,page.height,page.background);
    sex_pdx::serial_record!("[web.live.queues] owner={} frame={} scroll={} script_scroll={} timer={} classic={} fetches={} modules={} media_ops={} media_appends={} native_media={} media_loads={} url={}",owner,frame,scroll,script_scroll,timer.is_some()as u8,d.classic_scripts.len(),d.fetches.len(),d.modules_wanted.len(),d.media.len(),d.media_bytes.len(),media.len(),loads,text(&d.url));
    let mut kinds=[0usize;4];
    for i in page.items.iter().take(8192) {
        kinds[match i { super::sex_html::layout::Item::Rect{..}=>0,super::sex_html::layout::Item::Text{..}=>1,super::sex_html::layout::Item::Image{..}=>2,super::sex_html::layout::Item::Bullet{..}=>3 }]+=1;
    }
    sex_pdx::serial_record!("[web.live.paint] owner={} frame={} sampled_kinds={:?} item_limit=8192 links={} clips={} item_clips={} sampled_clipped={}",owner,frame,kinds,page.links.len(),page.clips.len(),page.item_clips.len(),page.item_clips.iter().take(8192).filter(|c|**c!=0).count());
    for (i,rect) in page.clips.iter().take(16).enumerate() {
        sex_pdx::serial_record!("[web.live.clip] owner={} frame={} clip={} rect={:?}",owner,frame,i,rect);
    }
    for (i,link) in page.links.iter().take(16).enumerate() {
        sex_pdx::serial_record!("[web.live.link] owner={} frame={} link={} node={} rect={:?} href={}",owner,frame,i,link.node,[link.x,link.y,link.w,link.h],text(&link.href));
    }
    let evals=unsafe{&*core::ptr::addr_of!(EVALS)};
    if let Some(e)=evals.get(at).and_then(Option::as_ref).filter(|e|e.owner==owner&&e.frame==frame) {
        sex_pdx::serial_record!("[web.live.evals] owner={} frame={} total={} ok={} thrown={} trapped={} other={} retained={} scope=initial-and-classic-eval callbacks=unavailable reasons=unavailable",owner,frame,e.total,e.ok,e.thrown,e.trapped,e.other,e.lines.len());
        for l in &e.lines {sex_pdx::serial_record!("{}",l);}
    } else {sex_pdx::serial_record!("[web.live.evals] owner={} frame={} status=unavailable-no-record",owner,frame);}
    #[cfg(feature="baseline-jit")]
    if let Doc::Scripted(s) = doc {
        let stats = s.jit_stats();
        sex_pdx::serial_record!("[web.live.jit] owner={} frame={} compiled={} executed={} bailouts={} seal_refused={} scope=host-jit-cache c_guard_skips=unavailable",owner,frame,stats.compiled,stats.executed,stats.bailouts,stats.refused);
        let candidates=s.diagnostic_jit_candidates();
        sex_pdx::serial_record!("[web.live.candidates] owner={} frame={} entries={} capacity=64 hot_threshold=32 key=bytecode-plus-arity functions=unavailable preguard_calls=unavailable",owner,frame,candidates.len());
        for (i,c) in candidates.iter().take(64).enumerate() {
            sex_pdx::serial_record!("[web.live.candidate] owner={} frame={} entry={} argc={} calls={} saturated={} compiled={} byte_len={} uncompiled_reason=unavailable",owner,frame,i,c.argc,c.calls,(c.calls==u16::MAX)as u8,c.compiled as u8,c.bytecode.len());
            for (part,bytes) in c.bytecode.chunks(64).enumerate() {
                let mut hex=String::new();for byte in bytes {use core::fmt::Write;let _=write!(hex,"{:02x}",byte);}
                sex_pdx::serial_record!("[web.live.bytecode] owner={} frame={} entry={} part={} hex={}",owner,frame,i,part,hex);
            }
        }
    } else {
        sex_pdx::serial_record!("[web.live.jit] owner={} frame={} status=unavailable-not-scripted",owner,frame);
    }
    #[cfg(not(feature="baseline-jit"))]
    sex_pdx::serial_record!("[web.live.jit] owner={} frame={} status=unavailable-feature-off",owner,frame);
    let caches = unsafe { &*core::ptr::addr_of!(CACHES) };
    if let Some(c) = caches.get(at).and_then(Option::as_ref).filter(|c| {
        let extent_ok=c.nodes==d.dom.nodes.len() || (d.dom.nodes.len()>=c.nodes&&d.dom.nodes.len()<=SCAN&&(c.nodes..d.dom.nodes.len()).all(|n|connected(&d.dom,n)==Some(false)));
        c.owner==owner&&c.frame==frame&&extent_ok&&c.boxes==page.boxes.len()&&c.view==view&&!d.dirty
    }) {
        sex_pdx::serial_record!("[web.live.layout] owner={} frame={} layout={} status=last-actual-layout rows={} layout_nodes={} current_nodes={} styles_recomputed=0",owner,frame,c.seq,c.lines.len(),c.nodes,d.dom.nodes.len());
        for l in &c.lines { sex_pdx::serial_record!("{}", l); }
    } else {
        sex_pdx::serial_record!("[web.live.layout] owner={} frame={} status=unavailable-stale-or-missing styles_recomputed=0",owner,frame);
    }
    for n in chosen(&d.dom) {
        let parent=d.dom.nodes[n].parent;
        sex_pdx::serial_record!("[web.live.node] owner={} frame={} node={} parent={:?} tag={} id={} children={} box={:?} parent_box={:?}",owner,frame,n,parent,d.dom.tag(n).unwrap_or(""),text(d.dom.attr(n,"id").unwrap_or("")),d.dom.nodes[n].children.len(),page.boxes.get(n).copied().flatten(),parent.and_then(|p|page.boxes.get(p).copied().flatten()));
        sex_pdx::serial_record!("[web.live.attrs] owner={} frame={} node={} class={} style={} hidden_attr={} width={} height={} src={}",owner,frame,n,text(d.dom.attr(n,"class").unwrap_or("")),text(d.dom.attr(n,"style").unwrap_or("")),d.dom.attr(n,"hidden").is_some()as u8,text(d.dom.attr(n,"width").unwrap_or("")),text(d.dom.attr(n,"height").unwrap_or("")),text(d.dom.attr(n,"src").unwrap_or("")));
        sex_pdx::serial_record!("[web.live.control] owner={} frame={} node={} role={} aria_label={} title={} disabled_attr={}",owner,frame,n,text(d.dom.attr(n,"role").unwrap_or("")),text(d.dom.attr(n,"aria-label").unwrap_or("")),text(d.dom.attr(n,"title").unwrap_or("")),d.dom.attr(n,"disabled").is_some()as u8);
    }
    let mut scripts=0;
    for n in 0..d.dom.nodes.len().min(SCAN) {
        if d.dom.tag(n)!=Some("script")||connected(&d.dom,n)!=Some(true){continue;}
        let src=d.dom.attr(n,"src").unwrap_or("");
        if src.is_empty(){continue;}
        let policy=if src.len()<=2048{Some(d.diagnostic_source_policy(src))}else{None};
        sex_pdx::serial_record!("[web.live.source] owner={} frame={} node={} crossorigin={} policy_lookup={:?} effective_c_runtime=unavailable src={}",owner,frame,n,d.dom.attr(n,"crossorigin").is_some()as u8,policy,text(src));
        scripts+=1;if scripts==16{break;}
    }
    for m in media.iter().take(16) {
        sex_pdx::serial_record!("[web.live.media] owner={} frame={} node={} state={:?} native_player={} want_play={} frames={} size={:?} mse={} muted={} volume={} events={} pending={:?} js_properties=unavailable src={}",owner,frame,m.node,m.state,m.player.is_some()as u8,m.want_play as u8,m.frames,m.size,m.mse.is_some()as u8,m.muted as u8,m.volume,m.events.len(),m.pending,text(&m.src));
    }
}
