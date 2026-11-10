fn main() {
    release_key();
    println!("cargo:rerun-if-env-changed=SEXFILES_RAMFS_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_DISKFS_OBJECT_TABLE_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_JOURNAL_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_REPLAY_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_CAP_RECORD_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_LINEN_SEXFILES_METADATA_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_FAULT_INJECTION_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_REAL_BLOCK_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_REBOOT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_EXTENT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXFILES_CHECKPOINT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_SEXOBJECT_VIEW_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOS_LINEN_DISK_OBJECT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXFS_V0_SUPERBLOCK_FORMAT_MOUNT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOBJECT_TABLE_PERSIST_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOBJECT_TABLE_EXTENT_ALLOC_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOBJECT_EXTENT_WRITE_FULL_BLOCK_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOBJECT_WRITE_READ_PERSIST_PROOF");
    println!("cargo:rerun-if-env-changed=SEXOBJECT_MULTI_OBJECT_PROOF");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_PROOF");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap2_proof)");
    if std::env::var("SEXFILES_DISKFS_100_PROOF").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap2_proof");
    }
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP3_PROOF");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap3_proof)");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs_multi_object_proof)");
    if std::env::var("SEXFILES_DISKFS_100_AP3_PROOF").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap3_proof");
        println!("cargo:rustc-cfg=sexfiles_diskfs_multi_object_proof");
    }
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP4_WRITE");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP4_READ");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap4_write)");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap4_read)");
    if std::env::var("SEXFILES_DISKFS_100_AP4_WRITE").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap4_write");
    }
    if std::env::var("SEXFILES_DISKFS_100_AP4_READ").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap4_read");
    }
    // AP5 negative test lanes
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP5_NEGATIVE");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP5_NEG_MISMATCH");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP5_NEG_MISSING_IMAGE");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP5_NEG_READ_NO_WRITE");
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP5_NEG_FLUSH_SKIP");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap5_neg_mismatch)");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap5_neg_missing_image)");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap5_neg_read_no_write)");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap5_neg_flush_skip)");
    if std::env::var("SEXFILES_DISKFS_100_AP5_NEGATIVE").as_deref() == Ok("1") {
        if std::env::var("SEXFILES_DISKFS_100_AP5_NEG_MISMATCH").as_deref() == Ok("1") {
            println!("cargo:rustc-cfg=sexfiles_diskfs100_ap5_neg_mismatch");
        }
        if std::env::var("SEXFILES_DISKFS_100_AP5_NEG_MISSING_IMAGE").as_deref() == Ok("1") {
            println!("cargo:rustc-cfg=sexfiles_diskfs100_ap5_neg_missing_image");
        }
        if std::env::var("SEXFILES_DISKFS_100_AP5_NEG_READ_NO_WRITE").as_deref() == Ok("1") {
            println!("cargo:rustc-cfg=sexfiles_diskfs100_ap5_neg_read_no_write");
            // read-no-write reuses the AP4 read path: set both cfgs
            println!("cargo:rustc-cfg=sexfiles_diskfs100_ap4_read");
        }
    }
    if std::env::var("SEXFILES_DISKFS_100_AP5_NEG_FLUSH_SKIP").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap5_neg_flush_skip");
    }
    // AP6: honest classification of flush/fsync
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_100_AP6_FLUSH_FSYNC");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs100_ap6_flush_fsync)");
    if std::env::var("SEXFILES_DISKFS_100_AP6_FLUSH_FSYNC").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs100_ap6_flush_fsync");
    }
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_BRIDGE_STRICT_PROOF");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs_bridge_strict_proof)");
    if std::env::var("SEXFILES_DISKFS_BRIDGE_STRICT_PROOF").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs_bridge_strict_proof");
    }
    println!("cargo:rerun-if-env-changed=SEXFILES_DISKFS_NEGATIVE_BOUNDS_AUTH_PROOF");
    println!("cargo:rustc-check-cfg=cfg(sexfiles_diskfs_negative_bounds_auth_proof)");
    if std::env::var("SEXFILES_DISKFS_NEGATIVE_BOUNDS_AUTH_PROOF").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=sexfiles_diskfs_negative_bounds_auth_proof");
    }
}

/// The public key updates must be signed with: `SEXOS_RELEASE_KEY` (64 hex digits) when
/// set, else `keys/release.pub` at the top of the tree. Without either, `update` refuses
/// every update: a build nobody can sign for takes none.
fn release_key() {
    println!("cargo:rerun-if-env-changed=SEXOS_RELEASE_KEY");
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../keys/release.pub");
    println!("cargo:rerun-if-changed={}", file.display());
    let key = std::env::var("SEXOS_RELEASE_KEY").ok().or_else(|| std::fs::read_to_string(&file).ok()).unwrap_or_default();
    let key: String = key.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<String>().trim().to_string();
    if !key.is_empty() && (key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit())) {
        panic!("release key must be 64 hex digits (an Ed25519 public key), got {key:?}");
    }
    println!("cargo:rustc-env=SEXOS_RELEASE_KEY_HEX={key}");
}
