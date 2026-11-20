// SPDX-License-Identifier: GPL-3.0-only
//! Voice: bounded native SexOS client for a host voice worker (sex-voice frames).
//! `run +net voice [http://HOST:PORT [VOICE_ID]]`; files stay in this run's folder.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::{files, net, Event, Window};
use sex_voice::{encode, decode, JobState, Message, Operation, Submit, MAX_FRAME};
use sha2::{Digest, Sha256};

const DEFAULT_ENDPOINT: &[u8] = b"http://10.0.2.2:8875";
const MAX_PROMPT: usize = 1024;
const AUDIO_CHUNK: u64 = 32 * 1024;
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_FRAMES: u32 = 4_000_000;
static mut PROMPT: [u8; MAX_PROMPT] = [0; MAX_PROMPT];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[voice.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn split_args(args: &[u8]) -> (&[u8], u64) {
    let mut words = args.split(|b| *b == b' ').filter(|w| !w.is_empty());
    let endpoint = words.next().unwrap_or(DEFAULT_ENDPOINT);
    let voice_id = words.next().and_then(|v| core::str::from_utf8(v).ok()?.parse::<u64>().ok()).unwrap_or(0);
    (endpoint, voice_id)
}

fn endpoint_url(endpoint: &[u8], suffix: &[u8], out: &mut [u8; 320]) -> Option<usize> {
    let base = endpoint.strip_suffix(b"/").unwrap_or(endpoint);
    if !base.starts_with(b"http://") && !base.starts_with(b"https://") { return None; }
    let len = base.len().checked_add(suffix.len())?;
    if len > out.len() { return None; }
    out[..base.len()].copy_from_slice(base);
    out[base.len()..len].copy_from_slice(suffix);
    Some(len)
}

fn frame_request(window: &mut Window, endpoint: &[u8], message: Message<'_>) -> Result<Message<'static>, &'static str> {
    // Decode borrows its frame; own a fixed response buffer for the caller.
    // This function returns the small response by copying its parsed fields below.
    let mut url = [0u8; 320];
    let n = endpoint_url(endpoint, b"/v1/frame", &mut url).ok_or("Invalid worker URL")?;
    let mut frame = [0u8; MAX_FRAME];
    let len = encode(message, &mut frame).map_err(|_| "Invalid voice request")?;
    let mut reply = [0u8; MAX_FRAME + 1024];
    let result = net::post(window, &url[..n], &frame[..len], &mut reply).map_err(|e| match e {
        net::Error::NotAllowed => "Start Voice with run +net",
        net::Error::TooSlow => "Voice worker timed out",
        _ => "Voice worker unavailable",
    })?;
    if result.status != 200 { return Err("Voice worker rejected request"); }
    // A borrowed Message cannot outlive reply. Materialize only Status or Result
    // through the global response slot below, which is overwritten per request.
    let body = &reply[result.body..result.end];
    if body.len() > MAX_FRAME { return Err("Voice response too large"); }
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(RESPONSE) };
    slot[..body.len()].copy_from_slice(body);
    let response = decode(&slot[..body.len()]).map_err(|_| "Voice response invalid")?;
    if response.job_id() != message.job_id() { return Err("Voice job mismatch"); }
    Ok(response)
}

static mut RESPONSE: [u8; MAX_FRAME] = [0; MAX_FRAME];

fn range_header(value: &[u8]) -> Option<(u64, u64, u64)> {
    let rest = value.strip_prefix(b"bytes ")?;
    let slash = rest.iter().position(|b| *b == b'/')?;
    let dash = rest[..slash].iter().position(|b| *b == b'-')?;
    let number = |bytes: &[u8]| core::str::from_utf8(bytes).ok()?.parse::<u64>().ok();
    Some((number(&rest[..dash])?, number(&rest[dash + 1..slash])?, number(&rest[slash + 1..])?))
}

fn save_result(window: &mut Window, endpoint: &[u8], job_id: u64, result: sex_voice::ResultAsset<'_>, prompt: &[u8]) -> Result<String, &'static str> {
    let suffix = format!("/v1/assets/{}", result.asset_id);
    let mut url = [0u8; 320];
    let n = endpoint_url(endpoint, suffix.as_bytes(), &mut url).ok_or("Invalid asset URL")?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(AUDIO_CHUNK as usize + 2048).map_err(|_| "Not enough memory for voice transfer")?;
    bytes.resize(AUDIO_CHUNK as usize + 2048, 0);
    let path = format!("take-{}.wav", job_id);
    let partial = format!("take-{}.partial", job_id);
    let mut digest = Sha256::new();
    let (mut at, mut total, mut created) = (0u64, None, false);
    loop {
        let end = at.saturating_add(AUDIO_CHUNK - 1);
        let response = net::fetch_range(window, &url[..n], at, end, &mut bytes).map_err(|e| match e {
            net::Error::NotAllowed => "Start Voice with run +net",
            net::Error::TooSlow => "Voice take transfer timed out; partial file retained",
            _ => "Could not fetch voice take; partial file retained",
        })?;
        if response.status != 206 { return Err("Voice worker did not honor byte ranges"); }
        let (start, last, length) = response.header(&bytes, b"Content-Range")
            .and_then(range_header).ok_or("Invalid voice range")?;
        let chunk = &bytes[response.body..response.end];
        if start != at || last < start || last - start + 1 != chunk.len() as u64
            || length == 0 || length > MAX_ASSET_BYTES || last >= length
            || total.is_some_and(|known| known != length) { return Err("Voice range mismatch"); }
        if response.header(&bytes, b"Content-Length")
            .and_then(|v| core::str::from_utf8(v).ok()?.parse::<usize>().ok()) != Some(chunk.len()) {
            return Err("Voice range truncated");
        }
        if !created {
            if chunk.len() < 44 || !chunk.starts_with(b"RIFF") || &chunk[8..12] != b"WAVE"
                || u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as u64 + 8 != length {
                return Err("Invalid WAV asset");
            }
            files::create(window, partial.as_bytes()).map_err(|_| "Voice take name already exists")?;
            created = true;
        }
        if files::write_at(window, partial.as_bytes(), at, chunk).map_err(|_| "Could not save voice chunk")? != chunk.len() {
            return Err("Disk full while saving voice take; partial file retained");
        }
        digest.update(chunk);
        at = last + 1;
        total = Some(length);
        if at == length { break; }
    }
    files::sync(window).map_err(|_| "Could not sync voice take")?;
    let actual_hash: [u8; 32] = digest.finalize().into();
    if actual_hash != result.content_hash { return Err("Voice asset hash mismatch; take retained for recovery"); }
    let partial_id = files::file_id(window, partial.as_bytes()).map_err(|_| "Could not inspect voice take")?;
    files::move_if_id(window, partial.as_bytes(), path.as_bytes(), partial_id).map_err(|_| "Could not publish voice take")?;
    // Save an immutable result frame and the exact prompt beside the WAV. The
    // source path is private to this program's BOOT-bound Files folder.
    let mut metadata = [0u8; MAX_FRAME];
    let meta_len = encode(Message::Result { job_id, data: result }, &mut metadata).map_err(|_| "Invalid result metadata")?;
    let meta_path = format!("take-{}.sxvc", job_id);
    files::write_new(window, meta_path.as_bytes(), &metadata[..meta_len]).map_err(|_| "Could not save result metadata")?;
    let prompt_path = format!("take-{}.txt", job_id);
    files::write_new(window, prompt_path.as_bytes(), prompt).map_err(|_| "Could not save prompt")?;
    sex_pdx::serial_record!("[voice.saved] job={} bytes={} rate={} channels={} frames={}",
        job_id, at, result.sample_rate, result.channels, result.frames);
    Ok(path)
}

fn display(window: &mut Window, endpoint: &[u8], voice_id: u64, prompt: &[u8], state: &str) {
    let endpoint = core::str::from_utf8(endpoint).unwrap_or("invalid");
    let prompt = core::str::from_utf8(prompt).unwrap_or("invalid");
    let text = format!("Voice\n\nWorker: {}\nVoice ID: {} (0 = direct)\n\n{}|\n\nCtrl+G generate · Ctrl+C cancel · F11 close\n{}",
        endpoint, voice_id, prompt, state);
    window.show(text.as_bytes());
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Voice");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let (endpoint, voice_id) = split_args(files::args(&mut window, &mut args));
    let endpoint = endpoint.to_vec();
    let prompt = unsafe { &mut *core::ptr::addr_of_mut!(PROMPT) };
    let (mut len, mut job_id, mut next_job) = (0usize, None, sex_pdx::get_ticks().max(1));
    let mut state = String::from("Type a prompt, then generate.");
    display(&mut window, &endpoint, voice_id, &prompt[..len], &state);
    sex_pdx::serial_record!("[voice.ready] target={} task={} voice={}", window.target, window.task, voice_id);
    loop {
        if let Some(event) = window.next_within(500) {
            match event {
                Event::Close => { sex_pdx::serial_record!("[voice.closed]"); window.close() }
                Event::Key(key) if key.control && key.code == 0x22 && job_id.is_none() && len > 0 => {
                    let text = core::str::from_utf8(&prompt[..len]).unwrap_or("");
                    let id = next_job; next_job = next_job.wrapping_add(1).max(1);
                    let request = Message::Submit { job_id: id, data: Submit {
                        operation: Operation::Synthesize, seed: 0, voice_id, reference_asset_id: 0,
                        max_frames: MAX_AUDIO_FRAMES, language: "en", model: "active", text,
                    }};
                    match frame_request(&mut window, &endpoint, request) {
                        Ok(Message::Status { data, .. }) if data.state == JobState::Queued => { job_id = Some(id); state = String::from("Queued."); },
                        Ok(_) => state = String::from("Unexpected worker response."),
                        Err(e) => state = String::from(e),
                    }
                }
                Event::Key(key) if key.control && key.code == 0x2e => {
                    if let Some(id) = job_id.take() {
                        let _ = frame_request(&mut window, &endpoint, Message::Cancel { job_id: id });
                        state = String::from("Cancelled (worker may finish current inference).");
                    }
                }
                Event::Key(key) if job_id.is_none() && key.code == 0x0E => { len = len.saturating_sub(1); state.clear(); }
                Event::Key(key) if job_id.is_none() => {
                    if let Some(c) = sex_app::typed(key) { if len < prompt.len() { prompt[len] = c; len += 1; state.clear(); } }
                }
                _ => {}
            }
        } else if let Some(id) = job_id {
            match frame_request(&mut window, &endpoint, Message::GetStatus { job_id: id }) {
                Ok(Message::Status { data, .. }) => match data.state {
                    JobState::Queued => state = String::from("Queued."),
                    JobState::Running => state = format!("Generating: {}%", data.progress_bps / 100),
                    JobState::Failed => { state = format!("Worker failed: {}", data.error_code); job_id = None; },
                    JobState::Cancelled => { state = String::from("Cancelled."); job_id = None; },
                    JobState::Complete => state = String::from("Finishing take."),
                },
                Ok(Message::Result { data, .. }) => {
                    state = match save_result(&mut window, &endpoint, id, data, &prompt[..len]) {
                        Ok(path) => format!("Saved {} with prompt and metadata.", path),
                        Err(e) => String::from(e),
                    };
                    job_id = None;
                }
                Err(e) => { state = String::from(e); job_id = None; }
                _ => { state = String::from("Unexpected worker response."); job_id = None; }
            }
        }
        display(&mut window, &endpoint, voice_id, &prompt[..len], &state);
    }
}
