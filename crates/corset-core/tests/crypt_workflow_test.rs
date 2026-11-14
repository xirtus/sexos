// SPDX-License-Identifier: GPL-3.0-only
//! Integration tests for BDSM Encrypted Volume workflow (Argon2id + XTS-AES-256).

use corset_core::crypt_flow::{
    erase_encrypted_header, format_encrypted_volume, rewrap_passphrase, unlock_encrypted_volume,
    Cost,
};
use corset_core::device::{BlockIo, MemBlockDevice};
use corset_core::error::CorsetError;

#[test]
fn test_encrypted_volume_lifecycle() {
    // 8 MiB device = 16384 sectors @ 512B
    let mut dev = MemBlockDevice::new_512(16384).unwrap();
    let salt = [0x77u8; 32];
    let passphrase = b"KinkySubmissivePassphrase123!";
    let test_cost = Cost {
        memory_kib: 64, // Fast cost for unit test
        passes: 1,
        lanes: 1,
    };

    // 1. Format encrypted volume
    let master_key = format_encrypted_volume(&mut dev, passphrase, &salt, None, Some(test_cost)).unwrap();
    assert_ne!(master_key, [0u8; 64]);

    // 2. Unlock with wrong passphrase should fail
    let wrong_pass = b"WrongPassword";
    let unlock_err = unlock_encrypted_volume(&mut dev, wrong_pass).unwrap_err();
    assert_eq!(unlock_err, CorsetError::CryptWrongPassword);

    // 3. Unlock with correct passphrase
    {
        let mut enc_io = unlock_encrypted_volume(&mut dev, passphrase).unwrap();
        assert_eq!(enc_io.sector_size(), 512);
        // Usable sectors = total (16384) - header sectors (4096 / 512 = 8) = 16376
        assert_eq!(enc_io.sector_count(), 16376);

        // Write a 4-sector plaintext pattern
        let mut plaintext = [0u8; 2048];
        for (i, b) in plaintext.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }

        enc_io.write_sectors(10, &plaintext).unwrap();
        enc_io.flush().unwrap();

        // Read back through enc_io: should match plaintext exactly!
        let mut read_buf = [0u8; 2048];
        enc_io.read_sectors(10, &mut read_buf).unwrap();
        assert_eq!(read_buf, plaintext);
    }

    // 4. Verify raw on-disk data at physical LBA (header 8 + 10 = 18) is ciphertext
    {
        let mut raw_buf = [0u8; 2048];
        dev.read_sectors(18, &mut raw_buf).unwrap();
        // Raw bytes should NOT equal plaintext
        let mut expected_plaintext = [0u8; 2048];
        for (i, b) in expected_plaintext.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        assert_ne!(raw_buf, expected_plaintext);
    }

    // 5. Change passphrase (rewrap)
    let new_salt = [0x88u8; 32];
    let new_passphrase = b"DominantNewSafePassword456!";
    rewrap_passphrase(&mut dev, passphrase, new_passphrase, &new_salt).unwrap();

    // 6. Old passphrase should now fail!
    let old_unlock_err = unlock_encrypted_volume(&mut dev, passphrase).unwrap_err();
    assert_eq!(old_unlock_err, CorsetError::CryptWrongPassword);

    // 7. New passphrase unlocks and plaintext is preserved!
    {
        let mut enc_io = unlock_encrypted_volume(&mut dev, new_passphrase).unwrap();
        let mut read_buf = [0u8; 2048];
        enc_io.read_sectors(10, &mut read_buf).unwrap();
        let mut expected_plaintext = [0u8; 2048];
        for (i, b) in expected_plaintext.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        assert_eq!(read_buf, expected_plaintext);
    }

    // 8. Cryptographic Header Erase
    erase_encrypted_header(&mut dev).unwrap();

    // After erase, unlocking fails with CryptNotEncrypted!
    let erased_err = unlock_encrypted_volume(&mut dev, new_passphrase).unwrap_err();
    assert_eq!(erased_err, CorsetError::CryptNotEncrypted);
}
