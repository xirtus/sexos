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

use sex_torrent::create::{Creator, FileSpec, Options, Version};
use sex_torrent::magnet::{Magnet, MagnetPeer};
use sex_torrent::metainfo::Metainfo;
use sex_torrent::session::{TorrentConfig, TorrentParams};
use sex_torrent::{InfoHashes, PeerAddr, PeerId};
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
    say("       swingers create PATH --out FILE.torrent [--v1|--v2|--hybrid] [--tracker URL]");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") => get(&args[1..]),
        Some("create") => create(&args[1..]),
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
    let mut source = source.unwrap_or_else(|| usage());
    // A bare 40-hex info-hash is a magnet link without trackers.
    if source.len() == 40 && sex_torrent::from_hex::<20>(&source).is_some() { source = format!("magnet:?xt=urn:btih:{source}"); }
    let peer_id = PeerId::generate(anonymous, &mut |b: &mut [u8]| random(b));
    let mut config = TorrentConfig { listen_port: Some(port), anonymous, ..TorrentConfig::default() };
    if anonymous { config.pex = false; config.client = String::new(); }
    let mut seed = [0u8; 8];
    random(&mut seed);

    let (params, meta, trackers, name) = if source.starts_with("magnet:") {
        let m = Magnet::parse(&source).unwrap_or_else(|e| { say(&format!("[swingers.failed] bad magnet: {e:?}")); std::process::exit(1) });
        for p in &m.peers {
            match p { MagnetPeer::Addr(a) => peers.push(*a), MagnetPeer::Host(h, n) => peers.extend(resolve(h, *n)) }
        }
        let mut p = TorrentParams::new(m.info_hashes, peer_id);
        p.config = config;
        let name = m.display_name.clone().unwrap_or_else(|| sex_torrent::to_hex(&m.info_hashes.wire()));
        (p, None, m.trackers.clone(), name)
    } else {
        let bytes = std::fs::read(&source).unwrap_or_else(|e| { say(&format!("[swingers.failed] cannot read {source}: {e}")); std::process::exit(1) });
        let m = Metainfo::parse(&bytes).unwrap_or_else(|e| { say(&format!("[swingers.failed] not a torrent: {e:?}")); std::process::exit(1) });
        let mut p = TorrentParams::new(m.info_hashes, peer_id);
        p.layout = Some(m.layout().unwrap_or_else(|e| { say(&format!("[swingers.failed] layout: {e:?}")); std::process::exit(1) }));
        p.verifier = Some(m.verifier());
        p.info = Some(m.info_bytes.clone());
        p.check_files = true;
        p.config = config;
        let trackers = m.trackers.iter().flatten().cloned().collect();
        let name = m.name.clone();
        (p, Some(m), trackers, name)
    };
    let mut params = params;
    params.rng_seed = u64::from_le_bytes(seed);
    say(&format!("[swingers.added] {} {}", sex_torrent::to_hex(&params.info_hashes.wire()), name));
    let hashes: InfoHashes = params.info_hashes;
    let mut client = net::Client::new(port, dir, peer_id, anonymous);
    client.add(params, meta, trackers, peers);
    client.run(seed_secs);
    let _ = hashes;
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
