// SPDX-License-Identifier: GPL-3.0-only
//! CW3b: a Windows program started the Wine way. The host does what Wine's Unix half does
//! before the first instruction: it maps ntdll.dll and the program, builds the PEB, the
//! process parameters and KUSER_SHARED_DATA, and puts the Wine DLLs and NLS tables in a
//! small virtual `C:\windows\system32`. Then it calls ntdll's own `LdrInitializeThunk`,
//! and Wine's loader does the rest in the cell: it loads kernel32.dll and kernelbase.dll
//! through NtOpenFile/NtCreateSection/NtMapViewOfSection, fixes the program's imports,
//! runs the DLLs' DllMain, and jumps to the program through RtlUserThreadStart and
//! kernel32's BaseThreadInitThunk. Every system call on the way is served in `nt`.
use crate::cell::{self, Access, PAGE};
use crate::process::{self, Host, LoadError, Stop, WinProcess};
use crate::win32;
use alloc::string::String;
use alloc::vec::Vec;
use drag_vmm::memory::Ram;
use drag_vmm::vmm::Vcpu;

/// The process parameters, environment and strings (host-built, like Wine's Unix half).
pub const PARAMS_VA: u64 = 0x0000_7FF7_FFC0_0000;
const PARAMS_PAGES: u64 = 8;
/// Console handles in the process parameters (multiples of 4, not console-style 3).
pub const STDIN_HANDLE: u64 = 0x10;
pub const STDOUT_HANDLE: u64 = 0x14;
pub const STDERR_HANDLE: u64 = 0x18;
/// Wine's API set schema (`apisetschema.dll`, mapped as a plain file, read-only).
pub const APISET_VA: u64 = 0x0000_7FF7_FFA0_0000;
const APISET_MAX_PAGES: u64 = 64;

/// What a Wine process starts from.
pub struct WineSetup<'a> {
    pub ntdll: &'a [u8],
    pub exe: &'a [u8],
    /// The program's DOS path, e.g. `C:\test\wine_hello.exe`.
    pub exe_path: &'a str,
    /// DLLs that go in `C:\windows\system32` (kernel32.dll, kernelbase.dll, ...).
    pub system32: &'a [(&'a str, &'a [u8])],
    /// NLS tables (locale.nls, l_intl.nls, c_1252.nls, ...) for Wine's locale code.
    pub nls: &'a [(&'a str, &'a [u8])],
    /// Wine's apisetschema.dll: its `.apiset` section maps `api-ms-win-crt-*` and the other
    /// API set names that MSVC-built programs import onto the DLLs that implement them.
    pub apiset: Option<&'a [u8]>,
}

/// Maps `file` read-only at APISET_VA and returns the address of its `.apiset` section's
/// API_SET_NAMESPACE (version 6), as Wine's Unix loader does (`load_apiset_dll`).
fn map_apiset(p: &mut WinProcess, file: &[u8]) -> Result<u64, LoadError> {
    let img = crate::pe::Image::parse(file)?;
    let sec = img.sections.iter().find(|s| &s.name == b".apiset\0").ok_or(LoadError::Pe(crate::pe::Error::NotPe))?;
    let at = sec.raw_offset as usize;
    let version = file.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if version != Some(6) || (sec.virtual_size as usize) > file.len().saturating_sub(at) { return Err(LoadError::Pe(crate::pe::Error::NotPe)); }
    let pages = (file.len() as u64).div_ceil(PAGE);
    if pages > APISET_MAX_PAGES { return Err(LoadError::Cell(cell::Error::OutOfMemory)); }
    p.cell.map(APISET_VA, pages, Access::USER_RW)?;
    p.cell.write(APISET_VA, file).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
    p.cell.protect(APISET_VA, pages, Access::USER_RO)?;
    Ok(APISET_VA + at as u64)
}

fn utf16(s: &str) -> Vec<u8> { s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect() }

/// Loads ntdll.dll and the program into a fresh cell over `ram` and lays out everything
/// Wine's PE half expects to find at its first instruction.
pub fn load(ram: Ram, setup: &WineSetup) -> Result<WinProcess, LoadError> {
    let mut p = WinProcess::load_dll(ram, setup.ntdll)?;
    crate::nt::prepare_ntdll(&mut p).map_err(|_| LoadError::Pe(crate::pe::Error::NotPe))?;
    let ntdll_size = p.image.size_of_image as u64;
    p.nt.modules.push((p.base, ntdll_size));
    let exe = crate::pe::Image::parse(setup.exe)?;
    if exe.is_dll() { return Err(LoadError::Pe(crate::pe::Error::NotPe)); }
    let exe_base = exe.preferred_base;
    p.map_image(setup.exe, exe_base)?;
    p.nt.modules.push((exe_base, exe.size_of_image as u64));
    p.nt.main_image = Some((exe_base, exe.clone()));
    p.nt.main_entry = exe_base + exe.entry_rva as u64;
    // Files: the program, the system DLLs (ntdll too, for its own module entry), NLS.
    let exe_key = crate::nt::normalize(setup.exe_path);
    p.nt.add_file(&exe_key, setup.exe.to_vec());
    p.nt.add_file("c:\\windows\\system32\\ntdll.dll", setup.ntdll.to_vec());
    for (key, base) in [(exe_key.clone(), exe_base), (String::from("c:\\windows\\system32\\ntdll.dll"), p.base)] {
        if let Some(f) = p.nt.file(&key) { p.nt.images.push((base, f)); }
    }
    for (name, data) in setup.system32 { p.nt.add_file(&alloc::format!("c:\\windows\\system32\\{}", name.to_ascii_lowercase()), data.to_vec()); }
    for (name, data) in setup.nls { p.nt.nls.push((name.to_ascii_lowercase(), data.to_vec())); }

    // PEB: the program's base, the parameters, OS version, one processor.
    let peb = process::PEB_VA;
    let w = |p: &mut WinProcess, at: u64, v: u64| p.cell.write_u64(peb + at, v).ok_or(LoadError::Cell(cell::Error::OutOfMemory));
    w(&mut p, win32::PEB_IMAGE_BASE, exe_base)?;
    w(&mut p, 0x20, PARAMS_VA)?; // ProcessParameters
    let w32 = |p: &mut WinProcess, at: u64, v: u32| p.cell.write_u32(peb + at, v).ok_or(LoadError::Cell(cell::Error::OutOfMemory));
    if let Some(file) = setup.apiset { let map = map_apiset(&mut p, file)?; w(&mut p, 0x68, map)?; } // ApiSetMap
    w32(&mut p, 0xB8, 1)?;       // NumberOfProcessors
    w32(&mut p, 0x118, 10)?;     // OSMajorVersion
    w32(&mut p, 0x11C, 0)?;      // OSMinorVersion
    w32(&mut p, 0x120, 19045)?;  // OSBuildNumber
    w32(&mut p, 0x124, 2)?;      // OSPlatformId (VER_PLATFORM_WIN32_NT)
    w32(&mut p, 0x128, exe.subsystem as u32)?; // ImageSubsystem
    w32(&mut p, 0x12C, 6)?;      // ImageSubsystemMajorVersion
    // The TEB's stack fields already describe the thread's stack; the PEB pointer is set.
    // The rest is what Wine's Unix half fills in (virtual.c init_teb): the activation
    // context stack, the static Unicode string, the real client id.
    let teb = process::TEB_VA;
    let tw = |p: &mut WinProcess, at: u64, v: u64| p.cell.write_u64(teb + at, v).ok_or(LoadError::Cell(cell::Error::OutOfMemory));
    tw(&mut p, 0x2C8, teb + 0x290)?; // ActivationContextStackPointer
    tw(&mut p, 0x298, teb + 0x298)?; // ActivationContextStack.FrameListCache.Flink
    tw(&mut p, 0x2A0, teb + 0x298)?; // ...Blink
    p.cell.write_u32(teb + 0x1258, 522 << 16).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?; // StaticUnicodeString
    tw(&mut p, 0x1260, teb + 0x1268)?;
    tw(&mut p, 0x7D8, process::PROCESS_ID as u64)?; // RealClientId
    tw(&mut p, 0x7E0, process::THREAD_ID as u64)?;

    // RTL_USER_PROCESS_PARAMETERS (normalized: absolute pointers), then its strings.
    p.cell.map(PARAMS_VA, PARAMS_PAGES, Access::USER_RW)?;
    let mut strings = Vec::new();
    let mut add = |s: &[u8]| -> (u64, u16) {
        let at = PARAMS_VA + 0x1000 + strings.len() as u64;
        strings.extend_from_slice(s);
        strings.extend_from_slice(&[0, 0]);
        while strings.len() % 8 != 0 { strings.push(0); }
        (at, s.len() as u16)
    };
    let dir = setup.exe_path.rsplit_once('\\').map(|(d, _)| alloc::format!("{d}\\")).unwrap_or_else(|| String::from("C:\\"));
    let image = add(&utf16(setup.exe_path));
    let cmdline = add(&utf16(&alloc::format!("\"{}\"", setup.exe_path)));
    let curdir = add(&utf16(&dir));
    let dllpath = add(&utf16("C:\\windows\\system32;C:\\windows"));
    let desktop = add(&utf16("WinSta0\\Default"));
    let title = add(&utf16(setup.exe_path));
    let env_text = "PATH=C:\\windows\\system32;C:\\windows\0SystemRoot=C:\\windows\0WINEDEBUG=-all\0\0";
    let env = add(&utf16(env_text));
    let mut prm = alloc::vec![0u8; 0x410];
    let put32 = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |b: &mut [u8], at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
    let ustr = |b: &mut [u8], at: usize, (va, len): (u64, u16)| {
        b[at..at + 2].copy_from_slice(&len.to_le_bytes());
        b[at + 2..at + 4].copy_from_slice(&(len + 2).to_le_bytes());
        b[at + 8..at + 16].copy_from_slice(&va.to_le_bytes());
    };
    put32(&mut prm, 0x00, 0x410); // AllocationSize
    put32(&mut prm, 0x04, 0x410); // Size
    put32(&mut prm, 0x08, 1);     // PROCESS_PARAMS_FLAG_NORMALIZED
    put64(&mut prm, 0x20, STDIN_HANDLE);
    put64(&mut prm, 0x28, STDOUT_HANDLE);
    put64(&mut prm, 0x30, STDERR_HANDLE);
    ustr(&mut prm, 0x38, curdir);
    ustr(&mut prm, 0x50, dllpath);
    ustr(&mut prm, 0x60, image);
    ustr(&mut prm, 0x70, cmdline);
    put64(&mut prm, 0x80, env.0);
    ustr(&mut prm, 0xB0, title);
    ustr(&mut prm, 0xC0, desktop);
    put64(&mut prm, 0x3F0, env.1 as u64 + 2); // EnvironmentSize
    if strings.len() as u64 > (PARAMS_PAGES - 1) * PAGE { return Err(LoadError::Cell(cell::Error::OutOfMemory)); }
    p.cell.write(PARAMS_VA, &prm).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
    p.cell.write(PARAMS_VA + 0x1000, &strings).ok_or(LoadError::Cell(cell::Error::OutOfMemory))?;
    p.nt.stdout = Some((STDOUT_HANDLE, STDERR_HANDLE));
    Ok(p)
}

/// Starts the process the way Wine's Unix half does: `LdrInitializeThunk(context)` on the
/// thread's stack, where the CONTEXT says to continue at `RtlUserThreadStart(entry, PEB)`.
/// Runs until the program exits (ExitProcess is NtTerminateProcess) or stops.
pub fn run(p: &mut WinProcess, cpu: &mut impl Vcpu, host: &mut impl Host) -> Stop {
    let (Some(init), Some(start)) = (p.export("LdrInitializeThunk"), p.export("RtlUserThreadStart")) else {
        return Stop::Unimplemented { dll: String::from("ntdll.dll"), name: String::from("LdrInitializeThunk") };
    };
    // The initial CONTEXT, below the call frame `call` lays out.
    let ctx_va = (process::STACK_TOP - 0x1000 - win32::CONTEXT_SIZE) & !0xF;
    let mut regs = sex_pdx::vm::Regs::default();
    regs.rip = start;
    regs.rsp = (process::STACK_TOP - 0x2000) & !0xF;
    regs.rsp -= 8;
    regs.rflags = cell::USER_RFLAGS;
    // The guest's Windows x64 argument registers (a guest register file, not SexOS IPC).
    { let g = &mut regs; g.rcx = p.nt.main_entry; g.rdx = process::PEB_VA; }
    let mut c = [0u8; win32::CONTEXT_SIZE as usize];
    win32::write_context(&mut c, &regs);
    if p.cell.write_user(ctx_va, &c).is_none() { return Stop::CellFault { vector: 0xFE, rip: ctx_va }; }
    // The call frame must sit below the CONTEXT.
    match p.call_at(cpu, host, init, &[ctx_va, 0, 0, 0], ctx_va - 0x100) {
        Ok(v) => Stop::Returned(v),
        Err(s) => s,
    }
}

/// Host scratch space for `load_library` (the last page of the process parameters).
const SCRATCH_VA: u64 = PARAMS_VA + (PARAMS_PAGES - 1) * PAGE;

/// Initializes the process without running the program: `LdrInitializeThunk` with a
/// CONTEXT that continues at the host call's return thunk instead of RtlUserThreadStart.
/// Wine's loader loads the program's DLLs (kernel32, kernelbase, ...) and runs their
/// DllMain; the process is then ready for `load_library` and host calls, as a plugin host
/// process is.
pub fn start(p: &mut WinProcess, cpu: &mut impl Vcpu, host: &mut impl Host) -> Result<(), Stop> {
    let Some(init) = p.export("LdrInitializeThunk") else {
        return Err(Stop::Unimplemented { dll: String::from("ntdll.dll"), name: String::from("LdrInitializeThunk") });
    };
    let ctx_va = (process::STACK_TOP - 0x1000 - win32::CONTEXT_SIZE) & !0xF;
    let mut regs = sex_pdx::vm::Regs::default();
    regs.rip = process::CALL_RETURN_VA;
    regs.rsp = (process::STACK_TOP - 0x2000) & !0xF;
    regs.rflags = cell::USER_RFLAGS;
    let mut c = [0u8; win32::CONTEXT_SIZE as usize];
    win32::write_context(&mut c, &regs);
    if p.cell.write_user(ctx_va, &c).is_none() { return Err(Stop::CellFault { vector: 0xFE, rip: ctx_va }); }
    p.call_at(cpu, host, init, &[ctx_va, 0, 0, 0], ctx_va - 0x100).map(|_| ())
}

/// Why a DLL did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadLibraryError { Status(u32), Stopped(Stop), Bad(&'static str) }

/// Loads the DLL at DOS path `path` (a file in the process's virtual file system) with
/// Wine's own `LdrLoadDll`: imports, relocation, TLS and DllMain are Wine's loader's.
/// Returns its base and exports.
pub fn load_library(p: &mut WinProcess, cpu: &mut impl Vcpu, host: &mut impl Host, path: &str) -> Result<(u64, Vec<(String, u64)>), LoadLibraryError> {
    let ldr = p.export("LdrLoadDll").ok_or(LoadLibraryError::Bad("no LdrLoadDll"))?;
    let name = utf16(path);
    if name.len() + 0x40 > PAGE as usize { return Err(LoadLibraryError::Bad("path too long")); }
    // UNICODE_STRING at +0, the module handle at +0x10, the text at +0x20.
    let mut b = alloc::vec![0u8; 0x20 + name.len() + 2];
    b[0..2].copy_from_slice(&(name.len() as u16).to_le_bytes());
    b[2..4].copy_from_slice(&(name.len() as u16 + 2).to_le_bytes());
    b[8..16].copy_from_slice(&(SCRATCH_VA + 0x20).to_le_bytes());
    b[0x20..0x20 + name.len()].copy_from_slice(&name);
    p.cell.write_user(SCRATCH_VA, &b).ok_or(LoadLibraryError::Bad("scratch"))?;
    let status = p.call(cpu, host, ldr, &[0, 0, SCRATCH_VA, SCRATCH_VA + 0x10], None).map_err(LoadLibraryError::Stopped)? as u32;
    if status != 0 { return Err(LoadLibraryError::Status(status)); }
    let mut h = [0u8; 8];
    p.cell.read_user(SCRATCH_VA + 0x10, &mut h).ok_or(LoadLibraryError::Bad("handle"))?;
    let base = u64::from_le_bytes(h);
    // The exports, read from the image as Wine mapped it.
    let file = p.nt.files.iter().find(|f| f.0 == crate::nt::normalize(path)).map(|f| f.1.clone()).ok_or(LoadLibraryError::Bad("file"))?;
    let img = crate::pe::Image::parse(&file).map_err(|_| LoadLibraryError::Bad("not PE"))?;
    let mut mem = alloc::vec![0u8; img.size_of_image as usize];
    p.cell.read_user(base, &mut mem).ok_or(LoadLibraryError::Bad("image"))?;
    let exports = img.exports(&mem).map_err(|_| LoadLibraryError::Bad("exports"))?.into_iter().map(|(n, rva)| (n, base + rva as u64)).collect();
    Ok((base, exports))
}
