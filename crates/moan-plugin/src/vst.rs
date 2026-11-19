//! VST3 audio component host using generated upstream bindings. No editor, OS
//! module loader or controller GUI is assumed. Native calls are child-only.
use crate::job::{BLOCK, MAX_FRAMES, RATE};
use alloc::vec::Vec;
use core::{ffi::c_void, ptr};
use vst3::{Class, ComPtr, ComWrapper, Interface, Steinberg::Vst::*, Steinberg::*};
#[derive(Debug)]
pub enum Error {
    Memory,
    Factory,
    Component,
    Ports,
    Lifecycle,
    Process,
    Frames,
    Audio,
}
pub struct Processor {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    _factory: ComPtr<IPluginFactory>,
    initialized: bool,
    active: bool,
    processing: bool,
}
impl Processor {
    /// # Safety
    /// `factory` is the genuine VST3 GetPluginFactory export from a relocated module
    /// that outlives this object. Only invoke in a disposable isolated child run.
    pub unsafe fn open(
        factory: unsafe extern "system" fn() -> *mut IPluginFactory,
    ) -> Result<Self, Error> {
        let factory = ComPtr::from_raw(factory()).ok_or(Error::Factory)?;
        let count = factory.countClasses();
        if count <= 0 || count > 64 {
            return Err(Error::Factory);
        }
        let mut cid = None;
        for index in 0..count {
            let mut info: PClassInfo = core::mem::zeroed();
            if factory.getClassInfo(index, &mut info) != kResultOk {
                return Err(Error::Factory);
            }
            let category = info.category.map(|b| b as u8);
            let n = category
                .iter()
                .position(|b| *b == 0)
                .ok_or(Error::Factory)?;
            if &category[..n] == b"Audio Module Class" {
                cid = Some(info.cid);
                break;
            }
        }
        let cid = cid.ok_or(Error::Component)?;
        let mut raw: *mut c_void = ptr::null_mut();
        if factory.createInstance(cid.as_ptr(), IComponent::IID.as_ptr().cast(), &mut raw)
            != kResultOk
        {
            return Err(Error::Component);
        }
        let component = ComPtr::from_raw(raw.cast::<IComponent>()).ok_or(Error::Component)?;
        let processor = component
            .cast::<IAudioProcessor>()
            .ok_or(Error::Component)?;
        let mut this = Self {
            component,
            processor,
            _factory: factory,
            initialized: false,
            active: false,
            processing: false,
        };
        if this.component.initialize(ptr::null_mut()) != kResultOk {
            return Err(Error::Lifecycle);
        }
        this.initialized = true;
        for direction in [BusDirections_::kInput, BusDirections_::kOutput] {
            if this
                .component
                .getBusCount(MediaTypes_::kAudio as i32, direction as i32)
                != 1
            {
                return Err(Error::Ports);
            }
            let mut info: BusInfo = core::mem::zeroed();
            if this
                .component
                .getBusInfo(MediaTypes_::kAudio as i32, direction as i32, 0, &mut info)
                != kResultOk
                || info.channelCount != 2
            {
                return Err(Error::Ports);
            }
            if this
                .component
                .activateBus(MediaTypes_::kAudio as i32, direction as i32, 0, 1)
                != kResultOk
            {
                return Err(Error::Ports);
            }
        }
        if this
            .processor
            .canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32)
            != kResultOk
        {
            return Err(Error::Ports);
        }
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kOffline as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: BLOCK as i32,
            sampleRate: RATE as f64,
        };
        if this.processor.setupProcessing(&mut setup) != kResultOk
            || this.component.setActive(1) != kResultOk
        {
            return Err(Error::Lifecycle);
        }
        this.active = true;
        if this.processor.setProcessing(1) != kResultOk {
            return Err(Error::Lifecycle);
        }
        this.processing = true;
        Ok(this)
    }
    pub fn render(&mut self, input: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, Error> {
        self.render_with_parameters(input, &[])
    }
    pub fn render_with_parameters(
        &mut self,
        input: &[[f32; 2]],
        parameters: &[crate::job::Parameter],
    ) -> Result<Vec<[f32; 2]>, Error> {
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
        if parameters.len() > crate::job::MAX_PARAMS
            || parameters
                .iter()
                .any(|p| !p.value.is_finite() || !(0.0..=1.0).contains(&p.value))
        {
            return Err(Error::Audio);
        }
        let queues = parameters
            .iter()
            .map(|p| {
                ComWrapper::new(Queue(*p))
                    .to_com_ptr::<IParamValueQueue>()
                    .unwrap()
            })
            .collect();
        let changes = ComWrapper::new(Changes(queues))
            .to_com_ptr::<IParameterChanges>()
            .unwrap();
        let mut out = crate::job::buffer(input.len()).map_err(|_| Error::Memory)?;
        let mut ins = [[0f32; BLOCK]; 2];
        let mut outs = [[0f32; BLOCK]; 2];
        for (block, frames) in input.chunks(BLOCK).enumerate() {
            for (i, f) in frames.iter().enumerate() {
                ins[0][i] = f[0];
                ins[1][i] = f[1];
                outs[0][i] = f32::NAN;
                outs[1][i] = f32::NAN;
            }
            let mut ip = [ins[0].as_mut_ptr(), ins[1].as_mut_ptr()];
            let mut op = [outs[0].as_mut_ptr(), outs[1].as_mut_ptr()];
            let mut input = AudioBusBuffers {
                numChannels: 2,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 {
                    channelBuffers32: ip.as_mut_ptr(),
                },
            };
            let mut output = AudioBusBuffers {
                numChannels: 2,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 {
                    channelBuffers32: op.as_mut_ptr(),
                },
            };
            let mut data = ProcessData {
                processMode: ProcessModes_::kOffline as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                numSamples: frames.len() as i32,
                numInputs: 1,
                numOutputs: 1,
                inputs: &mut input,
                outputs: &mut output,
                inputParameterChanges: if block == 0 {
                    changes.as_ptr()
                } else {
                    ptr::null_mut()
                },
                outputParameterChanges: ptr::null_mut(),
                inputEvents: ptr::null_mut(),
                outputEvents: ptr::null_mut(),
                processContext: ptr::null_mut(),
            };
            if unsafe { self.processor.process(&mut data) } != kResultOk {
                return Err(Error::Process);
            }
            for i in 0..frames.len() {
                let frame = [outs[0][i], outs[1][i]];
                if frame.iter().any(|v| !v.is_finite() || v.abs() > 16.0) {
                    return Err(Error::Audio);
                }
                out.push(frame);
            }
        }
        Ok(out)
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        unsafe {
            if self.processing {
                let _ = self.processor.setProcessing(0);
            }
            if self.active {
                let _ = self.component.setActive(0);
            }
            if self.initialized {
                let _ = self.component.terminate();
            }
        }
    }
}

struct Queue(crate::job::Parameter);
impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}
impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> u32 {
        self.0.id
    }
    unsafe fn getPointCount(&self) -> i32 {
        1
    }
    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut f64) -> i32 {
        if index != 0 || offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        *offset = 0;
        *value = self.0.value;
        kResultOk
    }
    unsafe fn addPoint(&self, _: i32, _: f64, _: *mut i32) -> i32 {
        kNotImplemented as i32
    }
}
struct Changes(Vec<ComPtr<IParamValueQueue>>);
impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}
impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> i32 {
        self.0.len() as i32
    }
    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if index < 0 {
            return ptr::null_mut();
        }
        self.0
            .get(index as usize)
            .map_or(ptr::null_mut(), |p| p.as_ptr())
    }
    unsafe fn addParameterData(&self, _: *const u32, _: *mut i32) -> *mut IParamValueQueue {
        ptr::null_mut()
    }
}
