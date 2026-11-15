// SPDX-License-Identifier: GPL-3.0-only
//! USB Video Class (UVC 1.1 / 1.5) Protocol, Frame Engine & Fixture.
//!
//! Pure-Rust `#![no_std]`, `#![forbid(unsafe_code)]`.
//! Provides descriptor parsing, probe/commit negotiation, packet stream
//! reassembly, and integer BT.601 color decoding (YUY2/NV12/MJPEG).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod control;
pub mod descriptors;
pub mod fixture;
pub mod photobooth;
pub mod pixel;
pub mod stream;

pub use control::VideoProbeCommitControl;
pub use descriptors::{parse_descriptors, FormatDescriptor, FormatKind, FrameDescriptor, UvcDeviceDescriptors};
pub use fixture::{SyntheticUvcDevice, WireMessage};
pub use photobooth::{apply_filter, compose_burst_contact_sheet, render_effects_grid, Filter};
pub use pixel::{decode_frame, nv12_to_argb, yuy2_to_argb};
pub use stream::{CompleteFrame, UvcPayloadHeader, UvcStreamAssembler};
