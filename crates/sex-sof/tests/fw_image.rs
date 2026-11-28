// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Firmware image checks: a synthetic image with the structure of `sof-tgl-h.ri`,
//! malformed variants, and (when `SOF_TGL_IMAGE` names a file) the real image.

#[path = "fw_builder.rs"]
mod fw_builder;

use sex_sof::fw::{self, Error, Version};
use sex_sof::regs::Region;

#[test]
fn synthetic_image_is_described() {
    let img = fw_builder::image();
    let info = fw::check(&img).expect("valid");
    assert_eq!(info.payload_offset, fw_builder::EXT_LEN);
    assert_eq!(info.payload_len, fw_builder::PAYLOAD_LEN);
    assert_eq!(info.payload(&img), &fw_builder::payload()[..]);
    let sof = info.sof.expect("version element");
    assert_eq!(sof.version, Version { major: 2, minor: 2, micro: 0, build: 0xffff });
    assert_eq!(sof.abi, 0x0301_6001);
    assert_eq!(&sof.tag, b"abcdef");
    let m = info.mailboxes.expect("window element");
    assert_eq!(m.host, Region { offset: fw_builder::HOST_BOX.0, size: fw_builder::HOST_BOX.1 });
    assert_eq!(m.dsp, Region { offset: fw_builder::DSP_BOX.0, size: fw_builder::DSP_BOX.1 });
    assert_eq!(info.fw_version, Version { major: 2, minor: 2, micro: 0, build: 1 });
    assert_eq!(info.modules, 2);
    assert_eq!(info.preload_pages, 3);
    assert_eq!(info.partition_entries, 2);
}

#[test]
fn payload_without_extended_manifest_is_accepted() {
    let p = fw_builder::payload();
    let info = fw::check(&p).expect("valid");
    assert_eq!(info.payload_offset, 0);
    assert_eq!(info.sof, None);
    assert_eq!(info.mailboxes, None);
}

#[test]
fn garbage_is_rejected() {
    assert_eq!(fw::check(&[]), Err(Error::Truncated));
    assert_eq!(fw::check(&[0u8; 3]), Err(Error::Truncated));
    assert_eq!(fw::check(&[0u8; 0x4000]), Err(Error::NoCodePartition));
    assert_eq!(fw::check(&[0u8; 0x100]), Err(Error::BadPayloadSize));
}

#[test]
fn bad_extended_manifest_is_rejected() {
    let mut img = fw_builder::image();
    let too_big = img.len() as u32 + 1;
    img[4..8].copy_from_slice(&too_big.to_le_bytes());
    assert_eq!(fw::check(&img), Err(Error::BadExtManifest));

    let mut img = fw_builder::image();
    img[12..16].copy_from_slice(&0x0200_0000u32.to_le_bytes());
    assert_eq!(fw::check(&img), Err(Error::ExtManifestVersion(0x0200_0000)));

    let mut img = fw_builder::image();
    img[16 + 4..16 + 8].copy_from_slice(&4u32.to_le_bytes());
    assert_eq!(fw::check(&img), Err(Error::BadExtElement(16)));
}

#[test]
fn bad_partition_is_rejected() {
    let e = fw_builder::EXT_LEN;
    let mut img = fw_builder::image();
    img[e + 4..e + 8].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(fw::check(&img), Err(Error::BadCodePartition));

    let mut img = fw_builder::image();
    let entry_len = e + 0x14 + 24 + 16;
    img[entry_len..entry_len + 4].copy_from_slice(&0x10_0000u32.to_le_bytes());
    assert_eq!(fw::check(&img), Err(Error::BadCodePartition));

    let mut img = fw_builder::image();
    img[e + 0x100 + 28] = b'X';
    assert_eq!(fw::check(&img), Err(Error::NoSignedManifest));

    let mut img = fw_builder::image();
    img[e + 0x2000] = b'X';
    assert_eq!(fw::check(&img), Err(Error::NoFirmwareDescriptor));

    let mut img = fw_builder::image();
    img.push(0);
    assert_eq!(fw::check(&img), Err(Error::BadPayloadSize));
}

/// `SOF_TGL_IMAGE=/path/to/sof-tgl-h.ri cargo test -p sex-sof` checks the real one.
#[test]
fn real_image_when_given() {
    let Ok(path) = std::env::var("SOF_TGL_IMAGE") else { return };
    let img = std::fs::read(&path).expect("read SOF_TGL_IMAGE");
    let info = fw::check(&img).expect("real image is valid");
    eprintln!("{path}: {info:?}");
    assert!(info.payload_offset > 0, "sof-bin images carry an extended manifest");
    assert_eq!(info.payload_offset + info.payload_len, img.len());
    assert!(info.sof.is_some());
    let m = info.mailboxes.expect("windows");
    assert_eq!(m.dsp, sex_sof::regs::dsp::DEFAULT_DSP_BOX);
    assert_eq!(m.host, sex_sof::regs::dsp::DEFAULT_HOST_BOX);
    assert!(info.modules > 0);
}
