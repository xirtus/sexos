// SPDX-License-Identifier: GPL-3.0-only
//! The key-instruction scanner (crates/sex-pdx/src/program_code.rs) against a plain
//! byte-by-byte reference (kernel plan phase 7, isolation matrix).
//!
//! The loader and the JIT seal refuse code holding WRPKRU (0F 01 EF), XRSTOR
//! (0F AE /5, memory form) or XRSTORS (0F C7 /3, memory form) at any offset, fed a
//! page at a time. The scanner skips 8-byte words that hold no 0x0F, so this
//! checks, for random code split into random pieces, with refused instructions
//! planted at random offsets (word edges and piece edges included), that the first
//! refused instruction it reports is exactly the reference's.
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_program_code
use proptest::prelude::*;
use sexos_kernel_models::program_code::{check, refused, Scan};

/// The first offset i with code[i..i+3] a refused instruction.
fn reference(code: &[u8]) -> Option<usize> {
    (0..code.len().saturating_sub(2)).find(|&i| code[i] == 0x0F && refused(code[i + 1], code[i + 2]))
}

/// Encodings of refused and nearly refused instructions to plant.
fn planted() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(vec![0x0F, 0x01, 0xEF]),                         // WRPKRU
        (0u8..=255).prop_map(|m| vec![0x0F, 0xAE, m]),         // XRSTOR when /5 memory, else LFENCE etc.
        (0u8..=255).prop_map(|m| vec![0x0F, 0xC7, m]),         // XRSTORS when /3 memory
        Just(vec![0x0F, 0x01, 0xEE]),                         // RDPKRU: allowed
        Just(vec![0x0F, 0x0F, 0x01, 0xEF]),                   // overlapping 0x0F
        Just(vec![0x0F]),
    ]
}

fn code() -> impl Strategy<Value = Vec<u8>> {
    // Mostly bytes that are not 0x0F, so the word skip is exercised, with plants.
    let byte = prop_oneof![8 => 0x10u8..=0xFF, 1 => Just(0x0Fu8), 1 => 0u8..0x10];
    (prop::collection::vec(byte, 0..300), prop::collection::vec((any::<prop::sample::Index>(), planted()), 0..4))
        .prop_map(|(mut code, plants)| {
            for (at, bytes) in plants {
                let at = if code.is_empty() { 0 } else { at.index(code.len()) };
                for (k, b) in bytes.into_iter().enumerate() { code.insert((at + k).min(code.len()), b); }
            }
            code
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(2048), ..ProptestConfig::default() })]

    #[test]
    fn whole_slice_check_matches_reference(code in code()) {
        prop_assert_eq!(check(&code).err(), reference(&code));
    }

    #[test]
    fn any_split_into_pieces_matches_reference(code in code(), cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6)) {
        let mut cuts: Vec<usize> = cuts.iter().map(|c| c.index(code.len() + 1)).collect();
        cuts.push(0); cuts.push(code.len()); cuts.sort(); cuts.dedup();
        let mut scan = Scan::new();
        let mut found = None;
        for w in cuts.windows(2) {
            if let Some(at) = scan.feed(&code[w[0]..w[1]], false) { found = Some(at as usize); break; }
        }
        prop_assert_eq!(found, reference(&code));
    }

    #[test]
    fn primed_scan_sees_instructions_across_the_boundary(before in prop::collection::vec(any::<u8>(), 2), code in code()) {
        let joined: Vec<u8> = before.iter().chain(&code).copied().collect();
        let got = Scan::primed([before[0], before[1]]).feed(&code, true);
        prop_assert_eq!(got, reference(&joined).map(|i| i as i64 - 2));
    }

    #[test]
    fn every_offset_of_every_refused_instruction_is_caught(len in 3usize..64, at in any::<prop::sample::Index>(), which in 0usize..3, modrm in 0u8..0xC0) {
        let insn = match which { 0 => [0x0F, 0x01, 0xEF], 1 => [0x0F, 0xAE, (modrm & !0x38) | 5 << 3], _ => [0x0F, 0xC7, (modrm & !0x38) | 3 << 3] };
        let mut code = vec![0x90u8; len];
        let at = at.index(len - 2);
        code[at..at + 3].copy_from_slice(&insn);
        prop_assert_eq!(check(&code), Err(at));
    }
}

/// The register forms of 0F AE /5 (LFENCE) and 0F C7 /3 are not refused.
#[test]
fn register_forms_are_allowed() {
    for modrm in 0xC0..=0xFFu8 {
        assert!(check(&[0x0F, 0xAE, modrm]).is_ok());
        assert!(check(&[0x0F, 0xC7, modrm]).is_ok());
    }
    assert!(check(&[0x0F, 0xAE, 0xE8]).is_ok()); // LFENCE
}
