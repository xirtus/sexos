// SPDX-License-Identifier: GPL-3.0-only
//! Adversarial, malformed, truncated and 10,000+ mutation fuzz stress tests.

use sex_codecs::{decode, probe, sniff, AnimationDecoder, Error, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn truncated_inputs_never_panic() {
    let names = ["gradient_lossless.webp", "anim.webp", "still.gif", "anim.gif", "smpte32.avif", "smpte32.heic"];
    for name in names {
        let b = fixture(name);
        for cut in [0, 1, 2, 4, 7, 8, 11, 12, 15, 16, 24, 32, b.len() / 2, b.len().saturating_sub(1)] {
            let slice = &b[..cut.min(b.len())];
            let _ = sniff(slice);
            let _ = probe(slice);
            let _ = decode(slice, &Limits::PHOTO);
            if let Ok(mut anim) = AnimationDecoder::open(slice, &Limits::PHOTO) {
                let mut canvas = vec![0u32; anim.width() * anim.height()];
                let _ = anim.next_frame(&mut canvas);
            }
        }
    }
}

#[test]
fn tiny_limits_refuse_before_allocating() {
    let tiny = Limits {
        max_width: 4,
        max_height: 4,
        max_pixels: 16,
        max_input_bytes: 50,
    };
    let b = fixture("gradient_lossless.webp");
    assert_eq!(decode(&b, &tiny), Err(Error::InputTooLarge));

    let small_dims = Limits {
        max_width: 4,
        max_height: 4,
        max_pixels: 16,
        max_input_bytes: 1 << 20,
    };
    assert_eq!(decode(&b, &small_dims), Err(Error::TooLarge));
}

#[test]
fn fifty_thousand_mutations_stress_fuzz() {
    let fixtures = [
        fixture("gradient_lossless.webp"),
        fixture("quadrant.webp"),
        fixture("anim.webp"),
        fixture("still.gif"),
        fixture("quadrant.gif"),
        fixture("anim.gif"),
        fixture("quadrant.avif"),
        fixture("smpte32.avif"),
        fixture("quadrant.heic"),
        fixture("smpte32.heic"),
    ];

    let mut prng = 0x1337_c0de_u64;
    let mut next_u32 = || {
        // xorshift64*
        prng ^= prng >> 12;
        prng ^= prng << 25;
        prng ^= prng >> 27;
        (prng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    };

    const TOTAL_ITERATIONS: usize = 50_000;


    for i in 0..TOTAL_ITERATIONS {
        let base = &fixtures[i % fixtures.len()];
        let mut mutated = base.clone();

        // Apply 1 to 5 random mutations
        let num_mutations = (next_u32() % 5) + 1;
        for _ in 0..num_mutations {
            let mut_type = next_u32() % 4;
            let len = mutated.len();
            if len == 0 { continue; }
            let pos = (next_u32() as usize) % len;

            match mut_type {
                0 => {
                    // Bit flip
                    let bit = 1u8 << (next_u32() % 8);
                    mutated[pos] ^= bit;
                }
                1 => {
                    // Random byte
                    mutated[pos] = next_u32() as u8;
                }
                2 => {
                    // Word overwrite with boundary values (0, 0xFFFFFFFF, etc.)
                    if pos + 4 <= len {
                        let val = match next_u32() % 4 {
                            0 => 0u32,
                            1 => 0xFFFF_FFFF,
                            2 => 0x7FFF_FFFF,
                            _ => next_u32(),
                        };
                        mutated[pos..pos + 4].copy_from_slice(&val.to_be_bytes());
                    }
                }
                _ => {
                    // Truncation
                    let new_len = (next_u32() as usize) % len;
                    mutated.truncate(new_len);
                }
            }
        }

        // Must never panic or crash
        let _ = sniff(&mutated);
        let _ = probe(&mutated);
        let _ = decode(&mutated, &Limits::PHOTO);
        if let Ok(mut anim) = AnimationDecoder::open(&mutated, &Limits::PHOTO) {
            let mut canvas = vec![0u32; anim.width() * anim.height()];
            let _ = anim.next_frame(&mut canvas);
        }
    }
}
