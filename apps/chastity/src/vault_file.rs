// SPDX-License-Identifier: GPL-3.0-only
//! Admission before file allocation, separate from authenticated KDBX preflight.
//! Only NotFound means Create. Refused existing files remain present/locked.
use std::io::{self, Read};
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub enum Loaded {
    Missing,
    Present(Vec<u8>),
    Refused,
}
fn bounded(mut reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    // Fixed allocation, no read_to_end growth or filesystem-reported length trust.
    let mut bytes = vec![0; limit];
    let mut used = 0;
    while used < limit {
        match reader.read(&mut bytes[used..]) {
            Ok(0) => {
                bytes.truncate(used);
                return Ok(bytes);
            }
            Ok(n) => used += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    let mut probe = [0];
    loop {
        match reader.read(&mut probe) {
            Ok(0) => return Ok(bytes),
            Ok(_) => return Err(io::Error::from(io::ErrorKind::InvalidData)),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}
pub fn admit<R: Read>(opened: io::Result<R>) -> Loaded {
    match opened {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Loaded::Missing,
        Err(_) => Loaded::Refused,
        Ok(file) => match bounded(file, MAX_BYTES) {
            Ok(bytes) => Loaded::Present(bytes),
            Err(_) => Loaded::Refused,
        },
    }
}
/// Preserve present status on errors without handing parser unbounded input.
pub fn parser_input(loaded: Loaded) -> Option<Vec<u8>> {
    match loaded {
        Loaded::Missing => None,
        Loaded::Present(bytes) => Some(bytes),
        Loaded::Refused => Some(Vec::new()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Huge {
        remaining: usize,
        read: usize,
    }
    impl Read for Huge {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = out.len().min(self.remaining);
            out[..n].fill(7);
            self.remaining -= n;
            self.read += n;
            Ok(n)
        }
    }
    #[test]
    fn oversized_reader_never_reads_or_allocates_the_announced_file_size() {
        let mut reader = Huge {
            remaining: usize::MAX,
            read: 0,
        };
        assert!(bounded(&mut reader, 64).is_err());
        assert_eq!(reader.read, 65);
        let mut exact = Huge {
            remaining: 64,
            read: 0,
        };
        let bytes = bounded(&mut exact, 64).unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(bytes.capacity(), 64);
    }
    #[test]
    fn only_missing_files_can_start_create() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidData,
            io::ErrorKind::Other,
        ] {
            assert_eq!(
                parser_input(admit::<io::Empty>(Err(io::Error::from(kind)))),
                Some(Vec::new())
            );
        }
        assert_eq!(
            parser_input(admit::<io::Empty>(Err(io::Error::from(
                io::ErrorKind::NotFound
            )))),
            None
        );
        assert_eq!(
            parser_input(admit(Ok(io::Cursor::new(b"vault".to_vec())))),
            Some(b"vault".to_vec())
        );
    }
    #[test]
    fn existing_oversized_file_is_present_not_new() {
        assert!(matches!(
            admit(Ok(Huge {
                remaining: MAX_BYTES + 1,
                read: 0
            })),
            Loaded::Refused
        ));
        assert!(parser_input(Loaded::Refused).is_some());
    }
}
