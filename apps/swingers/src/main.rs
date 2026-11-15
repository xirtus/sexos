// SPDX-License-Identifier: GPL-3.0-only
//! Swingers, the SexOS BitTorrent client. This program does the I/O; the protocols are
//! in `sex_torrent`, which never touches a socket or a file itself.
//!
//! `run +net swingers get MAGNET|FILE.torrent [--dir DIR] [--peer HOST:PORT]... [--seed-secs N] [--port P] [--anonymous]`
//! downloads a torrent, checks every piece, writes the files and seeds for N seconds.
//! `swingers create PATH --out FILE.torrent [--v1|--v2|--hybrid] [--tracker URL]` makes one.
//!
//! Progress is printed and recorded as `[swingers.*]` lines (the gates read them).
mod disk;
mod net;
mod remote;

use sex_torrent::create::{Creator, FileSpec, Options, Version};
use sex_torrent::magnet::{Magnet, MagnetPeer};
use sex_torrent::metainfo::Metainfo;
use sex_torrent::session::{TorrentConfig, TorrentParams};
use sex_torrent::{PeerAddr, PeerId};
use std::path::{Path, PathBuf};

#[cfg(target_os = "hermit")]
use sex_std as _;

/// A line for the person and, on SexOS, for the serial record the gates read.
pub fn say(line: &str) {
    println!("{line}");
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
}

/// Random bytes from the CPU's generator (RDRAND), mixed with the clock when it is missing.
pub fn random(out: &mut [u8]) {
    let has = std::arch::is_x86_feature_detected!("rdrand");
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
    for chunk in out.chunks_mut(8) {
        let mut v = 0u64;
        // SAFETY: only called when the CPU reports RDRAND.
        if has { for _ in 0..10 { if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 { break; } } }
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v ^= seed;
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
}

fn usage() -> ! {
    say("usage: swingers get MAGNET|FILE.torrent [--dir DIR] [--peer HOST:PORT]... [--seed-secs N] [--port P] [--anonymous]");
    say("       swingers daemon [--dir DIR] [--port P] [--rpc PORT] [--user U --password P]");
    say("       swingers create PATH --out FILE.torrent [--v1|--v2|--hybrid] [--tracker URL]");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") => get(&args[1..]),
        Some("create") => create(&args[1..]),
        Some("daemon") => daemon(&args[1..]),
        _ => usage(),
    }
}

/// Resolves `HOST:PORT` (or `[v6]:PORT`) to peer addresses.
pub fn resolve(host: &str, port: u16) -> Vec<PeerAddr> {
    use std::net::ToSocketAddrs;
    (host, port).to_socket_addrs().map(|it| it.map(net::to_peer).collect()).unwrap_or_default()
}

fn get(args: &[String]) {
    let (mut source, mut dir, mut peers, mut seed_secs, mut port, mut anonymous) = (None, PathBuf::from("."), Vec::new(), 0u64, 6881u16, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = PathBuf::from(it.next().unwrap_or_else(|| usage())),
            "--peer" => {
                let p = it.next().unwrap_or_else(|| usage());
                let (h, n) = p.rsplit_once(':').unwrap_or_else(|| usage());
                peers.extend(resolve(h.trim_matches(|c| c == '[' || c == ']'), n.parse().unwrap_or_else(|_| usage())));
            }
            "--seed-secs" => seed_secs = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()),
            "--port" => port = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()),
            "--anonymous" => anonymous = true,
            _ if source.is_none() => source = Some(a.clone()),
            _ => usage(),
        }
    }
    let source = source.unwrap_or_else(|| usage());
    let peer_id = PeerId::generate(anonymous, &mut |b: &mut [u8]| random(b));
    let mut client = net::Client::new(port, dir, peer_id, anonymous);
    if let Err(e) = client.add_source(&source, None, peers, None) { say(&format!("[swingers.failed] {e}")); std::process::exit(1) }
    client.run(Some(seed_secs));
}

/// A torrent ready to add: what the engine needs plus what the program keeps.
pub struct Prepared { pub params: TorrentParams, pub meta: Option<Metainfo>, pub trackers: Vec<String>, pub peers: Vec<PeerAddr>, pub name: String }

/// Reads a magnet link, a bare info-hash or a .torrent (path or bytes) into a torrent.
pub fn prepare(source: &str, bytes: Option<Vec<u8>>, peer_id: PeerId, port: u16, anonymous: bool) -> Result<Prepared, String> {
    let mut source = String::from(source);
    // A bare 40-hex info-hash is a magnet link without trackers.
    if source.len() == 40 && sex_torrent::from_hex::<20>(&source).is_some() { source = format!("magnet:?xt=urn:btih:{source}"); }
    let mut config = TorrentConfig { listen_port: Some(port), anonymous, ..TorrentConfig::default() };
    if anonymous { config.pex = false; config.client = String::new(); }
    let mut seed = [0u8; 8];
    random(&mut seed);
    let mut out = if bytes.is_none() && source.starts_with("magnet:") {
        let m = Magnet::parse(&source).map_err(|e| format!("bad magnet: {e:?}"))?;
        let mut peers = Vec::new();
        for p in &m.peers {
            match p { MagnetPeer::Addr(a) => peers.push(*a), MagnetPeer::Host(h, n) => peers.extend(resolve(h, *n)) }
        }
        let mut p = TorrentParams::new(m.info_hashes, peer_id);
        p.config = config;
        let name = m.display_name.clone().unwrap_or_else(|| sex_torrent::to_hex(&m.info_hashes.wire()));
        Prepared { params: p, meta: None, trackers: m.trackers.clone(), peers, name }
    } else {
        let bytes = match bytes { Some(b) => b, None => std::fs::read(&source).map_err(|e| format!("cannot read {source}: {e}"))? };
        let m = Metainfo::parse(&bytes).map_err(|e| format!("not a torrent: {e:?}"))?;
        let mut p = TorrentParams::new(m.info_hashes, peer_id);
        p.layout = Some(m.layout().map_err(|e| format!("layout: {e:?}"))?);
        p.verifier = Some(m.verifier());
        p.info = Some(m.info_bytes.clone());
        p.check_files = true;
        p.config = config;
        let trackers = m.trackers.iter().flatten().cloned().collect();
        let name = m.name.clone();
        Prepared { params: p, meta: Some(m), trackers, peers: Vec::new(), name }
    };
    out.params.rng_seed = u64::from_le_bytes(seed);
    Ok(out)
}

/// `swingers daemon [--dir DIR] [--port P] [--rpc PORT] [--user U --password P]`: keeps
/// running and takes torrents over Transmission's RPC (phone apps, transmission-remote)
/// and the web UI at /swingers/.
fn daemon(args: &[String]) {
    let (mut dir, mut port, mut rpc, mut user, mut password) = (PathBuf::from("."), 6881u16, 9091u16, String::new(), String::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().unwrap_or_else(|| usage());
        match a.as_str() {
            "--dir" => dir = PathBuf::from(val()),
            "--port" => port = val().parse().unwrap_or_else(|_| usage()),
            "--rpc" => rpc = val().parse().unwrap_or_else(|_| usage()),
            "--user" => user = val(),
            "--password" => password = val(),
            _ => usage(),
        }
    }
    let peer_id = PeerId::generate(false, &mut |b: &mut [u8]| random(b));
    let mut client = net::Client::new(port, dir, peer_id, false);
    if let Err(e) = client.listen_rpc(rpc, &user, &password) { say(&format!("[swingers.failed] rpc: {e}")); std::process::exit(1) }
    say(&format!("[swingers.rpc] listening on port {rpc}"));
    client.run(None);
}

fn create(args: &[String]) {
    let (mut path, mut out, mut version, mut trackers) = (None, None, Version::Hybrid, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().cloned(),
            "--v1" => version = Version::V1,
            "--v2" => version = Version::V2,
            "--hybrid" => version = Version::Hybrid,
            "--tracker" => trackers.push(it.next().cloned().unwrap_or_else(|| usage())),
            _ if path.is_none() => path = Some(PathBuf::from(a)),
            _ => usage(),
        }
    }
    let (path, out) = (path.unwrap_or_else(|| usage()), out.unwrap_or_else(|| usage()));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "torrent".into());
    let mut files = Vec::new();
    disk::walk(&path, &mut Vec::new(), &mut files);
    let single = path.is_file();
    let specs: Vec<FileSpec> = files.iter().map(|(rel, len)| {
        let parts: Vec<&str> = if single { vec![name.as_str()] } else { rel.iter().map(String::as_str).collect() };
        FileSpec::new(&parts, *len)
    }).collect();
    let mut opts = Options::new(&name, version);
    if !trackers.is_empty() { opts.trackers = vec![trackers]; }
    opts.created_by = Some("Swingers".into());
    let mut c = Creator::new(opts, specs).unwrap_or_else(|e| { say(&format!("[swingers.failed] create: {e:?}")); std::process::exit(1) });
    let order: Vec<Vec<String>> = c.files().iter().map(|f| f.path.clone()).collect();
    for rel in order {
        let file = if single { path.clone() } else { rel.iter().fold(path.clone(), |p, c| p.join(c)) };
        let data = std::fs::read(&file).unwrap_or_else(|e| { say(&format!("[swingers.failed] read {}: {e}", file.display())); std::process::exit(1) });
        c.feed(&data).unwrap_or_else(|e| { say(&format!("[swingers.failed] create: {e:?}")); std::process::exit(1) });
    }
    let bytes = c.finish().unwrap_or_else(|e| { say(&format!("[swingers.failed] create: {e:?}")); std::process::exit(1) });
    std::fs::write(&out, &bytes).unwrap_or_else(|e| { say(&format!("[swingers.failed] write {out}: {e}")); std::process::exit(1) });
    let m = Metainfo::parse(&bytes).expect("we just made it");
    let magnet = Magnet::from_metainfo(&m).to_uri();
    say(&format!("[swingers.created] {} {}", Path::new(&out).display(), magnet));
}
