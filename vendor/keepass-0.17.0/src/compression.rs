use flate2::read::GzDecoder;
#[cfg(feature = "save_kdbx4")]
use flate2::write::GzEncoder;
#[cfg(feature = "save_kdbx4")]
use flate2::Compression as Flate2Compression;
use std::io::{Error, ErrorKind, Read};
use zeroize::Zeroizing;

/// SexOS admission ceiling for each expanded inner payload or attachment.
pub const MAX_DECOMPRESSED_BYTES: usize = 16 * 1024 * 1024;
fn size_error() -> Error {
    Error::new(
        ErrorKind::InvalidData,
        "KeePass expanded payload exceeds 16MiB",
    )
}

pub(crate) fn attachment_size_error() -> Error {
    Error::new(
        ErrorKind::InvalidData,
        "KeePass cumulative attachments exceed 16MiB",
    )
}
#[cfg(test)]
std::thread_local! { pub(crate) static DECODE_LIMITS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) }; }

fn bounded_decode(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, Error> {
    let limit = limit.min(MAX_DECOMPRESSED_BYTES);
    #[cfg(test)]
    DECODE_LIMITS.with(|limits| limits.borrow_mut().push(limit));
    // Reserve once: growing a Vec can release an unwiped plaintext allocation.
    // Read fixed-size pieces, with only a one-byte probe beyond the ceiling.
    let mut output = Zeroizing::new(vec![0u8; limit]);
    let mut used = 0;
    loop {
        if used == limit {
            let mut probe = Zeroizing::new([0u8; 1]);
            return if reader.read(&mut probe[..])? == 0 {
                Ok(std::mem::take(&mut *output))
            } else {
                Err(size_error())
            };
        }
        let end = (used + 8192).min(limit);
        let n = reader.read(&mut output[used..end])?;
        if n == 0 {
            output.truncate(used);
            return Ok(std::mem::take(&mut *output));
        }
        used += n;
    }
}

#[cfg(feature = "save_kdbx4")]
use std::io::Write;

pub trait Compression {
    #[cfg(feature = "save_kdbx4")]
    fn compress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error>;
    fn decompress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error>;
}

pub struct NoCompression;

impl Compression for NoCompression {
    #[cfg(feature = "save_kdbx4")]
    fn compress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        if in_buffer.len() > MAX_DECOMPRESSED_BYTES {
            return Err(size_error());
        }
        Ok(in_buffer.to_vec())
    }
    fn decompress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        if in_buffer.len() > MAX_DECOMPRESSED_BYTES {
            return Err(size_error());
        }
        Ok(in_buffer.to_vec())
    }
}

pub struct GZipCompression;
impl GZipCompression {
    pub(crate) fn decompress_limited(&self, input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
        bounded_decode(GzDecoder::new(input), limit)
    }
}

impl Compression for GZipCompression {
    #[cfg(feature = "save_kdbx4")]
    fn compress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        if in_buffer.len() > MAX_DECOMPRESSED_BYTES {
            return Err(size_error());
        }
        let mut res = Zeroizing::new(Vec::new());
        let mut encoder = GzEncoder::new(&mut *res, Flate2Compression::default());
        encoder.write_all(in_buffer)?;
        encoder.flush()?;
        encoder.finish()?;
        Ok(std::mem::take(&mut *res))
    }
    fn decompress(&self, in_buffer: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        self.decompress_limited(in_buffer, MAX_DECOMPRESSED_BYTES)
    }
}

#[cfg(test)]
mod sexos_tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression as Level};
    use std::io::Write;
    fn gzip(n: usize) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Level::fast());
        let chunk = [b'x'; 8192];
        for _ in 0..n / chunk.len() {
            encoder.write_all(&chunk).unwrap();
        }
        encoder.write_all(&chunk[..n % chunk.len()]).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn gzip_boundary_and_bomb_are_bounded() {
        let exact = gzip(MAX_DECOMPRESSED_BYTES);
        assert_eq!(
            GZipCompression.decompress(&exact).unwrap().len(),
            MAX_DECOMPRESSED_BYTES
        );
        let bomb = gzip(MAX_DECOMPRESSED_BYTES + 1);
        assert!(bomb.len() < 100_000);
        assert_eq!(
            GZipCompression.decompress(&bomb).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
    #[test]
    fn malformed_and_truncated_streams_fail() {
        assert!(GZipCompression.decompress(b"not gzip").is_err());
        let mut truncated = gzip(8192);
        truncated.truncate(truncated.len() - 4);
        assert!(GZipCompression.decompress(&truncated).is_err());
    }
    #[test]
    fn decoder_reads_at_most_limit_plus_one() {
        struct Endless(usize);
        impl Read for Endless {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.0 += out.len();
                out.fill(0x42);
                Ok(out.len())
            }
        }
        let mut reader = Endless(0);
        assert!(bounded_decode(&mut reader, MAX_DECOMPRESSED_BYTES).is_err());
        assert_eq!(reader.0, MAX_DECOMPRESSED_BYTES + 1);
    }
    #[test]
    fn plain_boundary_rejects_before_copy() {
        let mut bytes = vec![0; MAX_DECOMPRESSED_BYTES];
        assert_eq!(NoCompression.decompress(&bytes).unwrap().len(), bytes.len());
        bytes.push(1);
        assert_eq!(
            NoCompression.decompress(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}
