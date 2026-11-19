// SPDX-License-Identifier: GPL-3.0-only
use sex_crypt::{cost, header, rewrap, unlock, Cost, Error, Xts, HEADER, KEY};

#[test]
fn test_header_validation_and_corruption() {
    let fast_cost = Cost { memory_kib: 64, passes: 1, lanes: 1 };
    let salt = [0x55u8; 32];
    let master = [0xA5u8; KEY];
    let pwd = b"supersecretpassphrase";

    let h = header(pwd, &salt, &master, fast_cost).unwrap();
    assert_eq!(cost(&h).unwrap(), fast_cost);

    // Header too short
    assert_eq!(cost(&h[..HEADER - 1]), Err(Error::NotEncrypted));

    // Wrong magic
    let mut bad_magic = h;
    bad_magic[..8].copy_from_slice(b"BADMAGIC");
    assert_eq!(cost(&bad_magic), Err(Error::NotEncrypted));

    // Unsupported version
    let mut bad_ver = h;
    bad_ver[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(cost(&bad_ver), Err(Error::Corrupt));

    // Memory bound too low (< 8 KiB)
    let mut bad_mem_low = h;
    bad_mem_low[12..16].copy_from_slice(&4u32.to_le_bytes());
    assert_eq!(cost(&bad_mem_low), Err(Error::Corrupt));

    // Passes bound too high (> 16)
    let mut bad_passes = h;
    bad_passes[16..20].copy_from_slice(&20u32.to_le_bytes());
    assert_eq!(cost(&bad_passes), Err(Error::Corrupt));

    // Lanes bound zero
    let mut bad_lanes = h;
    bad_lanes[20..24].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(cost(&bad_lanes), Err(Error::Corrupt));

    // Tampered checksum in header (bytes 128..160)
    let mut bad_check = h;
    bad_check[130] ^= 0xFF;
    assert_eq!(unlock(&bad_check, pwd), Err(Error::WrongPassword));

    // Tampered ciphertext in header (bytes 64..128)
    let mut bad_cipher = h;
    bad_cipher[70] ^= 0xFF;
    assert_eq!(unlock(&bad_cipher, pwd), Err(Error::WrongPassword));
}

#[test]
fn test_xts_sector_isolation() {
    let key = [0x33u8; KEY];
    let xts = Xts::new(&key);

    let plain = vec![0x42u8; 512]; // one standard sector

    let mut cipher_unit0 = plain.clone();
    xts.encrypt(0, &mut cipher_unit0);

    let mut cipher_unit1 = plain.clone();
    xts.encrypt(1, &mut cipher_unit1);

    // Encrypting the same plaintext under different sector unit numbers must yield different ciphertexts
    assert_ne!(cipher_unit0, cipher_unit1);

    // Decrypting cipher_unit0 using unit 1 must produce corrupted data, not original plaintext
    let mut decrypt_wrong_unit = cipher_unit0.clone();
    xts.decrypt(1, &mut decrypt_wrong_unit);
    assert_ne!(decrypt_wrong_unit, plain);

    // Decrypting with correct unit must recover plaintext
    let mut decrypt_correct = cipher_unit0;
    xts.decrypt(0, &mut decrypt_correct);
    assert_eq!(decrypt_correct, plain);
}

#[test]
fn test_rewrap_and_key_preservation() {
    let fast_cost = Cost { memory_kib: 64, passes: 1, lanes: 1 };
    let salt1 = [1u8; 32];
    let salt2 = [2u8; 32];
    let master: [u8; KEY] = core::array::from_fn(|i| (i as u8).wrapping_mul(7));

    let pwd1 = b"initial password";
    let pwd2 = b"new rotated password";

    let h1 = header(pwd1, &salt1, &master, fast_cost).unwrap();
    assert_eq!(unlock(&h1, pwd1).unwrap(), master);

    // Rewrap with new password
    let h2 = rewrap(&h1, pwd1, pwd2, &salt2).unwrap();

    // Old password must fail on new header
    assert_eq!(unlock(&h2, pwd1), Err(Error::WrongPassword));

    // New password recovers identical master key
    let recovered = unlock(&h2, pwd2).unwrap();
    assert_eq!(recovered, master);
}
