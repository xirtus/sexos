// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Kernel-mode-setting model for SexOS display drivers, kept out of the kernel.
//!
//! A display driver service (virtio-gpu in `sexdisplay`, Intel Gen12 in
//! `sexgpu-intel`, later NVIDIA) describes its hardware as connectors, CRTCs
//! and planes. A client asks for a new configuration with one atomic [`Commit`];
//! [`commit::check`] validates the whole request against the device and the
//! current state before any register is touched, so a commit either applies
//! completely or not at all. Page flips report a [`FlipEvent`] at vblank.
//!
//! Plain data and arithmetic only: no hardware access, no syscalls. The host
//! tests in `tests/` check it without a machine (`scripts/kms_host_gate.sh`).
#![no_std]
extern crate alloc;

pub mod commit;
pub mod edid;
pub mod layout;
pub mod mode;
pub mod model;
pub mod wire;

pub use commit::{check, Change, Commit, CommitError, FlipEvent};
pub use mode::Mode;
pub use model::{Connector, ConnectorKind, Crtc, Device, Format, Framebuffer, Plane, PlaneKind, Rect, State, Status};
