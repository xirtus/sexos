// SPDX-License-Identifier: GPL-3.0-only
//! Shared by the KVM and native tests: Wine's ntdll.dll exercised through a vCPU.
#![allow(dead_code)]
use drag_vmm::vmm::Vcpu;

/// A host with no console of interest.
pub struct Quiet;
impl sex_crossdress::process::Host for Quiet {
    fn output(&mut self, _: u32, _: &[u8]) {}
    fn now_ms(&mut self) -> u64 { 1 }
    fn sleep(&mut self, _: u32) {}
}

/// Wine's ntdll.dll (Wine 9.0 PE build, `scripts/fetch_wine_pe.sh`), from
/// CROSSDRESS_WINE_DIR; None (SKIP) when it is not there.
pub fn wine_dll(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("CROSSDRESS_WINE_DIR")?;
    std::fs::read(std::path::Path::new(&dir).join(name)).ok()
}

/// Calls into Wine's real ntdll.dll: CRT and Rtl functions that run purely in the PE half,
/// then NT system calls through Wine's own stubs and the cell's dispatcher, then Wine's
/// heap (RtlCreateHeap/RtlAllocateHeap) on top of NtAllocateVirtualMemory.
pub fn exercise_ntdll(cpu: &mut impl Vcpu, p: &mut sex_crossdress::process::WinProcess) -> Vec<String> {
    let mut con = Quiet;
    let mut log = Vec::new();
    let scratch = sex_crossdress::process::STACK_TOP - 0x8000;
    let f = |p: &sex_crossdress::process::WinProcess, n: &str| p.export(n).unwrap_or_else(|| panic!("ntdll exports {n}"));
    // RtlComputeCrc32(0, "123456789", 9) is the CRC-32 check value.
    p.cell.write_user(scratch, b"123456789").unwrap();
    let crc = p.call(cpu, &mut con, f(p, "RtlComputeCrc32"), &[0, scratch, 9], None);
    log.push(format!("RtlComputeCrc32 -> {crc:x?}"));
    assert_eq!(crc, Ok(0xCBF4_3926), "RtlComputeCrc32");
    // sprintf(buf, "%s %d %x", "wine", 9, 255) in ntdll's own CRT.
    p.cell.write_user(scratch + 0x100, b"%s %d %x\0").unwrap();
    p.cell.write_user(scratch + 0x200, b"wine\0").unwrap();
    let n = p.call(cpu, &mut con, f(p, "sprintf"), &[scratch + 0x300, scratch + 0x100, scratch + 0x200, 9, 255], None);
    let mut out = [0u8; 9];
    p.cell.read_user(scratch + 0x300, &mut out).unwrap();
    log.push(format!("sprintf -> {n:?} {:?}", String::from_utf8_lossy(&out)));
    assert_eq!((n, &out), (Ok(9), b"wine 9 ff"), "sprintf");
    // NtQuerySystemTime through Wine's stub: a system call.
    let t = p.call(cpu, &mut con, f(p, "NtQuerySystemTime"), &[scratch + 0x400], None);
    let ft = p.cell.read_u64(scratch + 0x400).unwrap();
    log.push(format!("NtQuerySystemTime -> {t:x?} {ft}"));
    assert_eq!(t, Ok(0));
    assert!(ft >= sex_crossdress::nt::EPOCH_FILETIME);
    // Wine's heap: RtlCreateHeap(HEAP_GROWABLE), RtlAllocateHeap(HEAP_ZERO_MEMORY, 1000),
    // write, RtlSizeHeap, RtlFreeHeap.
    let heap = p.call(cpu, &mut con, f(p, "RtlCreateHeap"), &[2, 0, 0, 0, 0, 0], None);
    log.push(format!("RtlCreateHeap -> {heap:x?} syscalls={}", p.counters.syscalls));
    let heap = heap.expect("RtlCreateHeap ran");
    assert_ne!(heap, 0, "RtlCreateHeap");
    let block = p.call(cpu, &mut con, f(p, "RtlAllocateHeap"), &[heap, 8, 1000], None).expect("RtlAllocateHeap ran");
    log.push(format!("RtlAllocateHeap -> {block:#x}"));
    assert_ne!(block, 0);
    let mut z = vec![1u8; 1000];
    p.cell.read_user(block, &mut z).unwrap();
    assert!(z.iter().all(|&b| b == 0), "HEAP_ZERO_MEMORY");
    p.cell.write_user(block, &[0xAB; 1000]).unwrap();
    let size = p.call(cpu, &mut con, f(p, "RtlSizeHeap"), &[heap, 0, block], None);
    assert_eq!(size, Ok(1000), "RtlSizeHeap");
    let freed = p.call(cpu, &mut con, f(p, "RtlFreeHeap"), &[heap, 0, block], None);
    assert_eq!(freed.map(|v| v as u8), Ok(1), "RtlFreeHeap");
    log.push(format!("heap ok: syscalls={} unix_calls={}", p.counters.syscalls, p.counters.unix_calls));
    log
}


/// A host that keeps what the program writes (stdout and stderr together).
#[derive(Default)]
pub struct Keep { pub text: Vec<u8> }
impl sex_crossdress::process::Host for Keep {
    fn output(&mut self, _: u32, b: &[u8]) {
        // With CROSSDRESS_NT_TRACE the trace is also echoed live, so a test that panics
        // still shows the calls that led there.
        if std::env::var_os("CROSSDRESS_NT_TRACE").is_some() { eprint!("{}", String::from_utf8_lossy(b)); }
        self.text.extend_from_slice(b);
    }
    fn now_ms(&mut self) -> u64 { 1 }
    fn sleep(&mut self, _: u32) {}
}

/// The Wine test program in a fresh cell, set up the Wine way (`sex_crossdress::wine`).
/// None (SKIP) without CROSSDRESS_WINE_DIR.
pub fn wine_program() -> Option<sex_crossdress::process::WinProcess> {
    // CROSSDRESS_WINE_EXE: the program as another LLVM rebuilt it (scripts/drag_host_gate.sh).
    let exe = std::env::var_os("CROSSDRESS_WINE_EXE").map(|f| std::fs::read(f).expect("CROSSDRESS_WINE_EXE")).unwrap_or_else(|| sex_crossdress::WINE_HELLO_EXE.to_vec());
    wine_process(&exe, "C:\\test\\wine_hello.exe")
}

/// The CW4 C runtime program (CROSSDRESS_CRT_EXE overrides it, as above).
pub fn crt_program() -> Option<sex_crossdress::process::WinProcess> {
    let exe = std::env::var_os("CROSSDRESS_CRT_EXE").map(|f| std::fs::read(f).expect("CROSSDRESS_CRT_EXE")).unwrap_or_else(|| sex_crossdress::WINE_CRT_EXE.to_vec());
    wine_process(&exe, "C:\\test\\wine_crt.exe")
}

/// The CW4 MSVC-style program (CROSSDRESS_MSVC_EXE overrides it, as above).
pub fn msvc_program() -> Option<sex_crossdress::process::WinProcess> {
    let exe = std::env::var_os("CROSSDRESS_MSVC_EXE").map(|f| std::fs::read(f).expect("CROSSDRESS_MSVC_EXE")).unwrap_or_else(|| sex_crossdress::WINE_MSVC_EXE.to_vec());
    wine_process(&exe, "C:\\test\\wine_msvc.exe")
}

/// `exe` in a fresh cell, set up the Wine way, at DOS path `path`.
pub fn wine_process(exe: &[u8], path: &str) -> Option<sex_crossdress::process::WinProcess> {
    let ntdll = wine_dll("ntdll.dll")?;
    let dlls: Vec<(&str, Vec<u8>)> = ["kernel32.dll", "kernelbase.dll", "ucrtbase.dll", "msvcrt.dll", "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"].iter().filter_map(|n| Some((*n, wine_dll(n)?))).collect();
    let nls: Vec<(&str, Vec<u8>)> = ["locale.nls", "l_intl.nls", "c_1252.nls", "c_437.nls", "c_20127.nls", "sortdefault.nls", "normnfc.nls", "normnfd.nls"].iter().filter_map(|n| Some((*n, wine_dll(n)?))).collect();
    let system32: Vec<(&str, &[u8])> = dlls.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    let nls_ref: Vec<(&str, &[u8])> = nls.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    let apiset = wine_dll("apisetschema.dll");
    let setup = sex_crossdress::wine::WineSetup { ntdll: &ntdll, exe, exe_path: path, system32: &system32, nls: &nls_ref, apiset: apiset.as_deref() };
    let mut p = sex_crossdress::wine::load(drag_vmm::memory::Ram::new(256 << 20).unwrap(), &setup).unwrap();
    p.nt.trace = std::env::var_os("CROSSDRESS_NT_TRACE").is_some();
    Some(p)
}

/// Where the Wine-hosted plugin tests put the plugin in the virtual file system.
pub const CLAP_PATH: &str = "C:\\Program Files\\Common Files\\CLAP\\gain.clap";

/// A Wine process ready to host a plugin: the CW3 program's image as the process image
/// (its entry never runs), `plugin` at CLAP_PATH. None (SKIP) without CROSSDRESS_WINE_DIR.
pub fn wine_plugin_host(plugin: &[u8]) -> Option<sex_crossdress::process::WinProcess> {
    let mut p = wine_process(sex_crossdress::WINE_HELLO_EXE, "C:\\Program Files\\Crossdress\\host.exe")?;
    p.nt.add_file(&sex_crossdress::nt::normalize(CLAP_PATH), plugin.to_vec());
    Some(p)
}

fn sine(frames: usize, offset: usize, hz: f64) -> Vec<f32> {
    (0..frames).map(|i| (((offset + i) as f64) * hz * 2.0 * core::f64::consts::PI / 48_000.0).sin() as f32 * 0.5).collect()
}

/// Opens the test gain plugin and runs the CX1 audio checks (descriptor, parameters,
/// host callbacks, three blocks with sample-accurate gain changes compared bit for bit,
/// changes echoed back, close). Returns lines to print.
pub fn drive_gain_plugin(plugin: &mut sex_crossdress::clap::ClapPlugin, cpu: &mut impl Vcpu, host: &mut impl sex_crossdress::process::Host) -> Vec<String> {
    use sex_crossdress::clap::ParamChange;
    let mut log = Vec::new();
    let r = plugin.open(cpu, host, CLAP_PATH);
    log.push(format!("open={r:?} counters={:?}", plugin.process.counters));
    if let Err(e) = r { panic!("open: {e:?} ({log:?})"); }
    let d = &plugin.descriptor;
    log.push(format!("id={} name={:?} params={:?}", d.id, d.name, plugin.params));
    assert_eq!(d.id, "org.sexos.crossdress.test-gain");
    assert_eq!(d.features, ["audio-effect", "utility", "stereo"]);
    assert_eq!(plugin.params.len(), 1);
    assert!(plugin.requests.extensions >= 1);
    assert_eq!((plugin.requests.callback, plugin.main_thread_calls), (1, 1));
    assert_eq!(plugin.param_value(cpu, host, 0).unwrap(), Some(1.0));
    plugin.activate(cpu, host, 48_000.0, 512).unwrap();
    let blocks: [(usize, Vec<ParamChange>); 3] = [
        (512, vec![]),
        (512, vec![ParamChange { time: 100, id: 0, value: 0.5 }, ParamChange { time: 300, id: 0, value: 1.5 }]),
        (256, vec![]),
    ];
    let (mut gain, mut at) = (1.0f32, 0usize);
    let mut reported = Vec::new();
    for (frames, changes) in &blocks {
        let (l, r) = (sine(*frames, at, 997.0), sine(*frames, at, 2000.0));
        let (mut ol, mut or) = (vec![9.0f32; *frames], vec![9.0f32; *frames]);
        let pushed = plugin.process(cpu, host, [&l, &r], [&mut ol, &mut or], changes).unwrap();
        for i in 0..*frames {
            for c in changes.iter().filter(|c| c.time as usize == i) { gain = c.value as f32; }
            assert_eq!((ol[i].to_bits(), or[i].to_bits()), ((l[i] * gain).to_bits(), (r[i] * gain).to_bits()), "frame {i} of the block at {at}");
        }
        reported.extend(pushed);
        at += frames;
    }
    assert_eq!(reported, [ParamChange { time: 100, id: 0, value: 0.5 }, ParamChange { time: 300, id: 0, value: 1.5 }]);
    assert_eq!(plugin.param_value(cpu, host, 0).unwrap(), Some(1.5));
    plugin.close(cpu, host).unwrap();
    log.push(format!("processed={at} frames bit-exact, reported={reported:?} counters={:?}", plugin.process.counters));
    log
}
