//! Native tests of the production register selector decoders.
mod ipc {
    pub const MAX_DOMAINS: usize = 1024;
}

#[path = "../kernel/src/capability_slot.rs"]
mod capability_slot;

use capability_slot::{decode_capability_slot, decode_domain_id, decode_pdx_slot, valid_syscall_selectors};

#[test]
fn preserves_every_valid_capability_slot() {
    for slot in 1..=capability_slot::CAPABILITY_SLOT_COUNT {
        assert_eq!(decode_capability_slot(slot), Some(slot as u32));
        assert_eq!(decode_pdx_slot(slot), Some(slot as u32));
    }
}

#[test]
fn slot_zero_is_only_the_explicit_pdx_bridge() {
    assert_eq!(decode_pdx_slot(0), Some(0));
    assert_eq!(decode_capability_slot(0), None);
}

#[test]
fn rejects_high_bits_that_would_alias_a_valid_slot_or_bridge() {
    for high in [1u64 << 32, 2u64 << 32, 1u64 << 63] {
        for low in [0, 1, 5, 13, 1024] {
            let raw = high | low;
            assert_eq!(decode_capability_slot(raw), None, "capability {raw}");
            assert_eq!(decode_pdx_slot(raw), None, "PDX {raw}");
        }
    }
}

#[test]
fn rejects_invalid_lend_destination_forms() {
    // All these previously passed the occupancy check as absent. insert_at
    // then silently ignored 0/1025, or accepted a truncated high-bit alias.
    for raw in [0, 1025, u32::MAX as u64, 1u64 << 32, (1u64 << 32) | 13, u64::MAX] {
        assert_eq!(decode_capability_slot(raw), None, "destination {raw}");
    }
}

#[test]
fn syscall_preflight_denies_aliases_and_invalid_lend_destinations() {
    for number in [0, 28, 29, 43, 50, 51, 68] {
        for selector in [1u64 << 32, (1u64 << 32) | 5, u64::MAX] {
            assert!(!valid_syscall_selectors(number, selector, 13));
        }
    }
    for destination in [0, 1025, (1u64 << 32) | 13, u64::MAX] {
        assert!(!valid_syscall_selectors(50, 5, destination));
    }
}

#[test]
fn syscall_preflight_preserves_existing_valid_calls() {
    for number in [0, 28, 29, 43, 50, 51, 68] {
        assert!(valid_syscall_selectors(number, 5, 13));
    }
    assert!(valid_syscall_selectors(0, 0, u64::MAX));
    assert!(valid_syscall_selectors(28, 0, u64::MAX));
    assert!(valid_syscall_selectors(29, 0, 0));
    for number in [43, 50, 51, 68] {
        assert!(!valid_syscall_selectors(number, 0, 13));
    }
    // Unrelated raw memory interfaces retain their existing behavior.
    for number in [30, 31, 40] {
        assert!(valid_syscall_selectors(number, u64::MAX, u64::MAX));
    }
}

#[test]
fn dispatch_rejects_before_any_operation_side_effect() {
    // Integration source guard complements the host-executed production
    // preflight: its rejection must return before the dispatch body, where
    // dequeue, allocation, mapping, and reply insertion occur.
    let source = include_str!("../kernel/src/syscalls/mod.rs");
    let dispatch = source.split("pub fn dispatch(regs: &mut SyscallRegs)").nth(1).unwrap();
    let preflight = dispatch.find("if !crate::capability_slot::valid_syscall_selectors(rax, rdi, rdx)").unwrap();
    let body = dispatch.find("let result = match rax").unwrap();
    assert!(preflight < body);
    assert!(dispatch[preflight..body].contains("regs.rax = error;\n        return error;"));
    assert!(!dispatch[..preflight].contains("GLOBAL_ALLOCATOR"));
    assert!(!dispatch[..preflight].contains("send_reply"));
    assert!(!dispatch[..preflight].contains(".dequeue("));
}

#[test]
fn rejects_slots_beyond_table_even_without_truncation() {
    for raw in [1025, 2048, u32::MAX as u64, u64::MAX] {
        assert_eq!(decode_capability_slot(raw), None);
        assert_eq!(decode_pdx_slot(raw), None);
    }
}

#[test]
fn preserves_exact_registry_ids_including_reserved_zero() {
    for id in 0..ipc::MAX_DOMAINS as u64 {
        assert_eq!(decode_domain_id(id), Some(id as u32));
    }
}

#[test]
fn rejects_reply_destinations_outside_registry_or_with_high_bits() {
    for id in [1024, 2048, (1u64 << 32) | 1, 1u64 << 32, u64::MAX] {
        assert_eq!(decode_domain_id(id), None, "destination {id}");
    }
}
