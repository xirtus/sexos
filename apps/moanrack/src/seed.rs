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
