// SPDX-License-Identifier: GPL-3.0-only
#![cfg(feature = "host-codecs")]
use moan_export::{Drain, Encoder, Error, Format};
use std::{path::PathBuf, process::Command};
static CODECS: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn drain(encoder: &mut Encoder, file: &mut Vec<u8>, short: bool) -> usize {
    let mut patches = 0;
    loop {
        match encoder.output().unwrap() {
            Drain::Write(write) => {
                let (token, at) = (write.token, write.offset as usize);
                let n = if short {
                    write.bytes.len().min(13)
                } else {
                    write.bytes.len()
                };
                if at < file.len() {
                    patches += 1;
                }
                file.resize(file.len().max(at + n), 0);
                file[at..at + n].copy_from_slice(&write.bytes[..n]);
                encoder.acknowledge(token, n).unwrap();
                assert_eq!(encoder.acknowledge(token, n), Err(Error::Invalid));
            }
            Drain::NeedFrames | Drain::End => return patches,
        }
    }
}
fn fixture(n: usize) -> Vec<[i16; 2]> {
    (0..n)
        .map(|i| {
            let sample =
                |hz: f64| ((i as f64 * std::f64::consts::TAU * hz / 48000.).sin() * 10000.) as i16;
            [sample(400.), sample(600.)]
        })
        .collect()
}
fn path(extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "moan-export-{}-{}.{extension}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ))
}
#[test]
fn real_codecs_probe_and_decode_with_ordered_header_patches_and_short_writes() {
    let _single_threaded_ffmpeg = CODECS.lock().unwrap();
    let n = 96037;
    let source = fixture(n);
    let pcm: Vec<u8> = source
        .iter()
        .flat_map(|f| f.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    for format in [Format::Wav, Format::Flac, Format::Mp3] {
        let mut encoder = Encoder::new(format, n as u64).unwrap();
        let mut file = Vec::new();
        assert_eq!(encoder.push(&[[0; 2]]), Err(Error::PendingOutput));
        let mut patches = drain(&mut encoder, &mut file, true);
        assert_eq!(encoder.finish(), Err(Error::Invalid));
        let capacity = encoder.frame_capacity();
        for block in source.chunks(capacity) {
            encoder.push(block).unwrap();
            patches += drain(&mut encoder, &mut file, true);
        }
        encoder.finish().unwrap();
        patches += drain(&mut encoder, &mut file, true);
        assert!(matches!(encoder.output().unwrap(), Drain::End));
        assert_eq!(encoder.frames(), n as u64);
        assert_eq!(encoder.length(), file.len() as u64);
        assert!(
            patches > 0,
            "container did not exercise seek-based final metadata patches"
        );
        assert_eq!(encoder.push(&[[0; 2]]), Err(Error::Closed));
        match format {
            Format::Wav => {
                assert_eq!(&file[..4], b"RIFF");
                assert_eq!(&file[8..12], b"WAVE");
                assert_eq!(
                    u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize + 8,
                    file.len()
                );
            }
            Format::Flac => assert_eq!(&file[..4], b"fLaC"),
            Format::Mp3 => {
                assert!(file.starts_with(b"ID3") || (file[0] == 0xff && file[1] & 0xe0 == 0xe0))
            }
        }
        let path = path(format.extension());
        std::fs::write(&path, &file).unwrap();
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let info = String::from_utf8(probe.stdout).unwrap();
        let codec = match format {
            Format::Wav => "pcm_s16le",
            Format::Flac => "flac",
            Format::Mp3 => "mp3",
        };
        assert!(
            info.contains(&format!("codec_name={codec}"))
                && info.contains("sample_rate=48000")
                && info.contains("channels=2"),
            "{info}"
        );
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        if format != Format::Mp3 {
            assert_eq!(decoded.stdout, pcm, "lossless export changed PCM");
        } else {
            let actual = decoded.stdout.len() / 4;
            assert!(
                actual.abs_diff(n) < 2304,
                "MP3 sample count {actual} vs {n}"
            );
            let samples: Vec<i16> = decoded
                .stdout
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            for (channel, hz) in [(0, 400.), (1, 600.)] {
                let values: Vec<_> = samples
                    .iter()
                    .skip(channel)
                    .step_by(2)
                    .skip(12000)
                    .take(48000)
                    .copied()
                    .collect();
                let crossings = values.windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count() as f64;
                assert!(
                    (crossings - hz).abs() < 5.,
                    "MP3 channel {channel}: {crossings}Hz"
                );
                let energy =
                    values.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / values.len() as f64;
                assert!(
                    (5000.0..8500.0).contains(&energy.sqrt()),
                    "MP3 level {}",
                    energy.sqrt()
                );
            }
        }
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn final_short_blocks_are_bounded_and_abort_never_finalizes() {
    let _single_threaded_ffmpeg = CODECS.lock().unwrap();
    for format in [Format::Wav, Format::Flac, Format::Mp3] {
        let mut encoder = Encoder::new(format, 12345).unwrap();
        let mut file = Vec::new();
        drain(&mut encoder, &mut file, false);
        let capacity = encoder.frame_capacity();
        assert_eq!(encoder.push(&[]), Err(Error::Invalid));
        assert_eq!(encoder.push(&[[0; 2]]), Err(Error::Invalid));
        assert_eq!(
            encoder.push(&vec![[0; 2]; capacity + 1]),
            Err(Error::Invalid)
        );
        encoder.push(&vec![[0; 2]; capacity]).unwrap();
        drain(&mut encoder, &mut file, false);
        let before = file.clone();
        drop(encoder);
        assert_eq!(file, before);
    }
    assert!(matches!(Encoder::new(Format::Wav, 0), Err(Error::Invalid)));
    assert!(matches!(
        Encoder::new(Format::Wav, moan_export::MAX_FRAMES + 1),
        Err(Error::Invalid)
    ));
}
