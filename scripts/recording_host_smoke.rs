//! Host boundary: system FFmpeg/libc, actual Rust encoder/output and C bridge.
//! This does not qualify the guest FFmpeg build or grant capture/file authority.
extern crate alloc;
#[path = "../crates/sex-media/src/recording/mod.rs"]
mod recording;
fn init_libc() {} // Declared host boundary: C allocations/logging use system libc.
use recording::{Config, Drain, Encoder, Error};

fn drain(encoder: &mut Encoder, file: &mut Vec<u8>) -> bool {
    loop {
        match encoder.output().unwrap() {
            Drain::NeedFrame => return false,
            Drain::End => return true,
            Drain::Write(write) => {
                let (token, offset, n) = (write.token, write.offset as usize, write.bytes.len());
                assert!(n <= recording::TURN_BYTES && offset + n < 64 * 1024 * 1024);
                file.resize(file.len().max(offset + n), 0);
                file[offset..offset + n].copy_from_slice(write.bytes);
                encoder.acknowledge(token, n).unwrap();
                assert_eq!(encoder.acknowledge(token, n), Err(Error::Invalid));
            }
        }
    }
}
fn main() {
    let dest = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&dest).unwrap();
    assert!(matches!(
        Encoder::new(Config {
            width: 0,
            height: 49,
            fps: 5
        }),
        Err(Error::Invalid)
    ));
    let config = Config {
        width: 65,
        height: 49,
        fps: 5,
    };
    let mut encoder = Encoder::new(config).unwrap();
    let mut pixels = vec![0; 65 * 49];
    assert_eq!(encoder.frame(&pixels, 0), Err(Error::PendingOutput));
    let mut movie = Vec::new();
    let mut expected = Vec::new();
    assert!(!drain(&mut encoder, &mut movie));
    assert_eq!(encoder.frame(&pixels[..1], 0), Err(Error::Invalid));
    for (frame, timestamp) in [0, 200, 600, 800].into_iter().enumerate() {
        for y in 0..49 {
            for x in 0..65 {
                let (r, g, b) = (
                    ((x + frame * 17) % 256) as u8,
                    ((y * 3 + frame * 7) % 256) as u8,
                    (((x ^ y) + frame * 33) % 256) as u8,
                );
                pixels[y * 65 + x] = (r as u32) << 16 | (g as u32) << 8 | b as u32;
                expected.extend_from_slice(&[r, g, b]);
            }
        }
        encoder.frame(&pixels, timestamp).unwrap();
        // The C frame owns its input; mutating the caller's buffer cannot alter
        // already submitted pictures, including any delayed encoder packet.
        pixels.fill(0x00ff_00ff);
        drain(&mut encoder, &mut movie);
        assert_eq!(encoder.frame(&pixels, timestamp), Err(Error::Invalid));
    }
    encoder.finish().unwrap();
    assert!(drain(&mut encoder, &mut movie));
    assert_eq!(encoder.frames(), 4);
    assert_eq!(encoder.length(), movie.len() as u64);
    assert_eq!(encoder.buffered_bytes(), 0);
    assert_eq!(encoder.finish(), Err(Error::Closed));
    std::fs::write(dest.join("proof.mkv"), &movie).unwrap();
    std::fs::write(dest.join("expected.rgb"), &expected).unwrap();
    println!(
        "{{\"ok\":true,\"frames\":4,\"width\":65,\"height\":49,\"bytes\":{}}}",
        movie.len()
    );
}
