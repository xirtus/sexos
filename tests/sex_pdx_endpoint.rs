//! Host tests of the IPC v2 message layout (crates/sex-pdx/src/endpoint.rs).
#[path = "../crates/sex-pdx/src/endpoint.rs"]
mod endpoint;
use endpoint::*;

#[test]
fn info_word_round_trips() {
    let info = MessageInfo::new(0xDEAD_BEEF, 7, 4096).unwrap();
    assert_eq!((info.label(), info.registers(), info.buffer_len()), (0xDEAD_BEEF, 7, 4096));
    assert_eq!(MessageInfo::from_raw(info.raw()), Ok(info));
    assert_eq!(MessageInfo::new(1, 8, 0), Err(MessageInfoError::TooManyRegisters));
    assert_eq!(MessageInfo::new(1, 0, 4097), Err(MessageInfoError::BufferTooLong));
}

#[test]
fn untrusted_info_words_are_rejected() {
    assert_eq!(MessageInfo::from_raw(1 << 17), Err(MessageInfoError::ReservedBits));
    assert_eq!(MessageInfo::from_raw(8), Err(MessageInfoError::TooManyRegisters));
    assert_eq!(MessageInfo::from_raw(4097 << 4), Err(MessageInfoError::BufferTooLong));
    assert_eq!(MessageInfo::from_raw(0xFFFF_FFFF_0000_0000).map(|i| i.label()), Ok(u32::MAX));
}

#[test]
fn payload_is_only_the_registers_in_use() {
    let m = Message::new(5, &[10, 20, 30]).unwrap();
    assert_eq!(m.payload(), &[10, 20, 30]);
    assert_eq!(m.info().unwrap().label(), 5);
    assert!(Message::new(5, &[0; 8]).is_err());
    let mut forged = Message::empty();
    forged.words[0] = 0xF;
    assert_eq!(forged.payload().len(), PAYLOAD_REGISTERS, "never reads past the registers");
}

#[test]
fn ipc_buffer_must_lie_in_the_callers_region() {
    let info = MessageInfo::new(0, 0, 100).unwrap();
    assert_eq!(validate_ipc_buffer(0x10_0000, info, 0x10_0000, 0x1000), Some(0x10_0000));
    assert_eq!(validate_ipc_buffer(0x10_0010, info, 0x10_0000, 0x2000), None, "unaligned");
    assert_eq!(validate_ipc_buffer(0x0F_F000, info, 0x10_0000, 0x2000), None, "below region");
    assert_eq!(validate_ipc_buffer(0x10_1000, MessageInfo::new(0, 0, 4096).unwrap(), 0x10_0000, 0x1800), None, "past end");
    assert_eq!(validate_ipc_buffer(u64::MAX - 0xFFF, info, 0, u64::MAX - 0x1000), None, "top of address space");
    assert_eq!(validate_ipc_buffer(0x123, MessageInfo::EMPTY, 0, 0), Some(0x123), "no buffer used, nothing to check");
}

#[test]
fn notification_bits() {
    assert_eq!(notification_bit(0), 1);
    assert_eq!(notification_bit(63), 1 << 63);
    assert_eq!(notification_bit(64), 0);
}
