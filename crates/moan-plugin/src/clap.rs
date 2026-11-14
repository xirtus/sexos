//! Real CLAP entry/factory/plugin lifecycle. All unsafe native calls belong in a
//! disposable helper; malformed native pointers cannot be made safe in-process.
use crate::job::{BLOCK, MAX_FRAMES, RATE};
use alloc::{boxed::Box, vec::Vec};
use clap_sys::{
    audio_buffer::*, entry::*, events::*, ext::audio_ports::*, factory::plugin_factory::*, host::*,
    plugin::*, process::*, version::*,
};
use core::sync::atomic::{AtomicBool, Ordering};
use core::{
    ffi::{c_char, c_void},
    ptr,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Version,
    Factory,
    Plugin,
    Lifecycle,
    Ports,
    Frames,
    Audio,
    Process,
    Restart,
}
struct Context {
    restart: AtomicBool,
    callback: AtomicBool,
}
pub struct Processor {
    entry: *const clap_plugin_entry,
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    context: Box<Context>,
    initialized: bool,
    active: bool,
    processing: bool,
}
unsafe extern "C" fn extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn restart(h: *const clap_host) {
    (*((*h).host_data as *const Context))
        .restart
        .store(true, Ordering::Release)
}
unsafe extern "C" fn request_process(_: *const clap_host) {}
unsafe extern "C" fn callback(h: *const clap_host) {
    (*((*h).host_data as *const Context))
        .callback
        .store(true, Ordering::Release)
}
unsafe extern "C" fn event_count(_: *const clap_input_events) -> u32 {
    0
}
unsafe extern "C" fn event_get(_: *const clap_input_events, _: u32) -> *const clap_event_header {
    ptr::null()
}
unsafe extern "C" fn output(_: *const clap_output_events, _: *const clap_event_header) -> bool {
    false
}
impl Processor {
    /// # Safety
    /// `entry` must point to a relocated genuine CLAP module for this ABI. It and
    /// its entire mapping must outlive this object. Invoke only in a child domain.
    pub unsafe fn open(entry: *const clap_plugin_entry) -> Result<Self, Error> {
        if entry.is_null() || (*entry).clap_version.major != 1 {
            return Err(Error::Version);
        }
        let mut context = Box::new(Context {
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(false),
        });
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *context as *mut Context).cast(),
            name: c"Moan offline".as_ptr(),
            vendor: c"SexOS".as_ptr(),
            url: c"https://github.com/xirtus/SexOS".as_ptr(),
            version: c"0.1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(restart),
            request_process: Some(request_process),
            request_callback: Some(callback),
        });
        let mut this = Self {
            entry,
            plugin: ptr::null(),
            _host: host,
            context,
            initialized: false,
            active: false,
            processing: false,
        };
        if !(*entry).init.ok_or(Error::Lifecycle)?(c"plugin.clap".as_ptr()) {
            return Err(Error::Lifecycle);
        }
        this.initialized = true;
        let factory = (*entry).get_factory.ok_or(Error::Factory)?(CLAP_PLUGIN_FACTORY_ID.as_ptr())
            as *const clap_plugin_factory;
        if factory.is_null() {
            return Err(Error::Factory);
        }
        let count = (*factory).get_plugin_count.ok_or(Error::Factory)?(factory);
        // Explicitly select first descriptor; arbitrary user-controlled index is never used.
        if count == 0 || count > 64 {
            return Err(Error::Factory);
        }
        let desc = (*factory).get_plugin_descriptor.ok_or(Error::Factory)?(factory, 0);
        if desc.is_null() || (*desc).id.is_null() || (*desc).clap_version.major != 1 {
            return Err(Error::Plugin);
        }
        this.plugin =
            (*factory).create_plugin.ok_or(Error::Factory)?(factory, &*this._host, (*desc).id);
        if this.plugin.is_null() {
            return Err(Error::Plugin);
        }
        let p = &*this.plugin;
        if p.destroy.is_none()
            || p.deactivate.is_none()
            || p.stop_processing.is_none()
            || p.process.is_none()
        {
            return Err(Error::Lifecycle);
        }
        if !p.init.ok_or(Error::Lifecycle)?(this.plugin) {
            return Err(Error::Lifecycle);
        }
        let ports = p.get_extension.ok_or(Error::Ports)?(this.plugin, CLAP_EXT_AUDIO_PORTS.as_ptr())
            as *const clap_plugin_audio_ports;
        if ports.is_null() {
            return Err(Error::Ports);
        }
        for input in [true, false] {
            if (*ports).count.ok_or(Error::Ports)?(this.plugin, input) != 1 {
                return Err(Error::Ports);
            }
            let mut info: clap_audio_port_info = core::mem::zeroed();
            if !(*ports).get.ok_or(Error::Ports)?(this.plugin, 0, input, &mut info)
                || info.channel_count != 2
            {
                return Err(Error::Ports);
            }
        }
        if !p.activate.ok_or(Error::Lifecycle)?(this.plugin, RATE as f64, 1, BLOCK as u32) {
            return Err(Error::Lifecycle);
        }
        this.active = true;
        if !p.start_processing.ok_or(Error::Lifecycle)?(this.plugin) {
            return Err(Error::Lifecycle);
        }
        this.processing = true;
        Ok(this)
    }
    /// Offline effect render: buffers and output are bounded; no allocation occurs
    /// inside a process call. Native processing must still be supervised by timeout.
    pub fn render(&mut self, input: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, Error> {
        if input.is_empty() || input.len() > MAX_FRAMES as usize {
            return Err(Error::Frames);
        }
        if input
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 16.0)
        {
            return Err(Error::Audio);
        }
        let mut result = Vec::with_capacity(input.len());
        let mut ins = [[0f32; BLOCK]; 2];
        let mut outs = [[0f32; BLOCK]; 2];
        let incoming = clap_input_events {
            ctx: ptr::null_mut(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let outgoing = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(output),
        };
        for (block, frames) in input.chunks(BLOCK).enumerate() {
            if self.context.restart.load(Ordering::Acquire) {
                return Err(Error::Restart);
            }
            if self.context.callback.swap(false, Ordering::AcqRel) {
                unsafe {
                    if let Some(f) = (*self.plugin).on_main_thread {
                        f(self.plugin)
                    }
                }
            }
            if self.context.restart.load(Ordering::Acquire) {
                return Err(Error::Restart);
            }
            for (i, f) in frames.iter().enumerate() {
                ins[0][i] = f[0];
                ins[1][i] = f[1];
                outs[0][i] = f32::NAN;
                outs[1][i] = f32::NAN;
            }
            let mut ip = [ins[0].as_mut_ptr(), ins[1].as_mut_ptr()];
            let mut op = [outs[0].as_mut_ptr(), outs[1].as_mut_ptr()];
            let input = clap_audio_buffer {
                data32: ip.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let mut output = clap_audio_buffer {
                data32: op.as_mut_ptr(),
                ..input
            };
            let process = clap_process {
                steady_time: (block * BLOCK) as i64,
                frames_count: frames.len() as u32,
                transport: ptr::null(),
                audio_inputs: &input,
                audio_outputs: &mut output,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &incoming,
                out_events: &outgoing,
            };
            let status =
                unsafe { (*self.plugin).process.ok_or(Error::Lifecycle)?(self.plugin, &process) };
            if self.context.restart.load(Ordering::Acquire) {
                return Err(Error::Restart);
            }
            if !(CLAP_PROCESS_CONTINUE..=CLAP_PROCESS_SLEEP).contains(&status) {
                return Err(Error::Process);
            }
            for i in 0..frames.len() {
                let frame = [outs[0][i], outs[1][i]];
                if frame.iter().any(|v| !v.is_finite() || v.abs() > 16.0) {
                    return Err(Error::Audio);
                }
                result.push(frame);
            }
        }
        if self.context.restart.load(Ordering::Acquire) {
            return Err(Error::Restart);
        }
        Ok(result)
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        unsafe {
            if !self.plugin.is_null() {
                let p = &*self.plugin;
                if self.processing {
                    if let Some(f) = p.stop_processing {
                        f(self.plugin)
                    }
                }
                if self.active {
                    if let Some(f) = p.deactivate {
                        f(self.plugin)
                    }
                }
                if let Some(f) = p.destroy {
                    f(self.plugin)
                }
            }
            if self.initialized {
                if let Some(f) = (*self.entry).deinit {
                    f()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicUsize;
    struct Calls {
        host: *const clap_host,
        calls: AtomicUsize,
        restart_at: usize,
    }
    unsafe extern "C" fn process(plugin: *const clap_plugin, p: *const clap_process) -> i32 {
        let state = &*((*plugin).plugin_data.cast::<Calls>());
        let call = state.calls.fetch_add(1, Ordering::Relaxed) + 1;
        for channel in 0..2 {
            for i in 0..(*p).frames_count as usize {
                let src = *(*(*p).audio_inputs).data32.add(channel);
                let dst = *(*(*p).audio_outputs).data32.add(channel);
                *dst.add(i) = *src.add(i);
            }
        }
        if call == state.restart_at {
            ((*state.host).request_restart.unwrap())(state.host)
        }
        CLAP_PROCESS_CONTINUE
    }
    unsafe extern "C" fn main_thread(plugin: *const clap_plugin) {
        let state = &*((*plugin).plugin_data.cast::<Calls>());
        ((*state.host).request_restart.unwrap())(state.host);
    }
    fn run(
        frames: usize,
        restart_at: usize,
        main_callback: bool,
    ) -> (Result<Vec<[f32; 2]>, Error>, usize) {
        let mut context = Box::new(Context {
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(main_callback),
        });
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *context as *mut Context).cast(),
            name: c"test".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(restart),
            request_process: Some(request_process),
            request_callback: Some(callback),
        });
        let calls = Calls {
            host: &*host,
            calls: AtomicUsize::new(0),
            restart_at,
        };
        let plugin = clap_plugin {
            desc: ptr::null(),
            plugin_data: (&calls as *const Calls).cast_mut().cast(),
            init: None,
            destroy: None,
            activate: None,
            deactivate: None,
            start_processing: None,
            stop_processing: None,
            reset: None,
            process: Some(process),
            get_extension: None,
            on_main_thread: Some(main_thread),
        };
        let mut renderer = Processor {
            entry: ptr::null(),
            plugin: &plugin,
            _host: host,
            context,
            initialized: false,
            active: false,
            processing: false,
        };
        let result = renderer.render(&alloc::vec![[0.25,-0.5];frames]);
        let n = calls.calls.load(Ordering::Relaxed);
        (result, n)
    }
    #[test]
    fn one_block_restart_never_publishes() {
        let (r, n) = run(1, 1, false);
        assert_eq!(n, 1);
        assert_eq!(r, Err(Error::Restart));
    }
    #[test]
    fn final_block_restart_never_publishes() {
        let (r, n) = run(BLOCK + 1, 2, false);
        assert_eq!(n, 2);
        assert_eq!(r, Err(Error::Restart));
    }
    #[test]
    fn main_callback_restart_prevents_processing() {
        let (r, n) = run(1, usize::MAX, true);
        assert_eq!(n, 0);
        assert_eq!(r, Err(Error::Restart));
    }
    #[test]
    fn no_restart_control_processes_every_frame() {
        let (r, n) = run(BLOCK + 1, usize::MAX, false);
        assert_eq!(n, 2);
        assert_eq!(r.unwrap(), alloc::vec![[0.25,-0.5];BLOCK+1]);
    }
}
