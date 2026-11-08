// The freestanding copies every service and the kernel use for memcpy/memset/memmove.
#[path = "../crates/sex-pdx/src/mem.rs"]
mod mem;

fn pattern(n: usize) -> Vec<u8> { (0..n).map(|i| (i * 31 + 7) as u8).collect() }

#[test]
fn copies_fill_and_moves_match_the_standard_library_at_every_size_and_offset() {
    for n in [0usize, 1, 7, 8, 9, 15, 16, 63, 64, 65, 4095, 4096, 4097, 131072] {
        for (so, d_o) in [(0usize, 0usize), (1, 3), (7, 0), (0, 5)] {
            let src = pattern(n + 16);
            let mut dst = vec![0xEEu8; n + 16];
            unsafe { mem::copy_forward(dst.as_mut_ptr().add(d_o), src.as_ptr().add(so), n); }
            assert_eq!(&dst[d_o..d_o + n], &src[so..so + n]);
            assert!(dst[..d_o].iter().all(|&b| b == 0xEE) && dst[d_o + n..].iter().all(|&b| b == 0xEE));
            let mut f = vec![0u8; n + 16];
            unsafe { mem::fill(f.as_mut_ptr().add(d_o), 0xA5, n); }
            assert!(f[d_o..d_o + n].iter().all(|&b| b == 0xA5));
            assert!(f[..d_o].iter().all(|&b| b == 0) && f[d_o + n..].iter().all(|&b| b == 0));
        }
        // Overlapping moves both ways.
        for shift in [1usize, 3, 8, 13] {
            let mut a = pattern(n + shift);
            let mut want = a.clone();
            want.copy_within(0..n, shift);
            unsafe { mem::move_bytes(a.as_mut_ptr().add(shift), a.as_ptr(), n); }
            assert_eq!(a, want, "up n={n} shift={shift}");
            let mut b = pattern(n + shift);
            let mut want = b.clone();
            want.copy_within(shift..shift + n, 0);
            unsafe { mem::move_bytes(b.as_mut_ptr(), b.as_ptr().add(shift), n); }
            assert_eq!(b, want, "down n={n} shift={shift}");
        }
    }
}
