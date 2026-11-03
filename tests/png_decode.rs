// SPDX-License-Identifier: GPL-3.0-only
//! Host tests of the allocation-free PNG decoder against zlib-made fixtures.
#[path="../crates/sex-graphics/src/qoi.rs"] #[allow(dead_code)] mod qoi;
#[path="../crates/sex-graphics/src/png.rs"] mod png;
#[path="fixtures/png/cases.rs"] mod cases;
use png::{decode, Error, SCRATCH_BYTES};

fn run(bytes: &[u8]) -> Result<(usize, usize, Vec<u32>), Error> {
    let mut scratch = Box::new([0u8; SCRATCH_BYTES]);
    let mut out = Box::new([0u32; qoi::MAX_PIXELS]);
    let (w, h) = decode(bytes, &mut scratch, &mut out)?;
    Ok((w, h, out[..w * h].to_vec()))
}

#[test]
fn decodes_every_supported_color_type_filter_and_block_kind() {
    for (name, bytes, w, h, pixels) in cases::CASES {
        let (gw, gh, got) = run(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!((gw, gh), (*w, *h), "{name}");
        assert_eq!(&got[..], *pixels, "{name}");
    }
}

#[test]
fn any_single_byte_corruption_is_refused_or_harmless_and_never_panics() {
    for (name, bytes, ..) in cases::CASES {
        for i in 0..bytes.len() {
            let mut b = bytes.to_vec();
            b[i] ^= 0x41;
            // CRCs cover every chunk, so a flipped byte past the signature fails.
            if i >= 8 { assert!(run(&b).is_err(), "{name} byte {i}"); }
            let _ = run(&b);
        }
        for cut in 0..bytes.len() { assert!(run(&bytes[..cut]).is_err(), "{name} cut {cut}"); }
    }
}

#[test]
fn refuses_what_it_does_not_support() {
    let (_, good, ..) = cases::CASES[1];
    let mut b = good.to_vec();
    assert_eq!(run(b"GIF89a").err(), Some(Error::Signature));
    b[24] = 16; // bit depth 16 (CRC now wrong too, but header checks come after CRC)
    assert!(run(&b).is_err());
    let mut big = vec![0u8; qoi::MAX_INPUT_BYTES + 1];
    big[..8].copy_from_slice(&good[..8]);
    assert_eq!(run(&big).err(), Some(Error::InputTooLarge));
}
