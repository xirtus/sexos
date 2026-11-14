// SPDX-License-Identifier: GPL-3.0-only
//! Host tool: connects to a VNC server, takes the first full update and writes it as a
//! PPM. `cargo run -p sex-rfb --example snapshot -- 127.0.0.1:5900 out.ppm [PASSWORD]`.
//! Checks sex-rfb against real servers (and the gate's Python server) without QEMU.
use sex_rfb::client::{Client, Event};
use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (addr, out) = (&args[0], &args[1]);
    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    let mut client = Client::new(args.get(2).map(|p| p.as_bytes()));
    let mut buf = vec![0u8; 65536];
    loop {
        let n = stream.read(&mut buf).expect("read");
        assert!(n > 0, "server closed");
        client.feed(&buf[..n]);
        while let Some(e) = client.next_event().expect("protocol") {
            eprintln!("{e:?}");
            if let Event::Updated { .. } = e {
                let fb = &client.fb;
                let mut ppm = format!("P6\n{} {}\n255\n", fb.width, fb.height).into_bytes();
                for p in &fb.pixels { ppm.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
                std::fs::write(out, ppm).expect("write");
                client.tap(0x61, false);
                stream.write_all(&client.take_output()).unwrap();
                return;
            }
        }
        stream.write_all(&client.take_output()).expect("write");
    }
}
