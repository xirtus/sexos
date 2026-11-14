// SPDX-License-Identifier: GPL-3.0-only
//! USB Video Class (UVC 1.1 / 1.5) Descriptor Parser.
//!
//! Strictly bounds-checked, `#![no_std]`, `#![forbid(unsafe_code)]`.
//! Parses Interface Association Descriptors (IAD), VideoControl interfaces,
//! Camera Terminals, Processing Units, VideoStreaming headers, formats
//! (YUY2, NV12, MJPEG), frames, and endpoints.

use alloc::vec::Vec;

pub const DESC_INTERFACE: u8 = 4;
pub const DESC_ENDPOINT: u8 = 5;
pub const DESC_INTERFACE_ASSOCIATION: u8 = 11;
pub const DESC_CS_INTERFACE: u8 = 0x24;
pub const DESC_CS_ENDPOINT: u8 = 0x25;

pub const CC_VIDEO: u8 = 0x0E;
pub const SC_UNDEFINED: u8 = 0x00;
pub const SC_VIDEOCONTROL: u8 = 0x01;
pub const SC_VIDEOSTREAMING: u8 = 0x02;
pub const SC_VIDEO_INTERFACE_COLLECTION: u8 = 0x03;
pub const PC_PROTOCOL_UNDEFINED: u8 = 0x00;
pub const PC_PROTOCOL_15: u8 = 0x01;

pub const VC_HEADER: u8 = 0x01;
pub const VC_INPUT_TERMINAL: u8 = 0x02;
pub const VC_OUTPUT_TERMINAL: u8 = 0x03;
pub const VC_SELECTOR_UNIT: u8 = 0x04;
pub const VC_PROCESSING_UNIT: u8 = 0x05;
pub const VC_EXTENSION_UNIT: u8 = 0x06;

pub const VS_INPUT_HEADER: u8 = 0x01;
pub const VS_OUTPUT_HEADER: u8 = 0x02;
pub const VS_STILL_IMAGE_FRAME: u8 = 0x03;
pub const VS_FORMAT_UNCOMPRESSED: u8 = 0x04;
pub const VS_FRAME_UNCOMPRESSED: u8 = 0x05;
pub const VS_FORMAT_MJPEG: u8 = 0x06;
pub const VS_FRAME_MJPEG: u8 = 0x07;
pub const VS_COLORFORMAT: u8 = 0x0D;

pub const ITT_CAMERA: u16 = 0x0201;
pub const TT_STREAMING: u16 = 0x0101;

pub const GUID_YUY2: [u8; 16] = [
    0x59, 0x55, 0x59, 0x32, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
pub const GUID_NV12: [u8; 16] = [
    0x4E, 0x56, 0x31, 0x32, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorError {
    Truncated,
    InvalidLength,
    InvalidHeader,
    MissingVideoControl,
    MissingVideoStreaming,
    MissingFormat,
    MalformedGuid,
    MalformedFrame,
    TooManyEntries,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatKind {
    Yuy2,
    Nv12,
    Mjpeg,
    Unknown([u8; 16]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceAssociationDescriptor {
    pub first_interface: u8,
    pub interface_count: u8,
    pub function_class: u8,
    pub function_subclass: u8,
    pub function_protocol: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraTerminal {
    pub terminal_id: u8,
    pub terminal_type: u16,
    pub objective_focal_length_min: u16,
    pub objective_focal_length_max: u16,
    pub ocular_focal_length: u16,
    pub controls: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessingUnit {
    pub unit_id: u8,
    pub source_id: u8,
    pub max_multiplier: u16,
    pub controls: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputTerminal {
    pub terminal_id: u8,
    pub terminal_type: u16,
    pub source_id: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlInterface {
    pub interface_number: u8,
    pub uvc_version: u16,
    pub clock_frequency: u32,
    pub camera_terminals: Vec<CameraTerminal>,
    pub processing_units: Vec<ProcessingUnit>,
    pub output_terminals: Vec<OutputTerminal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDescriptor {
    pub frame_index: u8,
    pub capabilities: u8,
    pub width: u16,
    pub height: u16,
    pub min_bit_rate: u32,
    pub max_bit_rate: u32,
    pub max_frame_buffer_size: u32,
    pub default_frame_interval: u32,
    pub frame_interval_type: u8,
    pub intervals: Vec<u32>,
    pub min_interval: u32,
    pub max_interval: u32,
    pub step_interval: u32,
}

impl FrameDescriptor {
    pub fn default_fps(&self) -> u32 {
        if self.default_frame_interval == 0 {
            0
        } else {
            10_000_000 / self.default_frame_interval
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatDescriptor {
    pub format_index: u8,
    pub kind: FormatKind,
    pub bits_per_pixel: u8,
    pub default_frame_index: u8,
    pub frames: Vec<FrameDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl EndpointDescriptor {
    pub fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    pub fn is_bulk(&self) -> bool {
        (self.attributes & 0x03) == 0x02
    }

    pub fn is_isoc(&self) -> bool {
        (self.attributes & 0x03) == 0x01
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingInterface {
    pub interface_number: u8,
    pub endpoint_address: u8,
    pub formats: Vec<FormatDescriptor>,
    pub still_frames: Vec<(u16, u16)>,
    pub endpoints: Vec<EndpointDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvcDeviceDescriptors {
    pub iad: Option<InterfaceAssociationDescriptor>,
    pub control: ControlInterface,
    pub streaming: Vec<StreamingInterface>,
}

pub fn parse_descriptors(bytes: &[u8]) -> Result<UvcDeviceDescriptors, DescriptorError> {
    let mut offset = 0;
    let mut iad = None;
    let mut control = None;
    let mut streaming = Vec::new();

    let mut current_iface_num = 0u8;
    let mut current_iface_subclass = 0u8;
    let mut current_format: Option<FormatDescriptor> = None;
    let mut current_streaming: Option<StreamingInterface> = None;

    let mut iterations = 0;
    while offset < bytes.len() {
        iterations += 1;
        if iterations > 2048 {
            return Err(DescriptorError::TooManyEntries);
        }

        if offset + 2 > bytes.len() {
            return Err(DescriptorError::Truncated);
        }
        let length = bytes[offset] as usize;
        if length < 2 || offset + length > bytes.len() {
            return Err(DescriptorError::InvalidLength);
        }
        let desc_type = bytes[offset + 1];
        let desc_data = &bytes[offset..offset + length];

        match desc_type {
            DESC_INTERFACE_ASSOCIATION => {
                if length < 8 {
                    return Err(DescriptorError::Truncated);
                }
                iad = Some(InterfaceAssociationDescriptor {
                    first_interface: desc_data[2],
                    interface_count: desc_data[3],
                    function_class: desc_data[4],
                    function_subclass: desc_data[5],
                    function_protocol: desc_data[6],
                });
            }
            DESC_INTERFACE => {
                if length < 9 {
                    return Err(DescriptorError::Truncated);
                }
                // If we were parsing a streaming interface, finish it
                if let Some(mut stream) = current_streaming.take() {
                    if let Some(fmt) = current_format.take() {
                        stream.formats.push(fmt);
                    }
                    streaming.push(stream);
                }

                current_iface_num = desc_data[2];
                let subclass = desc_data[6];
                current_iface_subclass = subclass;

                if subclass == SC_VIDEOSTREAMING {
                    current_streaming = Some(StreamingInterface {
                        interface_number: current_iface_num,
                        endpoint_address: 0,
                        formats: Vec::new(),
                        still_frames: Vec::new(),
                        endpoints: Vec::new(),
                    });
                }
            }
            DESC_CS_INTERFACE => {
                if length < 3 {
                    return Err(DescriptorError::Truncated);
                }
                let subtype = desc_data[2];

                if current_iface_subclass == SC_VIDEOCONTROL {
                    match subtype {
                        VC_HEADER => {
                            if length < 12 {
                                return Err(DescriptorError::Truncated);
                            }
                            let uvc_version = u16::from_le_bytes([desc_data[3], desc_data[4]]);
                            let clock_frequency = u32::from_le_bytes([
                                desc_data[7],
                                desc_data[8],
                                desc_data[9],
                                desc_data[10],
                            ]);
                            control = Some(ControlInterface {
                                interface_number: current_iface_num,
                                uvc_version,
                                clock_frequency,
                                camera_terminals: Vec::new(),
                                processing_units: Vec::new(),
                                output_terminals: Vec::new(),
                            });
                        }
                        VC_INPUT_TERMINAL => {
                            if length < 8 {
                                return Err(DescriptorError::Truncated);
                            }
                            let terminal_id = desc_data[3];
                            let terminal_type = u16::from_le_bytes([desc_data[4], desc_data[5]]);
                            if terminal_type == ITT_CAMERA && length >= 15 {
                                let min_focal = u16::from_le_bytes([desc_data[8], desc_data[9]]);
                                let max_focal = u16::from_le_bytes([desc_data[10], desc_data[11]]);
                                let ocular_focal = u16::from_le_bytes([desc_data[12], desc_data[13]]);
                                let ctrl_size = desc_data[14] as usize;
                                let ctrl_end = 15 + ctrl_size;
                                if ctrl_end > length {
                                    return Err(DescriptorError::Truncated);
                                }
                                let controls = desc_data[15..ctrl_end].to_vec();
                                if let Some(ctrl) = control.as_mut() {
                                    ctrl.camera_terminals.push(CameraTerminal {
                                        terminal_id,
                                        terminal_type,
                                        objective_focal_length_min: min_focal,
                                        objective_focal_length_max: max_focal,
                                        ocular_focal_length: ocular_focal,
                                        controls,
                                    });
                                }
                            }
                        }
                        VC_PROCESSING_UNIT => {
                            if length < 10 {
                                return Err(DescriptorError::Truncated);
                            }
                            let unit_id = desc_data[3];
                            let source_id = desc_data[4];
                            let max_mult = u16::from_le_bytes([desc_data[5], desc_data[6]]);
                            let ctrl_size = desc_data[7] as usize;
                            let ctrl_end = 8 + ctrl_size;
                            if ctrl_end > length {
                                return Err(DescriptorError::Truncated);
                            }
                            let controls = desc_data[8..ctrl_end].to_vec();
                            if let Some(ctrl) = control.as_mut() {
                                ctrl.processing_units.push(ProcessingUnit {
                                    unit_id,
                                    source_id,
                                    max_multiplier: max_mult,
                                    controls,
                                });
                            }
                        }
                        VC_OUTPUT_TERMINAL => {
                            if length < 9 {
                                return Err(DescriptorError::Truncated);
                            }
                            let terminal_id = desc_data[3];
                            let terminal_type = u16::from_le_bytes([desc_data[4], desc_data[5]]);
                            let source_id = desc_data[7];
                            if let Some(ctrl) = control.as_mut() {
                                ctrl.output_terminals.push(OutputTerminal {
                                    terminal_id,
                                    terminal_type,
                                    source_id,
                                });
                            }
                        }
                        _ => {}
                    }
                } else if current_iface_subclass == SC_VIDEOSTREAMING {
                    match subtype {
                        VS_INPUT_HEADER => {
                            if length < 13 {
                                return Err(DescriptorError::Truncated);
                            }
                            let ep_addr = desc_data[6];
                            if let Some(stream) = current_streaming.as_mut() {
                                stream.endpoint_address = ep_addr;
                            }
                        }
                        VS_FORMAT_UNCOMPRESSED => {
                            if length < 27 {
                                return Err(DescriptorError::Truncated);
                            }
                            if let Some(fmt) = current_format.take() {
                                if let Some(stream) = current_streaming.as_mut() {
                                    stream.formats.push(fmt);
                                }
                            }
                            let format_index = desc_data[3];
                            let guid: [u8; 16] = desc_data[5..21].try_into().map_err(|_| DescriptorError::MalformedGuid)?;
                            let kind = if guid == GUID_YUY2 {
                                FormatKind::Yuy2
                            } else if guid == GUID_NV12 {
                                FormatKind::Nv12
                            } else {
                                FormatKind::Unknown(guid)
                            };
                            let bits_per_pixel = desc_data[21];
                            let default_frame_index = desc_data[22];
                            current_format = Some(FormatDescriptor {
                                format_index,
                                kind,
                                bits_per_pixel,
                                default_frame_index,
                                frames: Vec::new(),
                            });
                        }
                        VS_FRAME_UNCOMPRESSED | VS_FRAME_MJPEG => {
                            if length < 26 {
                                return Err(DescriptorError::Truncated);
                            }
                            let frame_index = desc_data[3];
                            let capabilities = desc_data[4];
                            let width = u16::from_le_bytes([desc_data[5], desc_data[6]]);
                            let height = u16::from_le_bytes([desc_data[7], desc_data[8]]);
                            let min_bit_rate = u32::from_le_bytes([desc_data[9], desc_data[10], desc_data[11], desc_data[12]]);
                            let max_bit_rate = u32::from_le_bytes([desc_data[13], desc_data[14], desc_data[15], desc_data[16]]);
                            let max_frame_buffer_size = u32::from_le_bytes([desc_data[17], desc_data[18], desc_data[19], desc_data[20]]);
                            let default_frame_interval = u32::from_le_bytes([desc_data[21], desc_data[22], desc_data[23], desc_data[24]]);
                            let frame_interval_type = desc_data[25];

                            let mut intervals = Vec::new();
                            let mut min_interval = 0;
                            let mut max_interval = 0;
                            let mut step_interval = 0;

                            if frame_interval_type == 0 {
                                // Continuous
                                if length < 38 {
                                    return Err(DescriptorError::Truncated);
                                }
                                min_interval = u32::from_le_bytes([desc_data[26], desc_data[27], desc_data[28], desc_data[29]]);
                                max_interval = u32::from_le_bytes([desc_data[30], desc_data[31], desc_data[32], desc_data[33]]);
                                step_interval = u32::from_le_bytes([desc_data[34], desc_data[35], desc_data[36], desc_data[37]]);
                            } else {
                                let num_intervals = frame_interval_type as usize;
                                let needed = 26 + num_intervals * 4;
                                if length < needed {
                                    return Err(DescriptorError::Truncated);
                                }
                                for i in 0..num_intervals {
                                    let idx = 26 + i * 4;
                                    let iv = u32::from_le_bytes([
                                        desc_data[idx],
                                        desc_data[idx + 1],
                                        desc_data[idx + 2],
                                        desc_data[idx + 3],
                                    ]);
                                    intervals.push(iv);
                                }
                            }

                            if let Some(fmt) = current_format.as_mut() {
                                fmt.frames.push(FrameDescriptor {
                                    frame_index,
                                    capabilities,
                                    width,
                                    height,
                                    min_bit_rate,
                                    max_bit_rate,
                                    max_frame_buffer_size,
                                    default_frame_interval,
                                    frame_interval_type,
                                    intervals,
                                    min_interval,
                                    max_interval,
                                    step_interval,
                                });
                            }
                        }
                        VS_FORMAT_MJPEG => {
                            if length < 11 {
                                return Err(DescriptorError::Truncated);
                            }
                            if let Some(fmt) = current_format.take() {
                                if let Some(stream) = current_streaming.as_mut() {
                                    stream.formats.push(fmt);
                                }
                            }
                            let format_index = desc_data[3];
                            let default_frame_index = desc_data[5];
                            current_format = Some(FormatDescriptor {
                                format_index,
                                kind: FormatKind::Mjpeg,
                                bits_per_pixel: 0,
                                default_frame_index,
                                frames: Vec::new(),
                            });
                        }
                        VS_STILL_IMAGE_FRAME => {
                            if length < 6 {
                                return Err(DescriptorError::Truncated);
                            }
                            let num_patterns = desc_data[4] as usize;
                            let needed = 5 + num_patterns * 4;
                            if length < needed {
                                return Err(DescriptorError::Truncated);
                            }
                            if let Some(stream) = current_streaming.as_mut() {
                                for i in 0..num_patterns {
                                    let idx = 5 + i * 4;
                                    let w = u16::from_le_bytes([desc_data[idx], desc_data[idx + 1]]);
                                    let h = u16::from_le_bytes([desc_data[idx + 2], desc_data[idx + 3]]);
                                    stream.still_frames.push((w, h));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            DESC_ENDPOINT => {
                if length < 7 {
                    return Err(DescriptorError::Truncated);
                }
                let address = desc_data[2];
                let attributes = desc_data[3];
                let max_packet_size = u16::from_le_bytes([desc_data[4], desc_data[5]]);
                let interval = desc_data[6];

                if let Some(stream) = current_streaming.as_mut() {
                    stream.endpoints.push(EndpointDescriptor {
                        address,
                        attributes,
                        max_packet_size,
                        interval,
                    });
                }
            }
            _ => {}
        }

        offset += length;
    }

    if let Some(mut stream) = current_streaming.take() {
        if let Some(fmt) = current_format.take() {
            stream.formats.push(fmt);
        }
        streaming.push(stream);
    }

    let control = control.ok_or(DescriptorError::MissingVideoControl)?;
    if streaming.is_empty() {
        return Err(DescriptorError::MissingVideoStreaming);
    }
    for s in &streaming {
        if s.formats.is_empty() {
            return Err(DescriptorError::MissingFormat);
        }
        for f in &s.formats {
            if f.frames.is_empty() {
                return Err(DescriptorError::MalformedFrame);
            }
        }
        if s.endpoints.is_empty() {
            return Err(DescriptorError::Truncated);
        }
    }

    Ok(UvcDeviceDescriptors {
        iad,
        control,
        streaming,
    })
}
