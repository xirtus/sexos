// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The ways a driver call fails. Every failure is a value; a driver never panics.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A bounded wait ran out (deadline or poll count) before the device answered.
    Timeout,
    /// A register read all ones: the device fell off the bus, was surprise
    /// removed, or is powered down.
    Gone,
    /// The device is not the one this driver drives (identity check failed).
    NotThisDevice,
    /// The device reported a value outside what the driver accepts (a ring index
    /// past the ring, a length past the buffer, a reserved encoding).
    BadValue,
    /// The caller asked for something outside the device's limits.
    BadRequest,
    /// The device reported an error of its own; `u32` is its status word.
    Device(u32),
    /// The system refused: no lease at that slot, no DMA pool, vector not ours.
    Denied,
    /// Not supported by this device or this driver yet.
    Unsupported,
}

impl Error {
    /// Short stable name, for record lines (`reason=timeout`).
    pub fn name(self) -> &'static str {
        match self {
            Error::Timeout => "timeout",
            Error::Gone => "gone",
            Error::NotThisDevice => "not_this_device",
            Error::BadValue => "bad_value",
            Error::BadRequest => "bad_request",
            Error::Device(_) => "device",
            Error::Denied => "denied",
            Error::Unsupported => "unsupported",
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Device(s) => write!(f, "device error {s:#x}"),
            e => f.write_str(e.name()),
        }
    }
}
