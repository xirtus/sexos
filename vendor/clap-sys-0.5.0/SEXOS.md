# Local no_std port

clap-sys0.5.0, MIT OR Apache-2.0; original notices retained. Published registry
source is the baseline. Only Rust source substitutions std:: -> core:: and
#![no_std] were made. No ABI layout, constants, function signatures or unsafe
implementation was invented or changed. This avoids pulling a Linux std loader
into a freestanding plugin helper.
