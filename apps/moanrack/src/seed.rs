//! Developer-only asymmetric PCM used by the actual Moan UI gate.
extern crate alloc;
use alloc::vec::Vec;
pub const FRAMES: usize = 48_000;
pub fn sample(n: usize) -> [i16; 2] {
    [((n % 127) as i16 - 63) * 128, ((n % 97) as i16 - 48) * 96]
}
pub fn wav() -> Vec<u8> {
    let data = FRAMES * 4;
    let mut bytes = Vec::with_capacity(data + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data as u32 + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&192_000u32.to_le_bytes());
    bytes.extend_from_slice(b"\x04\0\x10\0data");
    bytes.extend_from_slice(&(data as u32).to_le_bytes());
    for n in 0..FRAMES {
        for value in sample(n) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_stereo_wav_and_asymmetric_frames() {
        let bytes = wav();
        assert_eq!(bytes.len(), 44 + FRAMES * 4);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize,
            FRAMES * 4
        );
        for n in 0..FRAMES {
            let at = 44 + n * 4;
            assert_eq!(
                [
                    i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()),
                    i16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap())
                ],
                sample(n)
            );
        }
        assert!(bytes[44..].chunks_exact(4).any(|f| f[..2] != f[2..]));
    }
}

/// Separate live-mixer qualification source, 12s of asymmetric400/600Hz PCM.
/// Exact-period resets prevent oscillator drift; no host audio file dependency.
pub fn live_wav() -> Vec<u8> {
    const N: usize = 12 * 48_000;
    let mut bytes = Vec::with_capacity(44 + N * 4);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(N as u32 * 4 + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&192_000u32.to_le_bytes());
    bytes.extend_from_slice(b"\x04\0\x10\0data");
    bytes.extend_from_slice(&(N as u32 * 4).to_le_bytes());
    let mut phase = [(0.0f64, 1.0f64); 2];
    for n in 0..N {
        for (channel, (period, amplitude, sin, cos)) in [
            (120, 6000.0, 0.052335956242943835, 0.9986295347545738),
            (80, 8000.0, 0.07845909572784494, 0.996917333733128),
        ]
        .into_iter()
        .enumerate()
        {
            if n % period == 0 {
                phase[channel] = (0.0, 1.0);
            }
            let (s, c) = phase[channel];
            bytes.extend_from_slice(&((s * amplitude) as i16).to_le_bytes());
            phase[channel] = (s * cos + c * sin, c * cos - s * sin);
        }
    }
    bytes
}
#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    fn exact_duration_independent_channels_and_energy() {
        let bytes = live_wav();
        assert_eq!(bytes.len(), 44 + 12 * 48_000 * 4);
        for (channel, hz, amplitude) in [(0, 400, 6000.0), (1, 600, 8000.0)] {
            let mut sum = 0f64;
            let mut crossings = 0;
            let mut last = 0i16;
            for frame in bytes[44..].chunks_exact(4) {
                let value =
                    i16::from_le_bytes(frame[channel * 2..channel * 2 + 2].try_into().unwrap());
                if last <= 0 && value > 0 {
                    crossings += 1;
                }
                last = value;
                sum += (value as f64) * (value as f64);
            }
            assert_eq!(crossings, hz * 12);
            let expected = amplitude * amplitude / 2.0;
            assert!((sum / (12.0 * 48_000.0) - expected).abs() < expected * 0.001);
        }
    }
}
