// SPDX-License-Identifier: GPL-3.0-only
//! Transport layer for [`crate::Client`]: in-memory buffers for batch/test
//! runs and the `sex` host-import channel for `#![no_std]` WASM guests running
//! inside `crates/polyamour-wasm` (`sex-wasm` / `wasmi`).

use alloc::{collections::VecDeque, string::String, vec::Vec};
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdkError {
    Wire(String),
    Transport(String),
    Protocol(String),
    Closed,
}

impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(msg) => write!(f, "wire error: {msg}"),
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Self::Closed => f.write_str("polyops session is already closed"),
        }
    }
}

/// Bi-directional line transport between a Rust guest/script and Polyamour's
/// `sex_3d::script::Session`.
pub trait Transport {
    /// Emit a single `polyops 1` command line (without trailing `\n`).
    fn write_line(&mut self, line: &str) -> Result<(), SdkError>;

    /// Read the next reply line (`@ID ok ...`) produced by Polyamour.
    fn read_reply(&mut self) -> Result<String, SdkError>;

    /// Optionally read the initial `POLYAMOUR_SCENE` snapshot text.
    fn read_scene_snapshot(&mut self) -> Result<Option<String>, SdkError> {
        Ok(None)
    }

    /// Emit a diagnostic log line outside the `polyops` command stream.
    fn log(&mut self, _message: &str) {}
}

/// In-memory transport for standalone `polyops` program generation and unit tests.
#[derive(Clone, Debug, Default)]
pub struct BufferTransport {
    lines: Vec<String>,
    canned_replies: VecDeque<String>,
    snapshot: Option<String>,
    logs: Vec<String>,
}

impl BufferTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_snapshot(snapshot: impl Into<String>) -> Self {
        Self {
            snapshot: Some(snapshot.into()),
            ..Self::default()
        }
    }

    pub fn push_canned_reply(&mut self, reply: impl Into<String>) {
        self.canned_replies.push_back(reply.into());
    }

    pub fn program(&self) -> String {
        let mut out = self.lines.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn logs(&self) -> &[String] {
        &self.logs
    }
}

impl Transport for BufferTransport {
    fn write_line(&mut self, line: &str) -> Result<(), SdkError> {
        if line.contains('\n') || line.contains('\r') {
            return Err(SdkError::Wire("a polyops command line cannot contain raw newlines".into()));
        }
        self.lines.push(String::from(line));
        Ok(())
    }

    fn read_reply(&mut self) -> Result<String, SdkError> {
        self.canned_replies
            .pop_front()
            .ok_or_else(|| SdkError::Transport("no reply queued in BufferTransport".into()))
    }

    fn read_scene_snapshot(&mut self) -> Result<Option<String>, SdkError> {
        Ok(self.snapshot.clone())
    }

    fn log(&mut self, message: &str) {
        self.logs.push(String::from(message));
    }
}

/// Native `#![no_std]` WebAssembly guest transport speaking over `sex-wasm`'s
/// `"sex"` host import module (`ops`, `replies`, `scene`, and `log`).
#[cfg(target_arch = "wasm32")]
pub mod wasm_guest {
    use super::{SdkError, Transport};
    use alloc::{format, string::String, vec, vec::Vec};

    #[link(wasm_import_module = "sex")]
    extern "C" {
        fn log(ptr: i32, len: i32) -> i32;
        fn read(path_ptr: i32, path_len: i32, out_ptr: i32, out_cap: i32) -> i32;
        fn write(path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32) -> i32;
        pub fn exit(code: i32) -> !;
    }

    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    extern "C" {
        fn args_sizes_get(argc_ptr: i32, argv_buf_size_ptr: i32) -> i32;
        fn args_get(argv_ptr: i32, argv_buf_ptr: i32) -> i32;
    }

    /// Read command-line arguments (`argv[1..]`, such as `steps=16` passed by
    /// Polyamour's sidebar UI when clicking an add-on button) inside a WASM guest.
    pub fn guest_args() -> Vec<String> {
        let mut argc: u32 = 0;
        let mut buf_size: u32 = 0;
        let rc = unsafe {
            args_sizes_get(
                (&mut argc as *mut u32) as i32,
                (&mut buf_size as *mut u32) as i32,
            )
        };
        if rc != 0 || argc <= 1 || buf_size > (1 << 20) {
            return Vec::new();
        }
        let mut ptrs = vec![0u32; argc as usize];
        let mut buf = vec![0u8; buf_size as usize];
        let rc2 = unsafe { args_get(ptrs.as_mut_ptr() as i32, buf.as_mut_ptr() as i32) };
        if rc2 != 0 {
            return Vec::new();
        }
        let base = buf.as_ptr() as usize;
        let mut out = Vec::with_capacity((argc - 1) as usize);
        for &p in &ptrs[1..] {
            let start = (p as usize).saturating_sub(base);
            if start >= buf.len() {
                continue;
            }
            let tail = &buf[start..];
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            if let Ok(s) = core::str::from_utf8(&tail[..end]) {
                out.push(String::from(s));
            }
        }
        out
    }

    /// Minimal single-threaded WASM page-growing bump allocator for `#![no_std]`
    /// short-lived Polyamour `.wasm` add-on runs.
    pub struct WasmBumpAlloc {
        state: core::cell::UnsafeCell<(usize, usize)>,
    }

    unsafe impl Sync for WasmBumpAlloc {}

    impl WasmBumpAlloc {
        pub const fn new() -> Self {
            Self {
                state: core::cell::UnsafeCell::new((0, 0)),
            }
        }
    }

    unsafe impl core::alloc::GlobalAlloc for WasmBumpAlloc {
        unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
            const PAGE: usize = 65_536;
            let align = layout.align().max(8);
            let size = layout.size();
            let (cur, end) = unsafe { &mut *self.state.get() };
            let aligned = (*cur + align - 1) & !(align - 1);
            let next = match aligned.checked_add(size) {
                Some(n) => n,
                None => return core::ptr::null_mut(),
            };
            if next > *end {
                let needed = next - *end;
                let pages = (needed + PAGE - 1) / PAGE;
                let prev_pages = core::arch::wasm32::memory_grow(0, pages);
                if prev_pages == usize::MAX {
                    return core::ptr::null_mut();
                }
                let grown_start = prev_pages * PAGE;
                if *end == 0 {
                    *cur = grown_start;
                }
                *end = (prev_pages + pages) * PAGE;
                let aligned_grown = (*cur + align - 1) & !(align - 1);
                let next_grown = match aligned_grown.checked_add(size) {
                    Some(n) if n <= *end => n,
                    _ => return core::ptr::null_mut(),
                };
                *cur = next_grown;
                return aligned_grown as *mut u8;
            }
            *cur = next;
            aligned as *mut u8
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
    }

    const OPS_PATH: &[u8] = b"ops";
    const REPLIES_PATH: &[u8] = b"replies";
    const SCENE_PATH: &[u8] = b"scene";

    #[derive(Default)]
    pub struct SexWasmTransport {
        reply_buf: Vec<u8>,
    }

    impl SexWasmTransport {
        pub const fn new() -> Self {
            Self { reply_buf: Vec::new() }
        }
    }

    impl Transport for SexWasmTransport {
        fn write_line(&mut self, line: &str) -> Result<(), SdkError> {
            if line.contains('\n') || line.contains('\r') {
                return Err(SdkError::Wire("a polyops line cannot contain raw newlines".into()));
            }
            let mut framed = Vec::with_capacity(line.len() + 1);
            framed.extend_from_slice(line.as_bytes());
            framed.push(b'\n');
            let rc = unsafe {
                write(
                    OPS_PATH.as_ptr() as i32,
                    OPS_PATH.len() as i32,
                    framed.as_ptr() as i32,
                    framed.len() as i32,
                )
            };
            if rc < 0 {
                return Err(SdkError::Transport(format!("sex.write(ops) refused with {rc}")));
            }
            Ok(())
        }

        fn read_reply(&mut self) -> Result<String, SdkError> {
            let mut scratch = [0u8; 4096];
            loop {
                if let Some(nl) = self.reply_buf.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = self.reply_buf.drain(..=nl).collect();
                    let s = core::str::from_utf8(&line_bytes[..line_bytes.len() - 1])
                        .map_err(|_| SdkError::Protocol("non-UTF8 reply from host".into()))?;
                    return Ok(String::from(s.trim_end_matches('\r')));
                }
                let n = unsafe {
                    read(
                        REPLIES_PATH.as_ptr() as i32,
                        REPLIES_PATH.len() as i32,
                        scratch.as_mut_ptr() as i32,
                        scratch.len() as i32,
                    )
                };
                if n < 0 {
                    return Err(SdkError::Transport(format!("sex.read(replies) refused with {n}")));
                }
                if n == 0 {
                    return Err(SdkError::Transport("host returned empty reply stream".into()));
                }
                self.reply_buf.extend_from_slice(&scratch[..n as usize]);
            }
        }

        fn read_scene_snapshot(&mut self) -> Result<Option<String>, SdkError> {
            let mut all = Vec::new();
            let mut chunk = vec![0u8; 16 * 1024];
            loop {
                let n = unsafe {
                    read(
                        SCENE_PATH.as_ptr() as i32,
                        SCENE_PATH.len() as i32,
                        chunk.as_mut_ptr() as i32,
                        chunk.len() as i32,
                    )
                };
                if n < 0 {
                    return Ok(None);
                }
                if n == 0 {
                    break;
                }
                all.extend_from_slice(&chunk[..n as usize]);
            }
            if all.is_empty() {
                return Ok(None);
            }
            let text = String::from_utf8(all)
                .map_err(|_| SdkError::Protocol("non-UTF8 scene snapshot".into()))?;
            Ok(Some(text))
        }

        fn log(&mut self, message: &str) {
            let bytes = message.as_bytes();
            unsafe {
                let _ = log(bytes.as_ptr() as i32, bytes.len() as i32);
            }
        }
    }
}
