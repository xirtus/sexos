// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
use sex_trust_config::*;

#[test]
fn test_profile_constants() {
    assert_eq!(PROFILE, 1);
    assert_eq!(VERSION, 1);
    assert_eq!(EXPECTED_NAME, "probe.sexos.test");
    assert!(!SCOPE.is_empty());
}

#[test]
fn test_root_der_structure() {
    assert!(!ROOT_DER.is_empty());
    // ASN.1 DER certificate begins with 0x30 (SEQUENCE tag)
    assert_eq!(ROOT_DER[0], 0x30);

    // Two-byte length format: 0x82 followed by 2 length bytes
    assert_eq!(ROOT_DER[1], 0x82);
    let der_len = ((ROOT_DER[2] as usize) << 8) | (ROOT_DER[3] as usize);
    assert_eq!(ROOT_DER.len(), der_len + 4);

    // The DER certificate subject should contain the expected name / lab root string
    let root_str = String::from_utf8_lossy(ROOT_DER);
    assert!(root_str.contains("SexOS PUBLIC controlled TLS lab root"));
}
