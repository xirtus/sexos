// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The terminal stream protocol on the host: layout, wrap-around and an echo program.
use sex_pdx::stream::*;

fn memory(ring: usize) -> Vec<u32> { vec![0u32; bytes_needed(ring) / 4] }

#[test]
fn layout_and_attach() {
    let mut m = memory(64);
    let (p, len) = (m.as_mut_ptr() as *mut u8, m.len() * 4);
    let host = unsafe { Stream::init(p, len, 80, 24) }.unwrap();
    let guest = unsafe { Stream::attach(p, len) }.unwrap();
    assert_eq!(guest.size(), (80, 24));
    assert_eq!(host.writable(Side::Host), 64);
    let mut junk = vec![0u32; 32];
    assert!(unsafe { Stream::attach(junk.as_mut_ptr() as *mut u8, 128) }.is_none());
}

#[test]
fn wraps_around_and_respects_capacity() {
    let mut m = memory(16);
    let s = unsafe { Stream::init(m.as_mut_ptr() as *mut u8, m.len() * 4, 1, 1) }.unwrap();
    let mut out = [0u8; 16];
    for round in 0..10u8 {
        let data: Vec<u8> = (0..11).map(|i| round * 11 + i).collect();
        assert_eq!(s.write(Side::Guest, &data), 11);
        assert_eq!(s.readable(Side::Host), 11);
        assert_eq!(s.read(Side::Host, &mut out), 11);
        assert_eq!(&out[..11], &data[..]);
    }
    assert_eq!(s.write(Side::Host, &[7; 40]), 16);
    assert_eq!(s.write(Side::Host, &[7]), 0);
}

#[test]
fn echo_program_round_trip_across_threads() {
    // A guest on another thread reads lines and answers each one upper-cased, the
    // shape of every line-oriented program behind a terminal.
    let mut m = memory(256);
    let len = m.len() * 4;
    let p = m.as_mut_ptr() as usize;
    let host = unsafe { Stream::init(p as *mut u8, len, 80, 24) }.unwrap();
    let guest = std::thread::spawn(move || {
        let s = unsafe { Stream::attach(p as *mut u8, len) }.unwrap();
        let mut line = Vec::new();
        let mut buf = [0u8; 7];
        loop {
            let n = s.read(Side::Guest, &mut buf);
            if n == 0 {
                if s.gone(Side::Host) && s.readable(Side::Guest) == 0 { break; }
                std::thread::yield_now();
                continue;
            }
            for &b in &buf[..n] {
                line.push(b);
                if b == b'\n' {
                    let up: Vec<u8> = line.iter().map(|c| c.to_ascii_uppercase()).collect();
                    let mut sent = 0;
                    while sent < up.len() { sent += s.write(Side::Guest, &up[sent..]); }
                    line.clear();
                }
            }
        }
        s.hang_up(Side::Guest);
    });
    let input: Vec<u8> = (0..200).flat_map(|i| format!("line {i} of input\n").into_bytes()).collect();
    let (mut sent, mut got, mut buf) = (0, Vec::new(), [0u8; 100]);
    while sent < input.len() || got.len() < input.len() {
        if sent < input.len() { sent += host.write(Side::Host, &input[sent..]); }
        let n = host.read(Side::Host, &mut buf);
        got.extend_from_slice(&buf[..n]);
    }
    host.hang_up(Side::Host);
    guest.join().unwrap();
    assert!(host.gone(Side::Guest));
    assert_eq!(got, input.to_ascii_uppercase());
}

#[test]
fn size_interrupts_and_hang_up() {
    let mut m = memory(32);
    let s = unsafe { Stream::init(m.as_mut_ptr() as *mut u8, m.len() * 4, 80, 24) }.unwrap();
    let seen = s.size_changes();
    s.set_size(100, 30);
    assert_eq!((s.size(), s.size_changes() - seen), ((100, 30), 1));
    s.interrupt();
    s.interrupt();
    assert_eq!(s.interrupts(), 2);
    s.hang_up(Side::Guest);
    assert_eq!(s.write(Side::Host, b"x"), 0);
    assert!(!s.gone(Side::Host));
}
