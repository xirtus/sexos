// SPDX-License-Identifier: GPL-3.0-only
//! The embedded web UI: one self-contained page (inline CSS and JavaScript, no external
//! requests) that drives Swingers through the Transmission RPC in [`crate::rpc`].
//!
//! It lists torrents with progress, status, speeds and ratio, adds magnets or URLs,
//! pauses, resumes, verifies and removes (optionally with data), and shows each
//! torrent's files with wanted toggles. Names are always inserted as text, never as
//! HTML. The page is served at `/swingers/` and, for remotes that expect it, at
//! `/transmission/web/`; [`serve`] does that routing and adds strict security headers
//! (a Content-Security-Policy that allows no outside origin, no framing, no sniffing).
//! Authentication is the RPC server's: it runs before routing, so the browser's Basic
//! auth prompt protects the page too.
use alloc::string::String;
use alloc::vec::Vec;

use crate::rpc::Response;

/// The page.
pub const PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Swingers</title>
<style>
:root{--bg:#f6f5f2;--fg:#1d1d1f;--mut:#6b6b70;--card:#fff;--line:#e3e1dc;--acc:#c2410c;--ok:#15803d;--bar:#ececea;--err:#b91c1c}
@media (prefers-color-scheme:dark){:root{--bg:#141416;--fg:#ececef;--mut:#9a9aa2;--card:#1d1d21;--line:#2c2c33;--acc:#fb923c;--ok:#4ade80;--bar:#2a2a30;--err:#f87171}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.4 system-ui,-apple-system,"Segoe UI",sans-serif}
header{display:flex;gap:12px;align-items:center;flex-wrap:wrap;padding:12px 16px;border-bottom:1px solid var(--line);background:var(--card);position:sticky;top:0;z-index:2}
h1{font-size:17px;margin:0 8px 0 0;letter-spacing:.02em}h1 b{color:var(--acc)}
input[type=text]{flex:1;min-width:160px;padding:7px 10px;border:1px solid var(--line);border-radius:7px;background:var(--bg);color:var(--fg)}
button{padding:6px 11px;border:1px solid var(--line);border-radius:7px;background:var(--card);color:var(--fg);cursor:pointer;font:inherit}
button:hover{border-color:var(--acc)}button.p{background:var(--acc);border-color:var(--acc);color:#fff}
#stats{color:var(--mut);font-variant-numeric:tabular-nums;white-space:nowrap}
main{max-width:1100px;margin:0 auto;padding:12px 16px 40px}
.t{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:10px 12px;margin:8px 0}
.r{display:flex;gap:10px;align-items:center}.n{flex:1;font-weight:600;overflow-wrap:anywhere;cursor:pointer}
.m{color:var(--mut);font-size:12.5px;font-variant-numeric:tabular-nums;margin-top:4px;display:flex;gap:14px;flex-wrap:wrap}
.bar{height:6px;background:var(--bar);border-radius:3px;margin-top:7px;overflow:hidden}.bar i{display:block;height:100%;background:var(--acc)}
.done .bar i{background:var(--ok)}.e{color:var(--err)}
.files{margin-top:8px;border-top:1px solid var(--line);padding-top:6px;max-height:360px;overflow:auto}
.files label{display:flex;gap:8px;padding:3px 0;font-size:13px;align-items:baseline}.files span{flex:1;overflow-wrap:anywhere}
.files em{color:var(--mut);font-style:normal;font-variant-numeric:tabular-nums}
#msg{position:fixed;bottom:14px;left:50%;transform:translateX(-50%);background:var(--fg);color:var(--bg);padding:8px 14px;border-radius:8px;display:none}
.empty{color:var(--mut);text-align:center;padding:40px 0}
@media (max-width:600px){.r button{padding:5px 8px}}
</style></head><body>
<header><h1><b>Swingers</b></h1>
<input id="add" type="text" placeholder="Paste a magnet link or .torrent URL" autocomplete="off">
<button class="p" id="addb">Add</button>
<input id="filt" type="text" placeholder="Filter" style="max-width:180px">
<span id="stats"></span></header>
<main id="list"><div class="empty">Connecting...</div></main>
<div id="msg"></div>
<script>
"use strict";
const RPC="/transmission/rpc";let sid="";const open=new Set();const files={};
const $=s=>document.querySelector(s);
const ST=["Stopped","Queued to check","Checking","Queued","Downloading","Queued to seed","Seeding"];
async function rpc(method,args){
 for(let i=0;i<3;i++){
  const r=await fetch(RPC,{method:"POST",headers:{"X-Transmission-Session-Id":sid,"Content-Type":"application/json"},body:JSON.stringify({method,arguments:args||{}})});
  if(r.status===409){sid=r.headers.get("X-Transmission-Session-Id")||"";continue}
  if(!r.ok)throw new Error("HTTP "+r.status);
  const j=await r.json();if(j.result!=="success")throw new Error(j.result);return j.arguments}
 throw new Error("session id refused")}
function say(t){const m=$("#msg");m.textContent=t;m.style.display="block";clearTimeout(say.t);say.t=setTimeout(()=>m.style.display="none",3500)}
function sz(b){if(!(b>0))return"0 B";const u=["B","kB","MB","GB","TB","PB"];let i=0;while(b>=1000&&i<5){b/=1000;i++}return(i?b.toFixed(b<10?2:1):b)+" "+u[i]}
function sp(b){return b>0?sz(b)+"/s":"-"}
function eta(s){if(s<0)return"";if(s<60)return s+"s";if(s<3600)return Math.round(s/60)+"m";if(s<86400)return Math.floor(s/3600)+"h "+Math.round(s%3600/60)+"m";return Math.floor(s/86400)+"d"}
function el(tag,cls,text){const e=document.createElement(tag);if(cls)e.className=cls;if(text!=null)e.textContent=text;return e}
function btn(label,fn){const b=el("button",null,label);b.onclick=e=>{e.stopPropagation();fn().then(refresh).catch(x=>say(x.message))};return b}
const F=["id","name","status","percentDone","rateDownload","rateUpload","eta","totalSize","sizeWhenDone","uploadRatio","peersConnected","error","errorString","isFinished","queuePosition","labels"];
async function refresh(){
 try{
  const a=await rpc("torrent-get",{fields:F});const q=$("#filt").value.toLowerCase();
  const ts=a.torrents.filter(t=>!q||t.name.toLowerCase().includes(q)).sort((x,y)=>x.queuePosition-y.queuePosition);
  let d=0,u=0;a.torrents.forEach(t=>{d+=t.rateDownload;u+=t.rateUpload});
  $("#stats").textContent=a.torrents.length+" torrents ↓ "+sp(d)+" ↑ "+sp(u);
  const list=$("#list");list.textContent="";
  if(!ts.length)list.appendChild(el("div","empty",a.torrents.length?"Nothing matches the filter.":"No torrents yet. Paste a magnet link above."));
  for(const t of ts){
   const c=el("div","t"+(t.percentDone>=1?" done":""));const r=el("div","r");
   const n=el("div","n",t.name||"(fetching metadata)");n.title="Show files";n.onclick=()=>{open.has(t.id)?open.delete(t.id):open.add(t.id);refresh()};r.appendChild(n);
   const stopped=t.status===0;
   r.appendChild(stopped?btn("Resume",()=>rpc("torrent-start",{ids:[t.id]})):btn("Pause",()=>rpc("torrent-stop",{ids:[t.id]})));
   r.appendChild(btn("Verify",()=>rpc("torrent-verify",{ids:[t.id]})));
   r.appendChild(btn("Remove",async()=>{if(!confirm("Remove “"+t.name+"”?"))return;const del=confirm("Also delete the downloaded data?");await rpc("torrent-remove",{ids:[t.id],"delete-local-data":del});open.delete(t.id)}));
   c.appendChild(r);
   const bar=el("div","bar"),fill=el("i");fill.style.width=(t.percentDone*100).toFixed(1)+"%";bar.appendChild(fill);c.appendChild(bar);
   const m=el("div","m");
   [ST[t.status]||"?",(t.percentDone*100).toFixed(1)+"% of "+sz(t.sizeWhenDone),"↓ "+sp(t.rateDownload),"↑ "+sp(t.rateUpload),"ratio "+(t.uploadRatio<0?"-":t.uploadRatio.toFixed(2)),t.peersConnected+" peers",eta(t.eta)].forEach(x=>x&&m.appendChild(el("span",null,x)));
   if(t.labels&&t.labels.length)m.appendChild(el("span",null,"#"+t.labels.join(" #")));
   if(t.error)m.appendChild(el("span","e",t.errorString));
   c.appendChild(m);
   if(open.has(t.id))c.appendChild(fileList(t.id));
   list.appendChild(c)}
 }catch(e){$("#stats").textContent="offline: "+e.message}}
function fileList(id){
 const box=el("div","files");const f=files[id];
 if(!f){box.textContent="Loading files...";rpc("torrent-get",{ids:[id],fields:["id","files","fileStats"]}).then(a=>{files[id]=a.torrents[0]||{files:[],fileStats:[]};refresh()}).catch(x=>say(x.message));return box}
 if(!f.files.length)box.textContent="No file list yet (metadata not received).";
 f.files.forEach((x,i)=>{
  const l=el("label");const cb=el("input");cb.type="checkbox";cb.checked=f.fileStats[i].wanted;
  cb.onchange=()=>{const k=cb.checked?"files-wanted":"files-unwanted";rpc("torrent-set",{ids:[id],[k]:[i]}).then(()=>{f.fileStats[i].wanted=cb.checked}).catch(e=>{cb.checked=!cb.checked;say(e.message)})};
  l.appendChild(cb);l.appendChild(el("span",null,x.name));l.appendChild(el("em",null,(x.length?Math.floor(100*x.bytesCompleted/x.length):100)+"% "+sz(x.length)));box.appendChild(l)});
 setTimeout(()=>{delete files[id]},4000);return box}
async function add(){const v=$("#add").value.trim();if(!v)return;
 try{const a=await rpc("torrent-add",{filename:v});const t=a["torrent-added"]||a["torrent-duplicate"];say((a["torrent-duplicate"]?"Already added: ":"Added: ")+(t&&t.name||v));$("#add").value="";refresh()}catch(e){say(e.message)}}
$("#addb").onclick=add;$("#add").onkeydown=e=>{if(e.key==="Enter")add()};$("#filt").oninput=refresh;
refresh();setInterval(()=>{if(!document.hidden)refresh()},2000);
</script></body></html>
"##;

/// Paths that serve the page.
pub const PAGE_PATHS: &[&str] = &["/swingers/", "/swingers/index.html", "/transmission/web/", "/transmission/web/index.html"];

/// What a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebRoute {
    Page,
    Redirect(&'static str),
    /// `/favicon.ico`: answered with 204 so browsers stop asking.
    NoContent,
    NotFound,
}

/// Routes a request path (a query string is ignored).
pub fn route(path: &str) -> WebRoute {
    let path = path.split(['?', '#']).next().unwrap_or("");
    if PAGE_PATHS.contains(&path) { return WebRoute::Page; }
    match path {
        "" | "/" | "/swingers" => WebRoute::Redirect("/swingers/"),
        "/transmission" | "/transmission/" | "/transmission/web" => WebRoute::Redirect("/transmission/web/"),
        "/favicon.ico" => WebRoute::NoContent,
        _ => WebRoute::NotFound,
    }
}

/// The security headers every UI response carries.
pub const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
    ("X-Frame-Options", "DENY"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-cache"),
];

/// The HTTP response for a UI path, or `None` when the path is not the UI's.
pub fn serve(method: &str, path: &str) -> Option<Response> {
    let r = route(path);
    if r == WebRoute::NotFound { return None; }
    let mut resp = if method != "GET" && method != "HEAD" {
        Response::message(405, "Use GET.").with("Allow", "GET, HEAD")
    } else {
        match r {
            WebRoute::Page => Response::new(200, "text/html; charset=UTF-8", if method == "HEAD" { Vec::new() } else { PAGE.as_bytes().to_vec() }),
            WebRoute::Redirect(to) => Response::message(301, "Moved.").with("Location", to),
            WebRoute::NoContent => Response { status: 204, headers: Vec::new(), body: Vec::new() },
            WebRoute::NotFound => return None,
        }
    };
    for (k, v) in SECURITY_HEADERS { resp.headers.push((String::from(*k), String::from(*v))); }
    Some(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        assert_eq!(route("/swingers/"), WebRoute::Page);
        assert_eq!(route("/swingers/?x=1"), WebRoute::Page);
        assert_eq!(route("/transmission/web/"), WebRoute::Page);
        assert_eq!(route("/"), WebRoute::Redirect("/swingers/"));
        assert_eq!(route("/swingers"), WebRoute::Redirect("/swingers/"));
        assert_eq!(route("/transmission/web"), WebRoute::Redirect("/transmission/web/"));
        assert_eq!(route("/favicon.ico"), WebRoute::NoContent);
        assert_eq!(route("/swingers/../etc/passwd"), WebRoute::NotFound);
        assert_eq!(route("/transmission/rpc"), WebRoute::NotFound);
    }

    #[test]
    fn serving() {
        let r = serve("GET", "/swingers/").unwrap();
        assert_eq!(r.status, 200);
        assert!(r.header("Content-Security-Policy").unwrap().contains("default-src 'none'"));
        assert_eq!(r.header("X-Frame-Options"), Some("DENY"));
        let bytes = r.to_bytes();
        assert!(bytes.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert_eq!(serve("HEAD", "/swingers/").unwrap().body.len(), 0);
        assert_eq!(serve("GET", "/").unwrap().header("Location"), Some("/swingers/"));
        assert_eq!(serve("POST", "/swingers/").unwrap().status, 405);
        assert_eq!(serve("GET", "/favicon.ico").unwrap().status, 204);
        assert!(serve("GET", "/nope").is_none());
    }

    #[test]
    fn page_is_small_and_self_contained() {
        assert!(PAGE.len() < 30 * 1024, "{} bytes", PAGE.len());
        assert!(PAGE.starts_with("<!doctype html>"));
        assert!(!PAGE.contains("http://") && !PAGE.contains("https://"), "no external resources");
        assert!(PAGE.contains("X-Transmission-Session-Id"));
        for m in ["torrent-get", "torrent-add", "torrent-start", "torrent-stop", "torrent-remove", "torrent-set", "files-wanted", "files-unwanted"] {
            assert!(PAGE.contains(m), "{}", m);
        }
        // Names go in as text, not HTML.
        assert!(!PAGE.contains("innerHTML"));
    }
}
