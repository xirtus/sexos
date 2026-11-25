// SPDX-License-Identifier: GPL-3.0-only
use sex_std::{sex_app::{self, Event}, sex_pdx::{self, pdf_sign as p}};
use shibari::{Keeper, Knot, Owner};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};
use crate::source;
use std::sync::mpsc::{self, Receiver, TryRecvError};

type Check = Receiver<Result<[u8; 32], source::Failure>>;

enum Session {
    Idle,
    Collect { client: u32, len: usize, data: Vec<u8>, next: usize, path_len: usize, field_len: usize, meta: Vec<u8>, meta_next: usize },
    Select { client: u32, data: Vec<u8>, source_path: String, field: String, ticket: Option<sex_app::files::PickTicket> },
    Checking { client: u32, data: Vec<u8>, source_path: String, field: String, result: Check },
    Approval { client: u32, data: Vec<u8>, source_path: String, field: String, source_hash: [u8; 32] },
    Recheck { data: Vec<u8>, source_hash: [u8; 32], result: Check },
    Signed(Vec<u8>),
    Refused,
}

fn hex_digit(c: u8) -> Option<u8> {
    match c { b'0'..=b'9' => Some(c - b'0'), b'a'..=b'f' => Some(c - b'a' + 10), b'A'..=b'F' => Some(c - b'A' + 10), _ => None }
}

fn scalar(input: &str) -> Option<Zeroizing<[u8; 32]>> {
    if input.len() != 64 { return None; }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, pair) in input.as_bytes().chunks_exact(2).enumerate() {
        key[i] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(key)
}

fn show(window: &mut sex_app::Window, fingerprint: &str, ready: bool, entered: usize, session: &Session) {
    let text = if !ready {
        format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nEnter the 64 hex digits of its P-256 private scalar here.\nThe scalar stays in this Shibari run and is never saved.\nEntered: {entered}/64    Enter loads    Backspace corrects\n")
    } else {
        match session {
            Session::Idle => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nKey loaded in memory. Open a PDF in Vellum and choose a digital signature field.\nEach signing request will be shown here for approval."),
            Session::Collect { .. } => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nReceiving a bounded Vellum signing request..."),
            Session::Select { source_path, field, .. } => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nChoose the saved PDF in Loom.\nRequested file: {source_path}\nSignature field: {field}\nOnly the exact selected file can be signed."),
            Session::Checking { .. } => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nChecking the chosen PDF and signature field..."),
            Session::Approval { client, source_path, field, source_hash, .. } => {
                let hash = source_hash.iter().map(|b| format!("{b:02X}")).collect::<String>();
                format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nProgram row {client} requests one PDF signature.\nFile: {source_path}\nField: {field}\nPDF SHA-256: {hash}\nPress Y to approve this exact saved file, N to refuse it.")
            },
            Session::Recheck { .. } => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nRechecking the saved PDF before signing..."),
            Session::Signed(_) => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nOne signature approved and returned. Open Vellum to see its result."),
            Session::Refused => format!("Shibari PDF signing\nCertificate SHA-256: {fingerprint}\n\nRequest refused. A new request can be started in Vellum."),
        }
    };
    window.show(text.as_bytes());
}

fn check_in_worker(path: &str, field: &str, data: &[u8], cert: &[u8], point: &[u8]) -> Option<Check> {
    let (tx, rx) = mpsc::channel();
    let (path, field, data, cert, point) = (path.to_owned(), field.to_owned(), data.to_vec(), cert.to_vec(), point.to_vec());
    std::thread::Builder::new().name("shibari-pdf-check".into()).spawn(move || {
        let answer = std::fs::read(&path).map_err(|_| source::Failure::Document)
            .and_then(|bytes| source::verify(&path, &path, &bytes, &field, &cert, &point, &data));
        let _ = tx.send(answer);
    }).ok()?;
    Some(rx)
}

fn receive(message: &sex_pdx::PdxMessage, cert: &[u8], point: &[u8], keeper: &Keeper, owner: Owner, knot: Knot, session: &mut Session) -> Option<u64> {
    if message.caller_pd != 3 || message.caller_task != 3 { return None; }
    let answer = match message.type_id {
        p::CERT => cert.chunks(8).nth(message.arg0 as usize).map(p::pack).unwrap_or(0),
        p::POINT => point.chunks(8).nth(message.arg0 as usize).map(p::pack).unwrap_or(0),
        // Shell owns the one active request. A prior client may have died while
        // Shibari was collecting or awaiting approval; a fresh BEGIN replaces it.
        p::BEGIN if message.arg1 > 0 && message.arg1 <= p::MAX_ATTRS as u64
            && p::decode_metadata_lengths(message.arg2).is_some() => {
            let (path_len, field_len) = p::decode_metadata_lengths(message.arg2).unwrap();
            *session = Session::Collect { client: message.arg0 as u32, len: message.arg1 as usize, data: Vec::with_capacity(message.arg1 as usize), next: 0,
                path_len, field_len, meta: Vec::with_capacity(path_len + field_len), meta_next: 0 };
            0
        }
        p::META => match session {
            Session::Collect { path_len, field_len, meta, meta_next, next, .. }
                if *next == 0 && message.arg0 as usize == *meta_next && *meta_next * 8 < *path_len + *field_len => {
                let n = (*path_len + *field_len - meta.len()).min(8);
                meta.extend_from_slice(&message.arg1.to_le_bytes()[..n]);
                *meta_next += 1;
                0
            }
            _ => 1,
        },
        p::CHUNK => match session {
            Session::Collect { len, data, next, path_len, field_len, meta, .. }
                if meta.len() == *path_len + *field_len && message.arg0 as usize == *next && *next * 8 < *len => {
                let n = (*len - data.len()).min(8);
                data.extend_from_slice(&message.arg1.to_le_bytes()[..n]);
                *next += 1;
                0
            }
            _ => 1,
        },
        p::FINISH => {
            let old = core::mem::replace(session, Session::Idle);
            match old {
                Session::Collect { client, len, data, path_len, field_len, meta, .. }
                    if data.len() == len && meta.len() == path_len + field_len => {
                    match (core::str::from_utf8(&meta[..path_len]), core::str::from_utf8(&meta[path_len..])) {
                        (Ok(path), Ok(field)) if source::canonical_path(path) && !field.is_empty()
                            && source::safe_label(field) => {
                            *session = Session::Select { client, data, source_path: path.into(), field: field.into(), ticket: None }; 0
                        }
                        _ => { *session = Session::Refused; 1 }
                    }
                }
                other => { *session = other; 1 }
            }
        }
        p::STATUS => match session { Session::Signed(signature) => signature.len() as u64, Session::Refused => p::REFUSED, _ => 0 },
        p::SIGNATURE => match session { Session::Signed(signature) => signature.chunks(8).nth(message.arg0 as usize).map(p::pack).unwrap_or(0), _ => 0 },
        p::END => { *session = Session::Idle; 0 }
        _ => return None,
    };
    let _ = (keeper, owner, knot);
    Some(answer)
}

/// Files grants Shibari its own read of the person's selection. Poll once per
/// event-loop turn so Shell STATUS messages remain serviced while Loom is open.
fn drive_selection(window: &mut sex_app::Window, cert: &[u8], point: &[u8], session: &mut Session) -> bool {
    let Session::Select { source_path, ticket, .. } = session else { return false };
    let Some(ticket_value) = *ticket else {
        match sex_app::files::pick_begin(window, sex_app::files::Pick::Open, source_path.as_bytes()) {
            Ok(opened) => *ticket = Some(opened),
            Err(_) => *session = Session::Refused,
        }
        return true;
    };
    let mut picked = [0u8; sex_pdx::fs::PATH_BYTES];
    match sex_app::files::pick_poll(window, ticket_value, &mut picked) {
        Ok(None) => false,
        Ok(Some(path)) => {
            let old = core::mem::replace(session, Session::Refused);
            if let Session::Select { client, data, source_path, field, .. } = old {
                if let Ok(selected) = core::str::from_utf8(path) {
                    if selected == source_path && source::canonical_path(selected) {
                        if let Some(result) = check_in_worker(selected, &field, &data, cert, point) {
                            *session = Session::Checking { client, data, source_path, field, result };
                        }
                    }
                }
            }
            true
        }
        Err(_) => { *session = Session::Refused; true }
    }
}

fn drive_check(session: &mut Session, keeper: &Keeper, owner: Owner, knot: Knot) -> bool {
    match session {
        Session::Checking { result, .. } => match result.try_recv() {
            Err(TryRecvError::Empty) => false,
            result => {
                let old = core::mem::replace(session, Session::Refused);
                if let (Session::Checking { client, data, source_path, field, .. }, Ok(Ok(source_hash))) = (old, result) {
                    *session = Session::Approval { client, data, source_path, field, source_hash };
                }
                sex_std::record(&format!("[shibari.pdf.document] matched={}", matches!(session, Session::Approval { .. }) as u8));
                true
            }
        },
        Session::Recheck { result, .. } => match result.try_recv() {
            Err(TryRecvError::Empty) => false,
            result => {
                let old = core::mem::replace(session, Session::Refused);
                if let (Session::Recheck { data, source_hash, .. }, Ok(Ok(current_hash))) = (old, result) {
                    if current_hash == source_hash {
                        *session = keeper.sign_p256_sha256(owner, knot, &data).ok()
                            .filter(|v| !v.is_empty() && v.len() <= p::MAX_SIGNATURE)
                            .map_or(Session::Refused, Session::Signed);
                    }
                }
                sex_std::record(&format!("[shibari.pdf.approved] ok={}", matches!(session, Session::Signed(_)) as u8));
                true
            }
        },
        _ => false,
    }
}

fn drive_progress(window: &mut sex_app::Window, fingerprint: &str, cert: &[u8], point: &[u8],
                  keeper: &Keeper, owner: Owner, knot: Option<Knot>, session: &mut Session) {
    if let Some(knot) = knot {
        if drive_selection(window, cert, point, session) || drive_check(session, keeper, owner, knot) {
            show(window, fingerprint, true, 0, session);
        }
    }
}

pub fn main() {
    let path = std::env::args().nth(1).unwrap_or_default();
    let cert = std::fs::read(&path).unwrap_or_default();
    let window = sex_std::window();
    window.title(b"Shibari");
    if cert.is_empty() || cert.len() > p::MAX_CERT || vellum_edit::cms::certificate_public_point(&cert).is_err() {
        window.show(b"Shibari needs one valid P-256 DER certificate.\nRun shibari-sign @certificate.der");
        loop { if matches!(window.next(), Event::Close) { std::process::exit(1); } }
    }
    let cert_point = vellum_edit::cms::certificate_public_point(&cert).unwrap();
    let (target, _) = sex_std::identity();
    let owner = Owner(target);
    let mut keeper = Keeper::new(sex_pdx::get_ticks() ^ target as u64, 1);
    let mut input = Zeroizing::new(String::new());
    let mut knot = None;
    let mut point = Vec::new();
    let digest = Sha256::digest(&cert);
    let fingerprint = digest.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let mut session = Session::Idle;
    show(window, &fingerprint, false, 0, &session);
    loop {
        let Some(message) = sex_app::mailbox::take(|m| m.caller_pd == 3).or_else(|| sex_pdx::pdx_try_listen_raw(0)) else {
            drive_progress(window, &fingerprint, &cert, &point, &keeper, owner, knot, &mut session);
            let _ = sex_pdx::sys_wait_self_ticks(16);
            continue
        };
        if let Some(knot) = knot {
            if let Some(answer) = receive(&message, &cert, &point, &keeper, owner, knot, &mut session) {
                sex_pdx::pdx_reply(message.caller_pd, answer);
                if message.type_id == p::FINISH || message.type_id == p::END { show(window, &fingerprint, true, 0, &session); }
                drive_progress(window, &fingerprint, &cert, &point, &keeper, owner, Some(knot), &mut session);
                continue;
            }
        }
        match window.receive(&message) {
            Some(Event::Close) => { input.zeroize(); std::process::exit(0); }
            Some(Event::Key(key)) if knot.is_none() => {
                if key.code == 0x0E { input.pop(); }
                else if key.code == 0x1C {
                    if let Some(mut secret) = scalar(&input) {
                        if let Ok(candidate) = keeper.tie(owner, "Vellum PDF certificate", &secret[..]) {
                            if let Ok(public) = keeper.p256_public_key(owner, candidate) {
                                if public == cert_point {
                                    point = public;
                                    knot = Some(candidate);
                                    if sex_app::pdf_sign::register(cert.len()).is_err() { knot = None; let _ = keeper.untie(owner, candidate); point.clear(); }
                                } else { let _ = keeper.untie(owner, candidate); }
                            }
                        }
                        secret.zeroize();
                    }
                    input.zeroize(); input.clear();
                } else if !key.control {
                    if let Some(c) = sex_app::text(key).filter(|c| c.is_ascii_hexdigit()) {
                        if input.len() < 64 { input.push(c); }
                    }
                }
                show(window, &fingerprint, knot.is_some(), input.len(), &session);
                if knot.is_some() { sex_std::record(&format!("[shibari.pdf.ready] row={} certificate_bytes={}", target, cert.len())); }
            }
            Some(Event::Key(key)) if matches!(session, Session::Approval { .. }) => {
                if let Some(c) = sex_app::text(key).map(|c| c.to_ascii_lowercase()) {
                    if c == 'y' {
                        if let Session::Approval { data, source_path, field, source_hash, .. } = core::mem::replace(&mut session, Session::Idle) {
                            // A second independent read closes the approval interval.
                            // Vellum's conditional promotion rejects any later mutation.
                            session = check_in_worker(&source_path, &field, &data, &cert, &point)
                                .map_or(Session::Refused, |result| Session::Recheck { data, source_hash, result });
                        }
                    } else if c == 'n' { session = Session::Refused; sex_std::record("[shibari.pdf.refused]"); }
                    show(window, &fingerprint, true, 0, &session);
                }
            }
            _ => {}
        }
        drive_progress(window, &fingerprint, &cert, &point, &keeper, owner, knot, &mut session);
    }
}
