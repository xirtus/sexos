// SPDX-License-Identifier: GPL-3.0-only
//! The window, keys, mouse and network fetches around sex-map.
use sex_map::mvt::{self, Tile};
use sex_map::offline::{self, Kind, Region};
use sex_map::render::{self, Text};
use sex_map::route::{self, Route};
use sex_map::search::{self, Place};
use sex_map::{http, Servers, TileId, View};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::Kind as Pointer;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const START: (f64, f64, f64) = (52.5163, 13.3777, 15.0);
const PANEL: u32 = 0x00FF_FFFF;
const INK: u32 = 0x0022_2222;
const DIM: u32 = 0x0070_7070;
const PICK: u32 = 0x00DD_E8FA;

fn record(line: String) { sex_std::record(&line); }

/// GET over plain HTTP: status and body (inflated when gzip).
fn get(url: &str) -> Result<(u16, Vec<u8>), String> {
    let u = http::parse_url(url).map_err(|e| format!("{url}: {e:?}"))?;
    let addr = (u.host.as_str(), u.port).to_socket_addrs().map_err(|e| format!("{}: {e}", u.host))?.next().ok_or("no address")?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(15)).map_err(|e| format!("{addr}: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    s.write_all(http::request(&u).as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let r = http::parse_response(&raw).map_err(|e| format!("{e:?}"))?;
    let gz = r.header("Content-Encoding").is_some_and(|e| e.eq_ignore_ascii_case("gzip")) || r.body.starts_with(&[0x1F, 0x8B]);
    let body = if gz { sex_map::inflate(&r.body).map_err(|_| "bad gzip".to_string())? } else { r.body };
    Ok((r.status, body))
}

enum Loaded { Vector(Tile, &'static str), Raster(usize, usize, Vec<u32>, &'static str), Missing(String) }

enum Job { Tile(TileId), Search(String), Route((f64, f64), (f64, f64)), Offline(Region) }
enum Done { Tile(TileId, Loaded), Search(String, Result<Vec<Place>, String>), Route(Result<Route, String>), Offline(Region, usize, usize) }

fn raster(bytes: &[u8]) -> Option<(usize, usize, Vec<u32>)> {
    let img = sex_image::decode(bytes, &sex_image::Limits { max_width: 1024, max_height: 1024, max_pixels: 1 << 20, max_input_bytes: 4 << 20 }).ok()?;
    Some((img.width, img.height, img.pixels))
}

fn load_tile(servers: &Servers, t: TileId) -> Loaded {
    if let Ok(b) = std::fs::read(offline::tile_path(t, Kind::Vector)) {
        if let Ok(tile) = mvt::decode(&b) { return Loaded::Vector(tile, "offline"); }
    }
    if let Some((w, h, px)) = std::fs::read(offline::tile_path(t, Kind::Raster)).ok().and_then(|b| raster(&b)) { return Loaded::Raster(w, h, px, "offline"); }
    let why = match get(&Servers::tile_url(&servers.vector, t)) {
        Ok((200, body)) => match mvt::decode(&body) { Ok(tile) => return Loaded::Vector(tile, "network"), Err(e) => format!("vector tile: {e:?}") },
        Ok((status, _)) => format!("vector tile: HTTP {status}"),
        Err(e) => e,
    };
    match get(&Servers::tile_url(&servers.raster, t)) {
        Ok((200, body)) => match raster(&body) { Some((w, h, px)) => Loaded::Raster(w, h, px, "network"), None => Loaded::Missing("raster tile is not an image".into()) },
        Ok((status, _)) => Loaded::Missing(format!("{why}; raster tile: HTTP {status}")),
        Err(e) => Loaded::Missing(format!("{why}; raster tile: {e}")),
    }
}

/// Keeps a tile's file in offline/. Returns whether it is now there.
fn keep_tile(servers: &Servers, t: TileId) -> bool {
    let _ = std::fs::create_dir_all(offline::tile_dir(t));
    if let Ok((200, body)) = get(&Servers::tile_url(&servers.vector, t)) {
        if mvt::decode(&body).is_ok() { return std::fs::write(offline::tile_path(t, Kind::Vector), &body).is_ok(); }
    }
    if let Ok((200, body)) = get(&Servers::tile_url(&servers.raster, t)) {
        if raster(&body).is_some() { return std::fs::write(offline::tile_path(t, Kind::Raster), &body).is_ok(); }
    }
    false
}

fn work(servers: Servers, jobs: Arc<Mutex<Receiver<Job>>>, done: Sender<Done>) {
    loop {
        let job = { let rx = jobs.lock().unwrap_or_else(|e| e.into_inner()); rx.recv() };
        let Ok(job) = job else { return };
        let out = match job {
            Job::Tile(t) => Done::Tile(t, load_tile(&servers, t)),
            Job::Search(q) => {
                let r = match get(&search::url(&servers.search, &q, 8)) {
                    Ok((200, body)) => search::parse(&body).map_err(|_| "the search answer is not a list of places".to_string()),
                    Ok((status, _)) => Err(format!("search: HTTP {status}")),
                    Err(e) => Err(e),
                };
                Done::Search(q, r)
            }
            Job::Route(a, b) => Done::Route(match get(&route::url(&servers.route, "driving", a, b)) {
                Ok((_, body)) => route::parse(&body).map_err(|e| format!("{e:?}")),
                Err(e) => Err(e),
            }),
            Job::Offline(region) => {
                let tiles = region.tiles().unwrap_or_default();
                let kept = tiles.iter().filter(|t| keep_tile(&servers, **t)).count();
                Done::Offline(region, tiles.len(), kept)
            }
        };
        if done.send(out).is_err() { return; }
    }
}

enum Mode { Map, Typing(String), Results }

struct App {
    view: View,
    start: View,
    servers: Servers,
    tiles: HashMap<TileId, Option<Loaded>>,
    jobs: Sender<Job>,
    results: Vec<Place>,
    pick: usize,
    chosen: Vec<Place>,
    route: Option<Route>,
    mode: Mode,
    status: String,
    text: Text,
    drag: Option<(i16, i16)>,
    pixmap: Option<tiny_skia::Pixmap>,
    reported: bool,
}

impl App {
    fn want_tiles(&mut self) -> usize {
        let z = self.view.tile_zoom(self.servers.max_zoom);
        let mut pending = 0;
        for (t, ..) in self.view.tiles(z) {
            match self.tiles.get(&t) {
                None => { self.tiles.insert(t, None); let _ = self.jobs.send(Job::Tile(t)); pending += 1; }
                Some(None) => pending += 1,
                Some(Some(_)) => {}
            }
        }
        // Forget far-away tiles now and then.
        if self.tiles.len() > 160 { let keep: Vec<TileId> = self.view.tiles(z).into_iter().map(|t| t.0).collect(); self.tiles.retain(|t, v| v.is_none() || keep.contains(t)); }
        pending
    }

    fn done(&mut self, d: Done) {
        match d {
            Done::Tile(t, l) => {
                match &l {
                    Loaded::Vector(tile, source) => record(format!("[rendezvous.tile] id={t} kind=vector source={source} layers={} features={}", tile.layers.len(), tile.features())),
                    Loaded::Raster(w, h, _, source) => record(format!("[rendezvous.tile] id={t} kind=raster source={source} size={w}x{h}")),
                    Loaded::Missing(why) => record(format!("[rendezvous.tile] id={t} kind=missing why={why}")),
                }
                self.tiles.insert(t, Some(l));
            }
            Done::Search(q, Ok(places)) => {
                record(format!("[rendezvous.search] query={q} results={} first={}", places.len(), places.first().map_or("", |p| p.short())));
                for (i, p) in places.iter().enumerate() { record(format!("[rendezvous.result] n={} name={} lat={:.5} lon={:.5} kind={}", i + 1, p.short(), p.lat, p.lon, p.kind)); }
                self.status = if places.is_empty() { format!("Nothing found for \"{q}\".") } else { format!("{} places for \"{q}\": Up/Down and Enter.", places.len()) };
                self.results = places;
                self.pick = 0;
                self.mode = if self.results.is_empty() { Mode::Map } else { Mode::Results };
            }
            Done::Search(q, Err(e)) => { record(format!("[rendezvous.search.failed] query={q} why={e}")); self.status = format!("Search failed: {e}"); self.mode = Mode::Map; }
            Done::Route(Ok(r)) => {
                record(format!("[rendezvous.route] points={} distance={:.0} duration={:.0}", r.points.len(), r.distance, r.duration));
                self.status = format!("Route: {:.1} km, {} min.", r.distance / 1000.0, (r.duration / 60.0).round());
                self.route = Some(r);
                self.reported = false;
            }
            Done::Route(Err(e)) => { record(format!("[rendezvous.route.failed] why={e}")); self.status = format!("No route: {e}"); }
            Done::Offline(region, total, kept) => {
                record(format!("[rendezvous.offline] region={} tiles={total} kept={kept} zooms={}-{}", region.name, region.min_zoom, region.max_zoom));
                let mut regions = offline::parse(&std::fs::read_to_string("regions.txt").unwrap_or_default());
                regions.retain(|r| r.name != region.name);
                regions.push(region);
                let _ = std::fs::write("regions.txt", offline::serialize(&regions));
                self.status = format!("Kept {kept} of {total} tiles for offline use.");
            }
        }
    }

    fn go_to(&mut self, p: Place) {
        let (w, h) = (self.view.width, self.view.height);
        self.view = View::new(p.lat, p.lon, self.view.zoom.max(15.0), w, h);
        record(format!("[rendezvous.place] name={} lat={:.5} lon={:.5}", p.short(), p.lat, p.lon));
        self.status = format!("{}. r: route here from {}.", p.short(), self.chosen.last().map_or("the map's centre", |c| c.short()));
        self.chosen.push(p);
        if self.chosen.len() > 2 { self.chosen.remove(0); }
        self.reported = false;
    }

    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        let typed = sex_app::typed(key);
        match &mut self.mode {
            Mode::Typing(q) => {
                match key.code {
                    0x1C => {
                        let q = q.trim().to_string();
                        if q.is_empty() { self.mode = Mode::Map; return; }
                        record(format!("[rendezvous.search.asked] query={q}"));
                        self.status = format!("Searching for \"{q}\"...");
                        let _ = self.jobs.send(Job::Search(q));
                        self.mode = Mode::Map;
                    }
                    0x01 => self.mode = Mode::Map,
                    0x0E => { q.pop(); }
                    _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) { if q.len() < 120 { q.push(c); } },
                }
                return;
            }
            Mode::Results => match (sex_app::nav(key), key.code) {
                (Some(Nav::Up), _) => { self.pick = self.pick.saturating_sub(1); return; }
                (Some(Nav::Down), _) => { self.pick = (self.pick + 1).min(self.results.len().saturating_sub(1)); return; }
                (_, 0x1C) => { let p = self.results[self.pick].clone(); self.mode = Mode::Map; self.go_to(p); return; }
                (_, 0x01) => { self.mode = Mode::Map; return; }
                _ => self.mode = Mode::Map,
            },
            Mode::Map => {}
        }
        let (w, h) = (self.view.width as f64, self.view.height as f64);
        match (sex_app::nav(key), typed, key.code) {
            (Some(Nav::Left), ..) => self.view.pan(w / 5.0, 0.0),
            (Some(Nav::Right), ..) => self.view.pan(-w / 5.0, 0.0),
            (Some(Nav::Up), ..) => self.view.pan(0.0, h / 5.0),
            (Some(Nav::Down), ..) => self.view.pan(0.0, -h / 5.0),
            (Some(Nav::Home), ..) => { let (vw, vh) = (self.view.width, self.view.height); self.view = self.start; self.view.width = vw; self.view.height = vh; }
            (_, Some(b'+' | b'='), _) | (_, _, 0x49) => self.view.zoom_at(1.0, w / 2.0, h / 2.0),
            (_, Some(b'-'), _) | (_, _, 0x51) => self.view.zoom_at(-1.0, w / 2.0, h / 2.0),
            (_, Some(b'/'), _) => { self.mode = Mode::Typing(String::new()); self.status = "Search: type a place and press Enter.".into(); }
            (_, Some(b'r'), _) => {
                let to = self.chosen.last().map(|p| (p.lat, p.lon));
                let from = if self.chosen.len() >= 2 { Some((self.chosen[0].lat, self.chosen[0].lon)) } else { Some(self.view.center()) };
                match (from, to) {
                    (Some(a), Some(b)) => { record(format!("[rendezvous.route.asked] from={:.5},{:.5} to={:.5},{:.5}", a.0, a.1, b.0, b.1)); self.status = "Finding a route...".into(); let _ = self.jobs.send(Job::Route(a, b)); }
                    _ => self.status = "Search for a place first, then press r.".into(),
                }
            }
            (_, Some(b'o'), _) => {
                let (n, wst) = self.view.to_lat_lon(0.0, 0.0);
                let (s, e) = self.view.to_lat_lon(w, h);
                let z = self.view.tile_zoom(self.servers.max_zoom);
                let (c_lat, c_lon) = self.view.center();
                let region = Region { name: format!("{c_lat:.3},{c_lon:.3}"), north: n, west: wst, south: s, east: e, min_zoom: z.saturating_sub(1), max_zoom: z };
                match region.tiles() {
                    Some(t) => { record(format!("[rendezvous.offline.asked] region={} tiles={}", region.name, t.len())); self.status = format!("Keeping {} tiles for offline use...", t.len()); let _ = self.jobs.send(Job::Offline(region)); }
                    None => self.status = "That area is too large to keep: zoom in.".into(),
                }
            }
            (_, _, 0x01) => { self.route = None; self.status.clear(); }
            _ => return,
        }
        self.reported = false;
    }

    fn pointer(&mut self, p: sex_std::sex_pdx::app_pointer::Pointer) -> bool {
        match p.kind {
            Pointer::Down { button: 0 } => { self.drag = Some((p.x, p.y)); false }
            Pointer::Up { button: 0 } | Pointer::Leave => { self.drag = None; false }
            Pointer::Move => match self.drag {
                Some((x, y)) if p.buttons & 1 != 0 => { self.view.pan((p.x - x) as f64, (p.y - y) as f64); self.drag = Some((p.x, p.y)); true }
                _ => false,
            },
            Pointer::Wheel { steps } => { self.view.zoom_at(steps as f64 * 0.5, p.x as f64, p.y as f64); true }
            _ => false,
        }
    }

    fn draw(&mut self, window: &mut sex_app::Window) {
        let (w, h) = window.size();
        let (w, h) = (w.max(1), h.max(1));
        if self.view.width != w as u32 || self.view.height != h as u32 { self.view.width = w as u32; self.view.height = h as u32; }
        let pending = self.want_tiles();
        if self.pixmap.as_ref().is_none_or(|p| p.width() != w as u32 || p.height() != h as u32) { self.pixmap = render::blank(w as u32, h as u32); }
        let Some(pm) = self.pixmap.as_mut() else { return };
        pm.fill(tiny_skia::Color::from_rgba8(0xF2, 0xEF, 0xE9, 255));
        let z = self.view.tile_zoom(self.servers.max_zoom);
        let mut labels = Vec::new();
        let (mut vector, mut rasters) = (0, 0);
        for (t, x, y, size) in self.view.tiles(z) {
            match self.tiles.get(&t) {
                Some(Some(Loaded::Vector(tile, _))) => { labels.extend(render::draw_vector(pm, tile, x, y, size)); vector += 1; }
                Some(Some(Loaded::Raster(tw, th, px, _))) => { render::draw_raster(pm, px, *tw, *th, x, y, size); rasters += 1; }
                _ => {}
            }
        }
        // Count the map's colours before anything covers them.
        let (water, road, building) = (render::count(pm, render::WATER), render::count(pm, render::MAJOR_ROAD), render::count(pm, render::BUILDING));
        if let Some(r) = &self.route { render::draw_route(pm, &self.view, &r.points); }
        let route_px = render::count(pm, render::ROUTE);
        for (i, p) in self.chosen.iter().enumerate() { render::draw_marker(pm, &self.view, p.lat, p.lon, if i + 1 == self.chosen.len() { render::MARKER } else { 0x0026_6BE0 }); }
        let placed = self.text.place(pm, labels);
        // The search box and results, top left.
        let th = self.text.height() as i32;
        let fill = |pm: &mut tiny_skia::Pixmap, x: i32, y: i32, w: i32, h: i32, c: u32| {
            if let Some(r) = tiny_skia::Rect::from_xywh(x as f32, y as f32, w.max(1) as f32, h.max(1) as f32) {
                let mut p = tiny_skia::Paint::default();
                p.set_color_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
                pm.fill_rect(r, &p, tiny_skia::Transform::identity(), None);
            }
        };
        let bw = (w as i32 - 24).min(460);
        match &self.mode {
            Mode::Typing(q) => {
                fill(pm, 12, 12, bw, th + 12, PANEL);
                self.text.draw(pm, &format!("Search: {q}_"), 18, 18, INK, false);
            }
            Mode::Results => {
                fill(pm, 12, 12, bw, (th + 6) * self.results.len() as i32 + 12, PANEL);
                for (i, p) in self.results.iter().enumerate() {
                    let y = 18 + i as i32 * (th + 6);
                    if i == self.pick { fill(pm, 14, y - 3, bw - 4, th + 6, PICK); }
                    let line: String = p.name.chars().take((bw as usize - 16) / self.text.width("m").max(1)).collect();
                    self.text.draw(pm, &line, 18, y, INK, false);
                }
            }
            Mode::Map => {}
        }
        // Status bar.
        let (lat, lon) = self.view.center();
        let bar = format!("{lat:.5}, {lon:.5}  zoom {:.1}  {}", self.view.zoom, self.status);
        fill(pm, 0, h as i32 - th - 10, w as i32, th + 10, PANEL);
        self.text.draw(pm, &bar, 10, h as i32 - th - 5, DIM, false);
        self.text.draw(pm, "(c) OpenStreetMap contributors", (w as i32 - self.text.width("(c) OpenStreetMap contributors") as i32 - 10).max(0), h as i32 - 2 * th - 16, INK, true);
        let out = window.pixels();
        render::to_window(pm, &mut out[..w * h]);
        window.show_pixels(w, h);
        if pending == 0 && !self.reported {
            self.reported = true;
            record(format!("[rendezvous.frame] zoom={:.2} center={lat:.5},{lon:.5} tiles={} vector={vector} raster={rasters} water={water} road={road} building={building} route={route_px} labels={placed} markers={}",
                self.view.zoom, vector + rasters, self.chosen.len()));
        }
    }
}

pub fn main() {
    let mut servers = std::fs::read_to_string("servers.txt").ok().map(|t| Servers::parse(&t));
    let mut start = std::fs::read_to_string("view.txt").ok().and_then(|t| parse_view(t.trim())).unwrap_or(START);
    for a in std::env::args().skip(1) {
        if a.starts_with("http://") || a.starts_with("https://") {
            let s = Servers::at(&a);
            let _ = std::fs::write("servers.txt", s.serialize());
            servers = Some(s);
        } else if let Some(v) = parse_view(&a) { start = v; }
        else { eprintln!("Not a server or LAT,LON[,ZOOM]: {a}"); }
    }
    let Some(servers) = servers else {
        println!("Rendezvous needs a map server.\n\nRun: rendezvous http://YOUR-TILE-SERVER\n(it is kept in servers.txt; see the program's notes for its layout)");
        std::process::exit(1);
    };
    let (target, task) = sex_std::identity();
    record(format!("[rendezvous.ready] target={target} task={task} vector={} search={} route={} start={:.5},{:.5},{:.1} offline_regions={}",
        servers.vector, servers.search, servers.route, start.0, start.1, start.2,
        offline::parse(&std::fs::read_to_string("regions.txt").unwrap_or_default()).len()));
    let (job_tx, job_rx) = channel();
    let (done_tx, done_rx) = channel();
    let jobs = Arc::new(Mutex::new(job_rx));
    for _ in 0..3 { let (s, j, d) = (servers.clone(), jobs.clone(), done_tx.clone()); std::thread::spawn(move || work(s, j, d)); }
    let window = sex_std::window();
    window.title(b"Rendezvous");
    let (w, h) = window.size();
    let view = View::new(start.0, start.1, start.2, w as u32, h as u32);
    let Some(text) = Text::new(8, 16) else { std::process::exit(4) };
    let mut app = App { view, start: view, servers, tiles: HashMap::new(), jobs: job_tx, results: Vec::new(), pick: 0, chosen: Vec::new(), route: None, mode: Mode::Map,
        status: "/ search  r route  o keep offline  arrows move  +/- zoom".into(), text, drag: None, pixmap: None, reported: false };
    app.draw(window);
    let mut last = Instant::now();
    loop {
        let mut redraw = false;
        while let Ok(d) = done_rx.try_recv() { app.done(d); redraw = true; }
        match window.next_within(50) {
            Some(Event::Close) => break,
            Some(Event::Resize { .. }) => { app.reported = false; redraw = true; }
            Some(Event::Key(k)) => { app.key(k); redraw = true; }
            Some(Event::Pointer(p)) => redraw |= app.pointer(p),
            None => {}
        }
        // Results arriving one after another are drawn together, a few times a second.
        if redraw || (!app.reported && last.elapsed().as_millis() > 400) { app.draw(window); last = Instant::now(); }
    }
    let (lat, lon) = app.view.center();
    let _ = std::fs::write("view.txt", format!("{lat:.6},{lon:.6},{:.2}\n", app.view.zoom));
    record("[rendezvous.closed]".into());
    std::process::exit(0);
}

fn parse_view(s: &str) -> Option<(f64, f64, f64)> {
    let mut it = s.split(',').map(|p| p.trim().parse::<f64>());
    let lat = it.next()?.ok()?;
    let lon = it.next()?.ok()?;
    let zoom = match it.next() { Some(z) => z.ok()?, None => START.2 };
    ((-85.0..=85.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon, zoom))
}

