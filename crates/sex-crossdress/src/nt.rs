// SPDX-License-Identifier: GPL-3.0-only
//! CW3: Wine's own PE DLLs in a cell, starting with ntdll.dll.
//!
//! Wine splits each DLL into a PE half (Windows code, run here unmodified in the cell)
//! and a Unix half (ntdll.so and friends). On SexOS the Unix half is this host side: the
//! PE half reaches it through two seams Wine already has.
//!
//! - **NT system calls.** Every `Nt*` stub in Wine's ntdll.dll is
//!   `mov r10, rcx; mov eax, N; test byte [0x7FFE0308], 1; jne +3; syscall; ret; ...;
//!   call [0x7FFE1000]`. The byte and the pointer live in KUSER_SHARED_DATA, which this
//!   module maps: the byte is set, so the stub calls the pointer, which is the cell's
//!   syscall dispatcher thunk (`process::SYSCALL_DISPATCHER_VA`). The call numbers are
//!   read from the stubs themselves (`WinProcess::syscall_names`), so the host matches
//!   whatever Wine build is loaded by name, not by number.
//! - **Unix calls.** `__wine_unix_call(handle, code, args)` jumps through ntdll's
//!   `__wine_unix_call_dispatcher` pointer, which this module points at the cell's Unix
//!   call thunk.
//!
//! The system calls implemented so far are the ones Wine's own heap and time functions
//! need. Any other stops the process cleanly as `ntdll.syscall!NtName`, so the next one
//! to write is always named. Threads, files and sections (CW4) come next.
use crate::cell::{Access, PAGE};
use crate::process::{self, Host, Stop, WinProcess};
use crate::win32::{self, Outcome};
use alloc::string::String;
use alloc::vec::Vec;
use sex_pdx::vm::Regs;

/// KUSER_SHARED_DATA, read-only for user code, at its fixed Windows address.
pub const KUSER_VA: u64 = 0x7FFE_0000;
const KUSER_PAGES: u64 = 2;
const KUSER_SYSTEM_CALL: u64 = 0x308;
const KUSER_DISPATCHER: u64 = 0x1000;
const DEBUG_OPTIONS_PAGES: u64 = 2;
/// The FILETIME (100 ns since 1601) the cell's clock starts at: 2026-01-01.
pub const EPOCH_FILETIME: u64 = 134_116_992_000_000_000;

pub const STATUS_SUCCESS: u64 = 0;
pub const STATUS_NOT_IMPLEMENTED: u64 = 0xC000_0002;
pub const STATUS_INVALID_PARAMETER: u64 = 0xC000_000D;
pub const STATUS_NO_MEMORY: u64 = 0xC000_0017;
pub const STATUS_CONFLICTING_ADDRESSES: u64 = 0xC000_0018;
pub const STATUS_ACCESS_VIOLATION: u64 = 0xC000_0005;

/// The handle ntdll's own Unix library gets (`__wine_unixlib_handle`).
pub const NTDLL_UNIXLIB: u64 = 0x5E05_0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WineError { NotNtdll, Cell }

/// Makes a process whose module is Wine's ntdll.dll ready to be called: maps
/// KUSER_SHARED_DATA with the syscall flag and dispatcher, and fills the data exports
/// the Unix half would (`__wine_unix_call_dispatcher`, `__wine_syscall_dispatcher`,
/// `__wine_unixlib_handle`).
pub fn prepare_ntdll(p: &mut WinProcess) -> Result<(), WineError> {
    let unix = p.export("__wine_unix_call_dispatcher").ok_or(WineError::NotNtdll)?;
    if p.syscall_names.is_empty() { return Err(WineError::NotNtdll); }
    p.cell.map(KUSER_VA, KUSER_PAGES, Access::USER_RO).map_err(|_| WineError::Cell)?;
    // Wine's Unix half keeps the debug channel options right after the PEB, at PEB+0x2000:
    // 16-byte entries {flags, name[15]} ending with an empty name. An empty list leaves
    // every channel off (no output through the Unix call that writes it).
    p.cell.map(process::PEB_VA + PAGE, DEBUG_OPTIONS_PAGES, Access::USER_RW).map_err(|_| WineError::Cell)?;
    let mut k = [0u8; 0x400];
    k[0x26C..0x270].copy_from_slice(&10u32.to_le_bytes()); // NtMajorVersion
    k[0x270..0x274].copy_from_slice(&0u32.to_le_bytes()); // NtMinorVersion
    k[0x260..0x264].copy_from_slice(&19045u32.to_le_bytes()); // NtBuildNumber
    k[0x26A..0x26C].copy_from_slice(&0x8664u16.to_le_bytes()); // NativeProcessorArchitecture... (ImageNumberHigh)
    k[0x268..0x26A].copy_from_slice(&0x8664u16.to_le_bytes()); // ImageNumberLow
    k[KUSER_SYSTEM_CALL as usize] = 1;
    p.cell.write(KUSER_VA, &k).ok_or(WineError::Cell)?;
    p.cell.write_u64(KUSER_VA + KUSER_DISPATCHER, process::SYSCALL_DISPATCHER_VA).ok_or(WineError::Cell)?;
    p.cell.write_u64(unix, process::UNIX_CALL_DISPATCHER_VA).ok_or(WineError::Cell)?;
    if let Some(sys) = p.export("__wine_syscall_dispatcher") { p.cell.write_u64(sys, process::SYSCALL_DISPATCHER_VA).ok_or(WineError::Cell)?; }
    if let Some(h) = p.export("__wine_unixlib_handle") { p.cell.write_u64(h, NTDLL_UNIXLIB).ok_or(WineError::Cell)?; }
    Ok(())
}

/// Reads the system call numbers out of a DLL's `Nt*` stubs (Wine's and Windows's
/// ntdll both start each with `mov r10, rcx; mov eax, N`).
pub fn syscall_numbers(exports: &[(String, u32)], mem: &[u8]) -> alloc::collections::BTreeMap<u32, String> {
    let mut out = alloc::collections::BTreeMap::new();
    for (name, rva) in exports {
        if !name.starts_with("Nt") { continue; }
        let at = *rva as usize;
        let Some(code) = mem.get(at..at + 8) else { continue };
        if code[..4] == [0x4C, 0x8B, 0xD1, 0xB8] {
            out.insert(u32::from_le_bytes([code[4], code[5], code[6], code[7]]), name.clone());
        }
    }
    out
}

/// The `n`th system call argument (0-based) at a syscall exit: R10, RDX, R8, R9, then the
/// caller's stack above the stub's return address, its caller's and the home space.
fn arg(p: &WinProcess, regs: &Regs, n: usize) -> Option<u64> {
    // The guest's Windows x64 argument registers (a guest register file, not SexOS IPC).
    let g = regs;
    match n {
        0 => Some(g.r10),
        1 => Some(g.rdx),
        2 => Some(g.r8),
        3 => Some(g.r9),
        _ => { let mut b = [0u8; 8]; p.cell.read_user(g.rsp + 0x30 + 8 * (n as u64 - 4), &mut b)?; Some(u64::from_le_bytes(b)) }
    }
}

fn get(p: &WinProcess, va: u64) -> Option<u64> { let mut b = [0u8; 8]; p.cell.read_user(va, &mut b)?; Some(u64::from_le_bytes(b)) }
fn put(p: &mut WinProcess, va: u64, v: u64) -> Option<()> { p.cell.write_user(va, &v.to_le_bytes()) }

/// Runs system call `number`.
pub fn syscall(p: &mut WinProcess, number: u32, regs: &Regs, host: &mut impl Host) -> Outcome {
    let name = match p.syscall_names.get(&number) { Some(n) => n.clone(), None => alloc::format!("#{number}") };
    let mut a = [0u64; 12];
    for (i, v) in a.iter_mut().enumerate() { *v = arg(p, regs, i).unwrap_or(0); }
    if p.nt.trace { host.output(2, alloc::format!("[nt] {name}({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})\n", a[0], a[1], a[2], a[3], a[4], a[5]).as_bytes()); }
    let status = match name.as_str() {
        "NtAllocateVirtualMemory" => allocate(p, a[0], a[1], a[3], a[4] as u32, a[5] as u32),
        // Pages are not given back to the cell yet (no unmap): freeing succeeds and the
        // range stays mapped. Wine's heap only frees what it allocated.
        "NtFreeVirtualMemory" => STATUS_SUCCESS,
        "NtProtectVirtualMemory" => protect(p, a[0], a[1], a[2], a[3] as u32, a[4]),
        "NtQueryVirtualMemory" => query_memory(p, a[0], a[1], a[2] as u32, a[3], a[4], a[5]),
        "NtQuerySystemTime" => match put(p, a[0], EPOCH_FILETIME + host.now_ms() * 10_000) { Some(()) => STATUS_SUCCESS, None => STATUS_ACCESS_VIOLATION },
        "NtQueryPerformanceCounter" => {
            let ok = put(p, a[0], host.now_ms() * 1000).is_some() && (a[1] == 0 || put(p, a[1], 1_000_000).is_some());
            if ok { STATUS_SUCCESS } else { STATUS_ACCESS_VIOLATION }
        }
        "NtClose" => { p.nt.handles.remove(&a[0]); STATUS_SUCCESS }
        "NtYieldExecution" | "NtFlushInstructionCache" | "NtSetInformationThread" | "NtSetInformationProcess" | "NtDelayExecution" => STATUS_SUCCESS,
        // Handle 0 ends every other thread of the process (RtlExitUserProcess does that before
        // DllMain(DETACH) flushes the C runtime): with one thread there is nothing to end.
        "NtTerminateProcess" if a[0] == 0 => STATUS_SUCCESS,
        "NtTerminateProcess" if a[0] == u64::MAX => return Outcome::Stop(Stop::Exited(a[1] as u32)),
        "NtTerminateThread" if a[0] == 0 || a[0] == u64::MAX - 1 => return Outcome::Stop(Stop::Exited(a[1] as u32)),
        "NtContinue" => {
            let mut c = [0u8; crate::win32::CONTEXT_SIZE as usize];
            if p.cell.read_user(a[0], &mut c).is_none() { return Outcome::Return(STATUS_ACCESS_VIOLATION); }
            let mut r = *regs;
            crate::win32::read_context(&c, &mut r);
            return Outcome::Jump(r);
        }
        "NtOpenFile" => open_file(p, a[0], a[2], a[3], host),
        "NtCreateFile" => if a[7] == 1 /* FILE_OPEN */ || a[7] == 3 /* FILE_OPEN_IF */ { open_file(p, a[0], a[2], a[3], host) } else { STATUS_ACCESS_DENIED },
        "NtQueryAttributesFile" => query_attributes(p, a[0], a[1], false),
        "NtQueryFullAttributesFile" => query_attributes(p, a[0], a[1], true),
        "NtCreateSection" => create_section(p, a[0], a[3], a[5] as u32, a[6]),
        "NtQuerySection" => query_section(p, a[0], a[1] as u32, a[2], a[3], a[4]),
        "NtMapViewOfSection" => map_view(p, a[0], a[1], a[2], a[6]),
        "NtUnmapViewOfSection" => STATUS_SUCCESS,
        "NtQueryInformationProcess" => query_process(p, a[0], a[1] as u32, a[2], a[3], a[4]),
        "NtQuerySystemInformation" => query_system(p, a[0] as u32, a[1], a[2], a[3]),
        "NtOpenKeyEx" | "NtOpenKey" | "NtCreateKey" | "NtOpenSymbolicLinkObject" | "NtOpenDirectoryObject" => STATUS_OBJECT_NAME_NOT_FOUND,
        "NtQueryDefaultLocale" => if put32(p, a[1], 0x409).is_some() { STATUS_SUCCESS } else { STATUS_ACCESS_VIOLATION },
        "NtQueryDefaultUILanguage" | "NtQueryInstallUILanguage" => if put16(p, a[0], 0x409).is_some() { STATUS_SUCCESS } else { STATUS_ACCESS_VIOLATION },
        "NtInitializeNlsFiles" => init_nls(p, a[0], a[1], a[2]),
        "NtGetNlsSectionPtr" => nls_section(p, a[0] as u32, a[1] as u32, a[3], a[4]),
        "NtWriteFile" => write_file(p, a[0], a[4], a[5], a[6] as u32, host),
        "NtQueryInformationThread" => STATUS_INVALID_PARAMETER,
        "NtQueryInformationToken" => query_token(p, a[1] as u32, a[2], a[3], a[4]),
        "NtQueryVolumeInformationFile" => volume_info(p, a[0], a[1], a[2], a[3], a[4] as u32),
        // Console driver requests (condrv): there is no console device yet, so programs see
        // a plain file and write through NtWriteFile (GetConsoleMode fails).
        "NtDeviceIoControlFile" => 0xC000_0010, // STATUS_INVALID_DEVICE_REQUEST
        // A second-chance (unhandled) exception ends the process; the stop names it.
        "NtRaiseException" if a[2] == 0 => {
            let (code, at) = (get(p, a[0]).unwrap_or(0) as u32, get(p, a[0] + 0x10).unwrap_or(0));
            // EXCEPTION_WINE_STUB: a function Wine does not implement; the record names it.
            if code == 0x8000_0100 {
                let text = |va: u64| -> String { let mut b = [0u8; 64]; let _ = p.cell.read_user(va, &mut b); String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(64)]).into_owned() };
                let (dll, name) = (get(p, a[0] + 0x20).unwrap_or(0), get(p, a[0] + 0x28).unwrap_or(0));
                let name = if name >> 16 == 0 { alloc::format!("#{name}") } else { text(name) };
                return Outcome::Stop(Stop::Unimplemented { dll: text(dll), name });
            }
            return Outcome::Stop(Stop::Unimplemented { dll: String::from("unhandled exception"), name: alloc::format!("{code:#x} at {at:#x}") });
        }
        "NtFsControlFile" => fs_control(p, a[0], a[4], a[5] as u32, a[8], a[9]),
        _ => return Outcome::Stop(Stop::Unimplemented { dll: String::from("ntdll.syscall"), name }),
    };
    if p.nt.trace { host.output(2, alloc::format!("[nt]   -> {status:#x}\n").as_bytes()); }
    Outcome::Return(status)
}

pub const STATUS_ACCESS_DENIED: u64 = 0xC000_0022;
pub const STATUS_OBJECT_NAME_NOT_FOUND: u64 = 0xC000_0034;
pub const STATUS_OBJECT_PATH_NOT_FOUND: u64 = 0xC000_003A;
pub const STATUS_INVALID_HANDLE: u64 = 0xC000_0008;
pub const STATUS_INFO_LENGTH_MISMATCH: u64 = 0xC000_0004;
pub const STATUS_INVALID_INFO_CLASS: u64 = 0xC000_0003;
pub const STATUS_IMAGE_NOT_AT_BASE: u64 = 0x4000_0003;
pub const STATUS_INVALID_IMAGE_FORMAT: u64 = 0xC000_007B;
pub const STATUS_NOT_MAPPED_VIEW: u64 = 0xC000_0019;
pub const STATUS_INVALID_ADDRESS: u64 = 0xC000_0141;

/// An NT object a handle names.
#[derive(Clone, Debug)]
pub enum Object { File { file: usize, pos: u64 }, Directory(String), Section { file: usize, image: bool } }

/// The NT side of a Wine process: its virtual files, handles and loaded images.
#[derive(Default)]
pub struct NtState {
    /// Files by normalized path (`c:\windows\system32\kernel32.dll`).
    pub files: Vec<(String, Vec<u8>)>,
    pub handles: alloc::collections::BTreeMap<u64, Object>,
    next_handle: u64,
    /// Images mapped in the cell: (base, SizeOfImage).
    pub modules: Vec<(u64, u64)>,
    /// Which file each image view came from (base, index in `files`), so a second
    /// NtMapViewOfSection of the same file returns the same view.
    pub images: Vec<(u64, usize)>,
    pub main_image: Option<(u64, crate::pe::Image)>,
    pub main_entry: u64,
    /// NLS tables by file name, and where each is mapped once asked for.
    pub nls: Vec<(String, Vec<u8>)>,
    nls_views: Vec<(String, u64, u64)>,
    /// (stdout, stderr) console handles.
    pub stdout: Option<(u64, u64)>,
    /// Print every system call to the host's stderr stream.
    pub trace: bool,
}

/// `\??\C:\Foo` and `C:\foo` alike become `c:\foo`.
pub fn normalize(path: &str) -> String {
    let p = path.strip_prefix(r"\??\").or_else(|| path.strip_prefix(r"\DosDevices\")).unwrap_or(path);
    p.replace('/', "\\").to_ascii_lowercase()
}

impl NtState {
    pub fn add_file(&mut self, path: &str, data: Vec<u8>) {
        let key = normalize(path);
        match self.files.iter_mut().find(|f| f.0 == key) { Some(f) => f.1 = data, None => self.files.push((key, data)) }
    }
    pub fn file(&self, path: &str) -> Option<usize> { self.files.iter().position(|f| f.0 == path) }
    fn is_dir(&self, path: &str) -> bool {
        let p = path.trim_end_matches('\\');
        p.len() == 2 && p.ends_with(':') || self.files.iter().any(|f| f.0.len() > p.len() + 1 && f.0.starts_with(p) && f.0.as_bytes()[p.len()] == b'\\')
    }
    fn handle(&mut self, o: Object) -> u64 {
        if self.next_handle == 0 { self.next_handle = 0x100; }
        let h = self.next_handle;
        self.next_handle += 4;
        self.handles.insert(h, o);
        h
    }
}

fn put32(p: &mut WinProcess, va: u64, v: u32) -> Option<()> { p.cell.write_user(va, &v.to_le_bytes()) }
fn put16(p: &mut WinProcess, va: u64, v: u16) -> Option<()> { p.cell.write_user(va, &v.to_le_bytes()) }

/// A UNICODE_STRING at `va`, as Rust text.
fn unicode_string(p: &WinProcess, va: u64) -> Option<String> {
    let mut h = [0u8; 16];
    p.cell.read_user(va, &mut h)?;
    let len = u16::from_le_bytes([h[0], h[1]]) as usize;
    let buf = u64::from_le_bytes(h[8..16].try_into().ok()?);
    let mut b = alloc::vec![0u8; len];
    p.cell.read_user(buf, &mut b)?;
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

/// The normalized path an OBJECT_ATTRIBUTES names (RootDirectory joined).
fn object_path(p: &WinProcess, oa: u64) -> Option<String> {
    let root = get(p, oa + 8)?;
    let name = unicode_string(p, get(p, oa + 16)?)?;
    let base = match p.nt.handles.get(&root) { Some(Object::Directory(d)) => alloc::format!("{}\\", d.trim_end_matches('\\')), _ => String::new() };
    Some(normalize(&alloc::format!("{base}{name}")))
}

fn io_status(p: &mut WinProcess, iosb: u64, status: u64, info: u64) { if iosb != 0 { let _ = put(p, iosb, status); let _ = put(p, iosb + 8, info); } }

/// NtOpenFile/NtCreateFile (open existing, read).
fn open_file(p: &mut WinProcess, handle_ptr: u64, oa: u64, iosb: u64, host: &mut impl Host) -> u64 {
    let Some(path) = object_path(p, oa) else { return STATUS_ACCESS_VIOLATION };
    if p.nt.trace { host.output(2, alloc::format!("[nt]   open {path}\n").as_bytes()); }
    let obj = if let Some(f) = p.nt.file(&path) { Object::File { file: f, pos: 0 } } else if p.nt.is_dir(&path) { Object::Directory(path) } else {
        return if path.contains('\\') && !p.nt.is_dir(path.rsplit_once('\\').map(|x| x.0).unwrap_or("")) { STATUS_OBJECT_PATH_NOT_FOUND } else { STATUS_OBJECT_NAME_NOT_FOUND };
    };
    let h = p.nt.handle(obj);
    if put(p, handle_ptr, h).is_none() { return STATUS_ACCESS_VIOLATION; }
    io_status(p, iosb, 0, 1 /* FILE_OPENED */);
    STATUS_SUCCESS
}

/// NtQueryAttributesFile / NtQueryFullAttributesFile.
fn query_attributes(p: &mut WinProcess, oa: u64, out: u64, full: bool) -> u64 {
    let Some(path) = object_path(p, oa) else { return STATUS_ACCESS_VIOLATION };
    let (attr, size) = if let Some(f) = p.nt.file(&path) { (0x80u32, p.nt.files[f].1.len() as u64) } else if p.nt.is_dir(&path) { (0x10, 0) } else { return STATUS_OBJECT_NAME_NOT_FOUND };
    let t = EPOCH_FILETIME;
    let mut b = alloc::vec![0u8; if full { 56 } else { 40 }];
    for i in 0..4 { b[8 * i..8 * i + 8].copy_from_slice(&t.to_le_bytes()); }
    if full {
        b[32..40].copy_from_slice(&size.next_multiple_of(4096).to_le_bytes());
        b[40..48].copy_from_slice(&size.to_le_bytes());
        b[48..52].copy_from_slice(&attr.to_le_bytes());
    } else { b[32..36].copy_from_slice(&attr.to_le_bytes()); }
    if p.cell.write_user(out, &b).is_none() { return STATUS_ACCESS_VIOLATION; }
    STATUS_SUCCESS
}

const SEC_IMAGE: u32 = 0x0100_0000;

/// NtCreateSection(*handle, access, oa, *size, protect, attributes, file).
fn create_section(p: &mut WinProcess, handle_ptr: u64, _size: u64, attributes: u32, file: u64) -> u64 {
    let Some(Object::File { file, .. }) = p.nt.handles.get(&file).cloned() else { return STATUS_INVALID_HANDLE };
    let image = attributes & SEC_IMAGE != 0;
    if image && crate::pe::Image::parse(&p.nt.files[file].1).is_err() { return STATUS_INVALID_IMAGE_FORMAT; }
    let h = p.nt.handle(Object::Section { file, image });
    if put(p, handle_ptr, h).is_none() { return STATUS_ACCESS_VIOLATION; }
    STATUS_SUCCESS
}

/// SECTION_IMAGE_INFORMATION for an image file mapped (or to be mapped) at `base`.
fn image_information(img: &crate::pe::Image, file: &[u8], base: u64) -> [u8; 0x40] {
    let mut b = [0u8; 0x40];
    let lfanew = u32::from_le_bytes([file[0x3C], file[0x3D], file[0x3E], file[0x3F]]) as usize;
    let opt = lfanew + 24;
    let u16_at = |o: usize| file.get(o..o + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).unwrap_or(0);
    let u32_at = |o: usize| file.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).unwrap_or(0);
    b[0..8].copy_from_slice(&(if img.entry_rva != 0 { base + img.entry_rva as u64 } else { 0 }).to_le_bytes());
    b[0x10..0x18].copy_from_slice(&img.stack_reserve.to_le_bytes());
    b[0x18..0x20].copy_from_slice(&img.stack_commit.to_le_bytes());
    b[0x20..0x24].copy_from_slice(&(img.subsystem as u32).to_le_bytes());
    b[0x24..0x26].copy_from_slice(&u16_at(opt + 50).to_le_bytes()); // MinorSubsystemVersion
    b[0x26..0x28].copy_from_slice(&u16_at(opt + 48).to_le_bytes()); // MajorSubsystemVersion
    b[0x28..0x2A].copy_from_slice(&u16_at(opt + 40).to_le_bytes()); // MajorOperatingSystemVersion
    b[0x2A..0x2C].copy_from_slice(&u16_at(opt + 42).to_le_bytes());
    b[0x2C..0x2E].copy_from_slice(&img.characteristics.to_le_bytes());
    b[0x2E..0x30].copy_from_slice(&img.dll_characteristics.to_le_bytes());
    b[0x30..0x32].copy_from_slice(&img.machine.to_le_bytes());
    b[0x32] = 1; // ImageContainsCode
    b[0x34..0x38].copy_from_slice(&u32_at(opt + 112).to_le_bytes()); // LoaderFlags
    b[0x38..0x3C].copy_from_slice(&(file.len() as u32).to_le_bytes());
    b[0x3C..0x40].copy_from_slice(&u32_at(opt + 64).to_le_bytes()); // CheckSum
    b
}

/// NtQuerySection(section, class, buffer, length, *returned).
fn query_section(p: &mut WinProcess, section: u64, class: u32, buf: u64, len: u64, ret: u64) -> u64 {
    let Some(Object::Section { file, image: true }) = p.nt.handles.get(&section).cloned() else { return STATUS_INVALID_HANDLE };
    if class != 1 { return STATUS_INVALID_INFO_CLASS; }
    if len < 0x40 { return STATUS_INFO_LENGTH_MISMATCH; }
    let data = &p.nt.files[file].1;
    let Ok(img) = crate::pe::Image::parse(data) else { return STATUS_INVALID_IMAGE_FORMAT };
    let info = image_information(&img, data, img.preferred_base);
    if p.cell.write_user(buf, &info).is_none() { return STATUS_ACCESS_VIOLATION; }
    if ret != 0 { let _ = put(p, ret, 0x40); }
    STATUS_SUCCESS
}

/// NtMapViewOfSection(section, process, *base, zero_bits, commit, *offset, *view_size, ...)
/// for image sections: the image is laid out and relocated in the host, like Wine's
/// Unix half does, at its preferred base when free.
fn map_view(p: &mut WinProcess, section: u64, process: u64, base_ptr: u64, size_ptr: u64) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_HANDLE; }
    let Some(Object::Section { file, image }) = p.nt.handles.get(&section).cloned() else { return STATUS_INVALID_HANDLE };
    if !image { return STATUS_NOT_IMPLEMENTED; }
    let data = p.nt.files[file].1.clone();
    let Ok(img) = crate::pe::Image::parse(&data) else { return STATUS_INVALID_IMAGE_FORMAT };
    let size = (img.size_of_image as u64).next_multiple_of(PAGE);
    if let Some(&(b, _)) = p.nt.images.iter().find(|m| m.1 == file) {
        // This file is already mapped (ntdll and the program, which the host mapped): the
        // same view again. Another image at the same preferred base is relocated below.
        let _ = put(p, base_ptr, b);
        let _ = put(p, size_ptr, size);
        return STATUS_SUCCESS;
    }
    let mut base = img.preferred_base;
    if p.cell.is_taken(base, size) {
        base = 0x0000_0001_8000_0000;
        while p.cell.is_taken(base, size) { base += process::ALLOCATION_GRANULARITY; }
    }
    if p.map_image(&data, base).is_err() { return STATUS_NO_MEMORY; }
    p.nt.modules.push((base, size));
    p.nt.images.push((base, file));
    if put(p, base_ptr, base).is_none() || put(p, size_ptr, size).is_none() { return STATUS_ACCESS_VIOLATION; }
    if base != img.preferred_base { STATUS_IMAGE_NOT_AT_BASE } else { STATUS_SUCCESS }
}

/// Windows PAGE_* protection to cell rights (None = PAGE_NOACCESS).
fn rights(protect: u32) -> Option<Option<Access>> {
    Some(match protect & 0xFF {
        0x01 => None,
        0x02 => Some(Access::USER_RO),
        0x04 | 0x08 => Some(Access::USER_RW),
        0x10 | 0x20 => Some(Access::USER_RX),
        0x40 | 0x80 => Some(Access::USER_RWX),
        _ => return None,
    })
}
fn protection(a: Access) -> u32 { match (a.write, a.execute) { (false, false) => 0x02, (true, false) => 0x04, (false, true) => 0x20, (true, true) => 0x40 } }

/// NtProtectVirtualMemory(process, *base, *size, new, *old).
fn protect(p: &mut WinProcess, process: u64, base_ptr: u64, size_ptr: u64, new: u32, old_ptr: u64) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_HANDLE; }
    let (Some(base), Some(size)) = (get(p, base_ptr), get(p, size_ptr)) else { return STATUS_ACCESS_VIOLATION };
    let start = base & !(PAGE - 1);
    let end = (base + size.max(1)).next_multiple_of(PAGE);
    let Some(Some(access)) = rights(new) else { return STATUS_INVALID_PARAMETER };
    match p.cell.protect(start, (end - start) / PAGE, access) {
        Ok(old) => {
            let _ = put(p, base_ptr, start);
            let _ = put(p, size_ptr, end - start);
            if old_ptr != 0 { let _ = put32(p, old_ptr, protection(old)); }
            STATUS_SUCCESS
        }
        Err(_) => STATUS_NOT_MAPPED_VIEW,
    }
}

/// NtQueryVirtualMemory(process, address, class, buffer, length, *returned):
/// MemoryBasicInformation from the cell's maps and the images it holds.
fn query_memory(p: &mut WinProcess, process: u64, addr: u64, class: u32, buf: u64, len: u64, ret: u64) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_HANDLE; }
    if class != 0 { return STATUS_INVALID_INFO_CLASS; }
    if len < 48 { return STATUS_INFO_LENGTH_MISMATCH; }
    let page = addr & !(PAGE - 1);
    let module = p.nt.modules.iter().find(|m| addr >= m.0 && addr < m.0 + m.1).copied();
    let map = p.cell.maps().iter().find(|m| m.access.user && page >= m.va && page < m.va + m.pages * PAGE).copied();
    let mut b = [0u8; 48];
    b[0..8].copy_from_slice(&page.to_le_bytes());
    match map {
        Some(m) => {
            let alloc_base = module.map(|x| x.0).unwrap_or(m.va);
            b[8..16].copy_from_slice(&alloc_base.to_le_bytes());
            b[16..20].copy_from_slice(&protection(m.access).to_le_bytes());
            b[24..32].copy_from_slice(&(m.va + m.pages * PAGE - page).to_le_bytes());
            b[32..36].copy_from_slice(&0x1000u32.to_le_bytes()); // MEM_COMMIT
            b[36..40].copy_from_slice(&protection(m.access).to_le_bytes());
            b[40..44].copy_from_slice(&(if module.is_some() { 0x0100_0000u32 } else { 0x0002_0000 }).to_le_bytes()); // MEM_IMAGE / MEM_PRIVATE
        }
        None => {
            if addr > crate::cell::USER_TOP { return STATUS_INVALID_PARAMETER; }
            // Free (or reserved): up to the next mapping.
            let next = p.cell.maps().iter().filter(|m| m.access.user && m.va > page).map(|m| m.va).min().unwrap_or(crate::cell::USER_TOP + 1);
            b[24..32].copy_from_slice(&(next - page).to_le_bytes());
            b[32..36].copy_from_slice(&0x1_0000u32.to_le_bytes()); // MEM_FREE
            b[36..40].copy_from_slice(&1u32.to_le_bytes()); // PAGE_NOACCESS
        }
    }
    if p.cell.write_user(buf, &b).is_none() { return STATUS_ACCESS_VIOLATION; }
    if ret != 0 { let _ = put(p, ret, 48); }
    STATUS_SUCCESS
}

/// NtQueryInformationProcess(process, class, buffer, length, *returned).
fn query_process(p: &mut WinProcess, process: u64, class: u32, buf: u64, len: u64, ret: u64) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_HANDLE; }
    let out: Vec<u8> = match class {
        0 => { // ProcessBasicInformation
            let mut b = alloc::vec![0u8; 48];
            b[8..16].copy_from_slice(&process::PEB_VA.to_le_bytes());
            b[16..24].copy_from_slice(&1u64.to_le_bytes());
            b[32..40].copy_from_slice(&(process::PROCESS_ID as u64).to_le_bytes());
            b
        }
        7 => alloc::vec![0u8; 8],  // ProcessDebugPort: no debugger
        26 => alloc::vec![0u8; 8], // ProcessWow64Information: not WoW64
        36 => 0x5EC5_0C0Eu32.to_le_bytes().to_vec(), // ProcessCookie
        34 => 0u32.to_le_bytes().to_vec(), // ProcessExecuteFlags
        37 => { // ProcessImageInformation
            let Some((base, img)) = p.nt.main_image.clone() else { return STATUS_INVALID_INFO_CLASS };
            let file = p.nt.files.iter().find(|f| f.0.ends_with(".exe")).map(|f| f.1.clone()).unwrap_or_default();
            image_information(&img, &file, base).to_vec()
        }
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if len < out.len() as u64 { return STATUS_INFO_LENGTH_MISMATCH; }
    if p.cell.write_user(buf, &out).is_none() { return STATUS_ACCESS_VIOLATION; }
    if ret != 0 { let _ = put32(p, ret, out.len() as u32); }
    STATUS_SUCCESS
}

/// NtQuerySystemInformation(class, buffer, length, *returned).
fn query_system(p: &mut WinProcess, class: u32, buf: u64, len: u64, ret: u64) -> u64 {
    let out: Vec<u8> = match class {
        0 => { // SystemBasicInformation
            let mut b = alloc::vec![0u8; 64];
            b[8..12].copy_from_slice(&15_625u32.to_le_bytes()); // TimerResolution (100 ns units)
            b[12..16].copy_from_slice(&(PAGE as u32).to_le_bytes());
            b[16..20].copy_from_slice(&0x4_0000u32.to_le_bytes()); // physical pages
            b[24..28].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
            b[28..32].copy_from_slice(&(process::ALLOCATION_GRANULARITY as u32).to_le_bytes());
            b[32..40].copy_from_slice(&0x1_0000u64.to_le_bytes());
            b[40..48].copy_from_slice(&crate::cell::USER_TOP.to_le_bytes());
            b[48..56].copy_from_slice(&1u64.to_le_bytes()); // ActiveProcessorsAffinityMask
            b[56] = 1; // NumberOfProcessors
            b
        }
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if len < out.len() as u64 { return STATUS_INFO_LENGTH_MISMATCH; }
    if p.cell.write_user(buf, &out).is_none() { return STATUS_ACCESS_VIOLATION; }
    if ret != 0 { let _ = put32(p, ret, out.len() as u32); }
    STATUS_SUCCESS
}

/// Maps an NLS table read-only in the cell (once) and returns (address, size).
fn nls_view(p: &mut WinProcess, name: &str) -> Option<(u64, u64)> {
    if let Some(v) = p.nt.nls_views.iter().find(|v| v.0 == name) { return Some((v.1, v.2)); }
    let data = p.nt.nls.iter().find(|f| f.0 == name)?.1.clone();
    let size = (data.len() as u64).next_multiple_of(PAGE);
    let mut at = 0x0000_0300_0000_0000u64;
    while p.cell.is_taken(at, size) { at += process::ALLOCATION_GRANULARITY; }
    p.cell.map(at, size / PAGE, Access::USER_RW).ok()?;
    p.cell.write(at, &data)?;
    p.cell.protect(at, size / PAGE, Access::USER_RO).ok()?;
    p.nt.nls_views.push((String::from(name), at, data.len() as u64));
    Some((at, data.len() as u64))
}

/// NtInitializeNlsFiles(*ptr, *lcid, *size): locale.nls.
fn init_nls(p: &mut WinProcess, ptr: u64, lcid: u64, size: u64) -> u64 {
    let Some((at, n)) = nls_view(p, "locale.nls") else { return STATUS_OBJECT_NAME_NOT_FOUND };
    let _ = put(p, ptr, at);
    if lcid != 0 { let _ = put32(p, lcid, 0x409); }
    if size != 0 { let _ = put(p, size, n); }
    STATUS_SUCCESS
}

/// NtGetNlsSectionPtr(type, id, unknown, *ptr, *size): code pages and the other tables.
fn nls_section(p: &mut WinProcess, kind: u32, id: u32, ptr: u64, size: u64) -> u64 {
    let name = match kind {
        9 => if id == 0 { String::from("sortdefault.nls") } else { return STATUS_INVALID_PARAMETER }, // NLS_SECTION_SORTKEYS
        10 => if id == 0 { String::from("l_intl.nls") } else { return 0xC000_0001 }, // NLS_SECTION_CASEMAP
        11 => alloc::format!("c_{id:03}.nls"),      // NLS_SECTION_CODEPAGE
        12 => match id { 1 => "normnfc.nls", 2 => "normnfd.nls", 5 => "normnfkc.nls", 6 => "normnfkd.nls", 13 => "normidna.nls", _ => "" }.into(), // NLS_SECTION_NORMALIZE
        _ => return STATUS_INVALID_PARAMETER,
    };
    let Some((at, n)) = nls_view(p, &name) else { return STATUS_OBJECT_NAME_NOT_FOUND };
    let _ = put(p, ptr, at);
    if size != 0 { let _ = put(p, size, n); }
    STATUS_SUCCESS
}

/// The user's SID, S-1-5-21-0-0-0-1000 (a local account; no security model yet).
fn user_sid() -> Vec<u8> {
    let mut b = alloc::vec![1u8, 5, 0, 0, 0, 0, 0, 5];
    for v in [21u32, 0, 0, 0, 1000] { b.extend_from_slice(&v.to_le_bytes()); }
    b
}

/// NtQueryInformationToken(token, class, buffer, length, *returned): TokenUser.
fn query_token(p: &mut WinProcess, class: u32, buf: u64, len: u64, ret: u64) -> u64 {
    let out = match class {
        1 => { // TokenUser: SID_AND_ATTRIBUTES then the SID
            let mut b = alloc::vec![0u8; 16];
            b[0..8].copy_from_slice(&(buf + 16).to_le_bytes());
            b.extend_from_slice(&user_sid());
            b
        }
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if ret != 0 { let _ = put32(p, ret, out.len() as u32); }
    if len < out.len() as u64 { return 0xC000_0023; } // STATUS_BUFFER_TOO_SMALL
    if p.cell.write_user(buf, &out).is_none() { return STATUS_ACCESS_VIOLATION; }
    STATUS_SUCCESS
}

/// NtQueryVolumeInformationFile(handle, iosb, buffer, length, class).
fn volume_info(p: &mut WinProcess, handle: u64, iosb: u64, buf: u64, len: u64, class: u32) -> u64 {
    if !p.nt.handles.contains_key(&handle) && p.nt.stdout.map_or(true, |(o, e)| handle != o && handle != e) { return STATUS_INVALID_HANDLE; }
    let out: Vec<u8> = match class {
        4 => { // FileFsDeviceInformation: a disk, or a pipe for the std handles
            // The host's output is a byte stream, not a console: as a pipe (FILE_TYPE_PIPE)
            // the C runtime buffers whole lines instead of probing a console per character.
            let stream = p.nt.stdout.map_or(false, |(o, e)| handle == o || handle == e);
            let mut b = alloc::vec![0u8; 8];
            b[0..4].copy_from_slice(&(if stream { 0x11u32 } else { 7 }).to_le_bytes());
            b
        }
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if len < out.len() as u64 { return STATUS_INFO_LENGTH_MISMATCH; }
    if p.cell.write_user(buf, &out).is_none() { return STATUS_ACCESS_VIOLATION; }
    io_status(p, iosb, 0, out.len() as u64);
    STATUS_SUCCESS
}

/// NtFsControlFile(handle, event, apc, ctx, iosb, code, in, in_len, out, out_len). Only
/// FSCTL_GET_OBJECT_ID: each virtual file's id is its index, so Wine's loader can tell
/// that two paths name the same DLL.
fn fs_control(p: &mut WinProcess, handle: u64, iosb: u64, code: u32, out: u64, out_len: u64) -> u64 {
    const FSCTL_GET_OBJECT_ID: u32 = 0x9009C;
    const STATUS_INVALID_DEVICE_REQUEST: u64 = 0xC000_0010;
    let file = match p.nt.handles.get(&handle) { Some(Object::File { file, .. }) => *file, Some(_) => return STATUS_INVALID_DEVICE_REQUEST, None => return STATUS_INVALID_HANDLE };
    if code != FSCTL_GET_OBJECT_ID { return STATUS_INVALID_DEVICE_REQUEST; }
    let mut b = [0u8; 64]; // FILE_OBJECTID_BUFFER: ObjectId, then 48 bytes of extended info
    b[0..8].copy_from_slice(&(file as u64 + 1).to_le_bytes());
    b[8..16].copy_from_slice(b"crossdrs");
    if out_len < 64 { return STATUS_INFO_LENGTH_MISMATCH; }
    if p.cell.write_user(out, &b).is_none() { return STATUS_ACCESS_VIOLATION; }
    io_status(p, iosb, 0, 64);
    STATUS_SUCCESS
}

/// NtWriteFile(handle, event, apc, ctx, iosb, buffer, length, *offset, key): the console.
fn write_file(p: &mut WinProcess, handle: u64, iosb: u64, buf: u64, len: u32, host: &mut impl Host) -> u64 {
    let stream = match p.nt.stdout { Some((o, _)) if o == handle => 1, Some((_, e)) if e == handle => 2, _ => return STATUS_INVALID_HANDLE };
    let n = (len as usize).min(64 << 10);
    let mut b = alloc::vec![0u8; n];
    if p.cell.read_user(buf, &mut b).is_none() { return STATUS_ACCESS_VIOLATION; }
    host.output(stream, &b);
    io_status(p, iosb, 0, n as u64);
    STATUS_SUCCESS
}

/// NtAllocateVirtualMemory(process, *base, zero_bits, *size, type, protect).
fn allocate(p: &mut WinProcess, process: u64, base_ptr: u64, size_ptr: u64, kind: u32, protect: u32) -> u64 {
    if process != u64::MAX { return STATUS_INVALID_PARAMETER; }
    let (Some(base), Some(size)) = (get(p, base_ptr), get(p, size_ptr)) else { return STATUS_ACCESS_VIOLATION };
    if size == 0 { return STATUS_INVALID_PARAMETER; }
    // MEM_TOP_DOWN and friends are hints; only commit/reserve matter here.
    // MEM_COMMIT without an address reserves too (as on Windows).
    let kind = if base == 0 { (kind & 0x3000) | 0x2000 } else { kind & 0x3000 };
    let start = win32::virtual_alloc(p, base, size, kind, protect);
    if start == 0 { return if base != 0 { STATUS_CONFLICTING_ADDRESSES } else { STATUS_NO_MEMORY }; }
    let end = if base != 0 { (base + size).next_multiple_of(PAGE) } else { start + size.next_multiple_of(PAGE) };
    if put(p, base_ptr, start).is_none() || put(p, size_ptr, end - start).is_none() { return STATUS_ACCESS_VIOLATION; }
    STATUS_SUCCESS
}

/// `__wine_unix_call(handle, code, args)`. For ntdll's own Unix library (Wine 9's
/// `unix_call_funcs` order): 1 `unwind_builtin_dll` finds nothing, 2 `wine_dbg_write(str, len)` goes to the host's stderr,
/// 7 `system_time_precise(*time)`. Others stop the process with handle and code.
pub fn unix_call(p: &mut WinProcess, regs: &Regs, host: &mut impl Host) -> Outcome {
    let g = regs;
    let (handle, code, args) = (g.rcx, g.rdx as u32, g.r8);
    if handle == NTDLL_UNIXLIB {
        match code {
            // unwind_builtin_dll: the host has no unwind data for code outside PE images (the
            // cell's thunks), so Wine's unwinder treats such a frame as a leaf function.
            1 => return Outcome::Return(0xC000_0001), // STATUS_UNSUCCESSFUL
            2 => {
                let (Some(s), Some(n)) = (get(p, args), get(p, args + 8)) else { return Outcome::Return(STATUS_ACCESS_VIOLATION) };
                let mut b = alloc::vec![0u8; (n as usize).min(4096)];
                if p.cell.read_user(s, &mut b).is_some() { host.output(2, &b); }
                return Outcome::Return(STATUS_SUCCESS);
            }
            7 => {
                let t = EPOCH_FILETIME + host.now_ms() * 10_000;
                return Outcome::Return(if put(p, args, t).is_some() { STATUS_SUCCESS } else { STATUS_ACCESS_VIOLATION });
            }
            _ => {}
        }
    }
    Outcome::Stop(Stop::Unimplemented { dll: alloc::format!("unixlib#{handle:#x}"), name: alloc::format!("code {code}") })
}
