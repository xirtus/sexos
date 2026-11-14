//! Disposable offline CLAP helper. No window, network request or parent pointers.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use moan_plugin::{clap::Processor, elf, job, mapped::Mapped};
use sex_app::files::direct as files;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[moanplug.fatal] payload=redacted");
    sex_pdx::task_finish(1)
}
fn read(path: &[u8], limit: usize) -> Result<Vec<u8>, ()> {
    let stat = files::stat(path).map_err(|_| ())?;
    if stat.dir || stat.size == 0 || stat.size > limit as u64 {
        return Err(());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(stat.size as usize)
        .map_err(|_| ())?;
    bytes.resize(stat.size as usize, 0);
    let mut at = 0;
    while at < bytes.len() {
        let n = files::read_at(path, at as u64, &mut bytes[at..]).map_err(|_| ())?;
        if n == 0 || n > bytes.len() - at {
            return Err(());
        }
        at += n;
    }
    let mut extra = [0];
    if files::read_at(path, at as u64, &mut extra).map_err(|_| ())? != 0 {
        return Err(());
    }
    Ok(bytes)
}
fn run() -> Result<(), ()> {
    let mut buffer = [0; 256];
    let len = files::args(&mut buffer);
    let args = &buffer[..len];
    let parts: Vec<&[u8]> = args
        .split(|b| *b == b' ')
        .filter(|t| !t.is_empty())
        .collect();
    if parts.len() != 5 || parts[0].len() != 32 {
        return Err(());
    }
    let mut nonce = [0; 16];
    for (i, c) in parts[0].chunks_exact(2).enumerate() {
        fn digit(b: u8) -> Option<u8> {
            match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            }
        }
        nonce[i] = digit(c[0]).ok_or(())? * 16 + digit(c[1]).ok_or(())?;
    }
    if nonce == [0; 16]
        || parts[1..4]
            .iter()
            .any(|p| !p.starts_with(b"@/") || p.len() > 192 || p.contains(&0))
        || parts[1] == parts[2]
        || parts[1] == parts[3]
        || parts[2] == parts[3]
    {
        return Err(());
    }
    let binary = read(parts[1], elf::MAX_FILE)?;
    let input = read(
        parts[2],
        job::HEADER + job::MAX_FRAMES as usize * 8 + job::MAX_PARAMS * 16,
    )?;
    if input.len() < job::HEADER {
        return Err(());
    }
    let frames = u32::from_le_bytes(input[28..32].try_into().map_err(|_| ())?);
    let request = job::decode_request(&input, nonce).map_err(|_| ())?;
    drop(input);
    let mapped = Mapped::load(&binary).map_err(|_| ())?;
    let start = sex_pdx::get_ticks();
    let rendered = match parts[4] {
        b"clap" => {
            if !request.parameters.is_empty() {
                return Err(());
            }
            let entry = unsafe { mapped.export(&binary, b"clap_entry").map_err(|_| ())? };
            let mut plugin = unsafe { Processor::open(entry.cast()).map_err(|_| ())? };
            plugin.render(&request.pcm).map_err(|_| ())?
        }
        b"vst3" => unsafe {
            let entry = core::mem::transmute::<
                *const u8,
                unsafe extern "system" fn(*mut core::ffi::c_void) -> bool,
            >(mapped.export(&binary, b"ModuleEntry").map_err(|_| ())?);
            let exit = core::mem::transmute::<*const u8, unsafe extern "system" fn() -> bool>(
                mapped.export(&binary, b"ModuleExit").map_err(|_| ())?,
            );
            if !entry(core::ptr::null_mut()) {
                return Err(());
            }
            struct End(unsafe extern "system" fn() -> bool);
            impl Drop for End {
                fn drop(&mut self) {
                    unsafe {
                        (self.0)();
                    }
                }
            }
            let end = End(exit);
            let factory = core::mem::transmute(
                mapped
                    .export(&binary, b"GetPluginFactory")
                    .map_err(|_| ())?,
            );
            let result = {
                let mut plugin = moan_plugin::vst::Processor::open(factory).map_err(|_| ())?;
                plugin
                    .render_with_parameters(&request.pcm, &request.parameters)
                    .map_err(|_| ())?
            };
            drop(end);
            result
        },
        _ => return Err(()),
    };
    drop(mapped);
    let encoded = job::encode(nonce, &rendered).map_err(|_| ())?;
    files::create(parts[3]).map_err(|_| ())?;
    files::truncate(parts[3], 0).map_err(|_| ())?;
    let mut at = 0;
    while at < encoded.len() {
        let n = files::write_at(parts[3], at as u64, &encoded[at..]).map_err(|_| ())?;
        if n == 0 || n > encoded.len() - at {
            return Err(());
        }
        at += n;
    }
    files::sync().map_err(|_| ())?;
    sex_pdx::serial_record!(
        "[moanplug.rendered] frames={} blocks={} ticks={} format={} offline=1",
        frames,
        frames.div_ceil(job::BLOCK as u32),
        sex_pdx::get_ticks().saturating_sub(start),
        core::str::from_utf8(parts[4]).unwrap_or("invalid")
    );
    Ok(())
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let own = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT)
        .expect("helper identity");
    sex_pdx::serial_record!(
        "[moanplug.started] pd={} task={} network_requested=0",
        own.target_pd,
        own.task_id
    );
    let code = if run().is_ok() { 0 } else { 2 };
    sex_app::launch::exit(code)
}
