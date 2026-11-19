// SPDX-License-Identifier: GPL-3.0-only
//! Keyboard contents never belong in a normal image's serial diagnostics.
//! The explicit diagnostic build is unsuitable for passwords or private input.
pub const PAYLOAD_PROOF: bool = cfg!(sexos_input_payload_proof);
pub const fn mode() -> &'static str { if PAYLOAD_PROOF { "proof" } else { "redacted" } }

/// Keep a payload-free delivery receipt in normal builds. The full format and
/// its arguments are evaluated only in an explicitly declared diagnostic image.
#[macro_export]
macro_rules! serial_input_record {
    (($($public:tt)*); ($($private:tt)*)) => {{
        if $crate::input_diagnostics::PAYLOAD_PROOF {
            $crate::serial_record!($($private)*);
        } else {
            $crate::serial_record!($($public)*);
        }
    }};
}
