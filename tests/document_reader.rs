// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure reader, shared text metrics and BOOT profile data. No Display,
//! pixels, input routing or network authority is established by this wrapper.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/network_config.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;

#[test]fn https_reader_labels_actual_lab_endpoint_and_erases_when_switching_sources(){
 let mut reader=document_reader::Reader::new();reader.resize(624,706);
 let f=reader.render_https(b"authenticated body\n\x1b\xff",b"/welcome");
 assert!(f.bytes[..f.len].starts_with(b"Controlled HTTPS / lab root"));
 assert!(f.bytes[..f.len].windows(b"probe.sexos.test:46080 /welcome".len()).any(|w|w==b"probe.sexos.test:46080 /welcome"));
 assert!(f.bytes[..f.len].windows(b"authenticated body".len()).any(|w|w==b"authenticated body"));
 assert!(!f.bytes[..f.len].contains(&0x1b));assert!(!f.bytes[..f.len].contains(&0xff));
 let plain=reader.render_path(b"plain",b"/");assert!(plain.bytes[..plain.len].starts_with(b"Plain HTTP / no TLS"));assert!(!plain.bytes[..plain.len].windows(5).any(|w|w==b"46080"));
}
