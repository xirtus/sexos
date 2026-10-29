//! Actual production bodies, symbol-renamed only to avoid replacing host libc.
include!(env!("SILK_MEMORY_SOURCE"));

#[test]
fn copy_varied_lengths_alignments_and_guard_bytes() {
    for n in [0usize,1,2,7,8,15,16,31,63,64,127,255,256,511,1024] {
        for offset in 0..16 {
            let source:Vec<u8>=(0..(n+32)).map(|i|(i.wrapping_mul(37)^0xa5) as u8).collect();
            let mut destination=vec![0xcc;n+32];
            let ptr=unsafe{destination.as_mut_ptr().add(offset)};
            assert_eq!(unsafe{tested_memcpy(ptr,source.as_ptr().add(15-offset),n)},ptr);
            assert_eq!(&destination[offset..offset+n],&source[15-offset..15-offset+n]);
            assert!(destination[..offset].iter().chain(destination[offset+n..].iter()).all(|&v|v==0xcc));
        }
    }
}
#[test]
fn memset_truncates_byte_and_preserves_guards() {
    for value in [0,1,0x7f,0xff,0x1234,-1,i32::MIN] {
        for n in [0,1,7,16,63,255,256,1024] {
            let mut bytes=vec![0xa5;n+32];
            let ptr=unsafe{bytes.as_mut_ptr().add(3)};
            assert_eq!(unsafe{tested_memset(ptr,value,n)},ptr);
            assert!(bytes[3..3+n].iter().all(|&b|b==value as u8));
            assert!(bytes[..3].iter().chain(bytes[3+n..].iter()).all(|&b|b==0xa5));
        }
    }
}
#[test]
fn zero_length_does_not_dereference_null() {
    assert!(unsafe{tested_memcpy(core::ptr::null_mut(),core::ptr::null(),0)}.is_null());
    assert!(unsafe{tested_memset(core::ptr::null_mut(),42,0)}.is_null());
}
#[test]
fn repeated_record_sized_copies_retain_exact_bytes() {
    let source=[0x59;256];let mut target=[0u8;256];
    for i in 0..10000 {
        unsafe{tested_memset(target.as_mut_ptr(),i,256);tested_memcpy(target.as_mut_ptr(),source.as_ptr(),256);}
        assert_eq!(target,source);
    }
}
