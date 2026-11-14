//! No pointers cross the helper boundary. A completed run must match both its
//! authenticated Shell identity and the caller-generated, nonzero 128-bit nonce.
use alloc::vec::Vec;
pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 256;
pub const MAX_FRAMES: u32 = RATE * 60;
pub const MAX_EVENTS: usize = 256;
pub const MAX_STATE: usize = 64 * 1024;
pub const HEADER: usize = 40;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    Header,
    Bounds,
    Nonce,
    Identity,
    Sequence,
    Audio,
    State,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub row: u32,
    pub task: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    CancelWaiting,
    Running(Identity),
    Cancelling(Identity),
    Complete,
    Failed,
}
pub struct Job {
    nonce: [u8; 16],
    cookie: u32,
    frames: u32,
    status: Status,
}
impl Job {
    pub fn new(nonce: [u8; 16], cookie: u32, frames: u32) -> Result<Self, Error> {
        if nonce == [0; 16] {
            return Err(Error::Nonce);
        }
        if cookie == 0 || frames == 0 || frames > MAX_FRAMES {
            return Err(Error::Bounds);
        }
        Ok(Self {
            nonce,
            cookie,
            frames,
            status: Status::Waiting,
        })
    }
    pub fn cookie(&self) -> u32 {
        self.cookie
    }
    pub fn status(&self) -> Status {
        self.status
    }
    pub fn started(&mut self, cookie: u32, id: Identity) -> Result<(), Error> {
        if cookie != self.cookie
            || id.task == 0
            || !matches!(self.status, Status::Waiting | Status::CancelWaiting)
        {
            return Err(Error::Identity);
        }
        self.status = if self.status == Status::CancelWaiting {
            Status::Cancelling(id)
        } else {
            Status::Running(id)
        };
        Ok(())
    }
    /// Cancellation is terminal only after the exact run's retirement receipt.
    pub fn cancel(&mut self) {
        match self.status {
            Status::Running(id) => self.status = Status::Cancelling(id),
            Status::Waiting => self.status = Status::CancelWaiting,
            _ => (),
        }
    }
    pub fn retired(&mut self, cookie: u32, id: Identity, code: Option<u32>) -> Result<(), Error> {
        if cookie != self.cookie {
            return Err(Error::Identity);
        }
        match self.status {
            Status::Running(who) if who == id => {
                self.status = if code == Some(0) {
                    Status::Complete
                } else {
                    Status::Failed
                };
                Ok(())
            }
            Status::Cancelling(who) if who == id => {
                self.status = Status::Failed;
                Ok(())
            }
            _ => Err(Error::Identity),
        }
    }
    pub fn result(&self, bytes: &[u8]) -> Result<Vec<[f32; 2]>, Error> {
        if self.status != Status::Complete {
            return Err(Error::State);
        }
        decode(bytes, self.nonce, self.frames)
    }
}
/// A fixed-size header followed by exactly `frames` interleaved stereo f32 frames.
/// No optional chunks, trailing data, state or extension lengths are trusted.
pub fn encode(nonce: [u8; 16], frames: &[[f32; 2]]) -> Result<Vec<u8>, Error> {
    encode_inner(nonce, frames, 0)
}
fn encode_inner(nonce: [u8; 16], frames: &[[f32; 2]], extra: usize) -> Result<Vec<u8>, Error> {
    if nonce == [0; 16] || frames.is_empty() || frames.len() > MAX_FRAMES as usize {
        return Err(Error::Bounds);
    }
    if frames
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 16.0)
    {
        return Err(Error::Audio);
    }
    let mut out = buffer(HEADER + frames.len() * 8 + extra)?;
    out.extend_from_slice(b"MOANPCM1");
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for frame in frames {
        for v in frame {
            out.extend_from_slice(&v.to_le_bytes())
        }
    }
    Ok(out)
}
pub fn decode(bytes: &[u8], nonce: [u8; 16], frames: u32) -> Result<Vec<[f32; 2]>, Error> {
    if frames == 0 || frames > MAX_FRAMES || bytes.len() != HEADER + frames as usize * 8 {
        return Err(Error::Bounds);
    }
    if &bytes[..8] != b"MOANPCM1"
        || bytes[24..28] != RATE.to_le_bytes()
        || bytes[28..32] != frames.to_le_bytes()
        || bytes[32..36] != 2u32.to_le_bytes()
        || bytes[36..40] != [0; 4]
    {
        return Err(Error::Header);
    }
    if nonce == [0; 16] || bytes[8..24] != nonce {
        return Err(Error::Nonce);
    }
    let mut out = buffer(frames as usize)?;
    for c in bytes[HEADER..].chunks_exact(8) {
        let a = f32::from_le_bytes(c[..4].try_into().unwrap());
        let b = f32::from_le_bytes(c[4..].try_into().unwrap());
        if !a.is_finite() || !b.is_finite() || a.abs() > 16.0 || b.abs() > 16.0 {
            return Err(Error::Audio);
        }
        out.push([a, b]);
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_retirement_precedes_result() {
        let n = [7; 16];
        let b = encode(n, &[[0.25, -0.5]; 3]).unwrap();
        let mut j = Job::new(n, 4, 3).unwrap();
        let id = Identity { row: 42, task: 99 };
        assert_eq!(j.result(&b), Err(Error::State));
        j.started(4, id).unwrap();
        assert_eq!(
            j.retired(4, Identity { task: 100, ..id }, Some(0)),
            Err(Error::Identity)
        );
        assert_eq!(j.result(&b), Err(Error::State));
        j.retired(4, id, Some(0)).unwrap();
        assert_eq!(j.result(&b).unwrap(), [[0.25, -0.5]; 3]);
        assert!(j.retired(4, id, Some(0)).is_err());
    }
    #[test]
    fn cancel_crash_stale_and_partial_never_publish() {
        for code in [None, Some(1), Some(0)] {
            let mut j = Job::new([1; 16], 1, 1).unwrap();
            let id = Identity { row: 42, task: 99 };
            j.started(1, id).unwrap();
            j.cancel();
            j.retired(1, id, code).unwrap();
            assert_eq!(j.status(), Status::Failed);
        }
        let b = encode([2; 16], &[[1.0, 0.0]]).unwrap();
        assert_eq!(decode(&b, [3; 16], 1), Err(Error::Nonce));
        assert_eq!(decode(&b[..b.len() - 1], [2; 16], 1), Err(Error::Bounds));
    }
    #[test]
    fn finite_and_exact_bounds() {
        assert_eq!(encode([1; 16], &[[f32::NAN, 0.0]]), Err(Error::Audio));
        assert_eq!(Job::new([0; 16], 1, 1).err(), Some(Error::Nonce));
        let mut b = encode([1; 16], &[[0.0; 2]]).unwrap();
        b.extend_from_slice(&[0]);
        assert_eq!(decode(&b, [1; 16], 1), Err(Error::Bounds));
    }
}

pub const MAX_PARAMS: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameter {
    pub id: u32,
    pub value: f64,
}
pub struct Request {
    pub pcm: Vec<[f32; 2]>,
    pub parameters: Vec<Parameter>,
}
fn valid_parameters(parameters: &[Parameter]) -> bool {
    parameters.len() <= MAX_PARAMS
        && parameters.iter().enumerate().all(|(i, p)| {
            p.id != u32::MAX
                && p.value.is_finite()
                && p.value.abs() <= 1e9
                && !parameters[..i].iter().any(|q| q.id == p.id)
        })
}
/// Initial parameters at frame zero; automation/MIDI/state chunks are deliberately
/// absent from this first offline-effect format. Unknown trailing data is refused.
pub fn encode_request(
    nonce: [u8; 16],
    pcm: &[[f32; 2]],
    parameters: &[Parameter],
) -> Result<Vec<u8>, Error> {
    if !valid_parameters(parameters) {
        return Err(Error::Bounds);
    }
    let mut out = encode_inner(nonce, pcm, parameters.len() * 16)?;
    out[..8].copy_from_slice(b"MOANREQ1");
    out[36..40].copy_from_slice(&(parameters.len() as u32).to_le_bytes());
    for p in parameters {
        out.extend_from_slice(&p.id.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&p.value.to_le_bytes());
    }
    Ok(out)
}
pub fn decode_request(bytes: &[u8], nonce: [u8; 16]) -> Result<Request, Error> {
    if bytes.len() < HEADER || &bytes[..8] != b"MOANREQ1" {
        return Err(Error::Header);
    }
    let count = u32::from_le_bytes(bytes[36..40].try_into().unwrap()) as usize;
    let frames = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    if count > MAX_PARAMS || frames == 0 || frames > MAX_FRAMES {
        return Err(Error::Bounds);
    }
    let end = HEADER + frames as usize * 8;
    if bytes.len() != end + count * 16 {
        return Err(Error::Bounds);
    }
    // A single bounded PCM conversion; no temporary full-size plaintext file copy.
    if nonce == [0; 16] || bytes[8..24] != nonce {
        return Err(Error::Nonce);
    }
    if bytes[24..28] != RATE.to_le_bytes() || bytes[32..36] != 2u32.to_le_bytes() {
        return Err(Error::Header);
    }
    let mut pcm = buffer(frames as usize)?;
    for c in bytes[HEADER..end].chunks_exact(8) {
        let a = f32::from_le_bytes(c[..4].try_into().unwrap());
        let b = f32::from_le_bytes(c[4..].try_into().unwrap());
        if !a.is_finite() || !b.is_finite() || a.abs() > 16.0 || b.abs() > 16.0 {
            return Err(Error::Audio);
        }
        pcm.push([a, b]);
    }
    let mut parameters = buffer(count)?;
    for c in bytes[end..].chunks_exact(16) {
        if c[4..8] != [0; 4] {
            return Err(Error::Header);
        }
        parameters.push(Parameter {
            id: u32::from_le_bytes(c[..4].try_into().unwrap()),
            value: f64::from_le_bytes(c[8..].try_into().unwrap()),
        });
    }
    if !valid_parameters(&parameters) {
        return Err(Error::Bounds);
    }
    Ok(Request { pcm, parameters })
}
#[cfg(test)]
mod request_tests {
    use super::*;
    #[test]
    fn parameters_roundtrip_and_duplicates_rejected() {
        let p = [Parameter { id: 1, value: 0.25 }];
        let b = encode_request([1; 16], &[[0.0, 0.5]], &p).unwrap();
        let r = decode_request(&b, [1; 16]).unwrap();
        assert_eq!(r.pcm, [[0.0, 0.5]]);
        assert_eq!(r.parameters, p);
        assert_eq!(
            encode_request([1; 16], &[[0.0; 2]], &[p[0], p[0]]),
            Err(Error::Bounds)
        );
        assert!(decode_request(&b, [2; 16]).is_err());
    }
    #[test]
    fn malformed_parameter_lengths_and_nan_rejected() {
        let b = encode_request([1; 16], &[[0.0; 2]], &[Parameter { id: 0, value: 0.25 }]).unwrap();
        for n in 0..b.len() {
            assert!(decode_request(&b[..n], [1; 16]).is_err());
        }
        let mut bad = b.clone();
        bad[36..40].copy_from_slice(&65u32.to_le_bytes());
        assert!(decode_request(&bad, [1; 16]).is_err());
        let n = bad.len();
        bad = b;
        bad[n - 8..].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode_request(&bad, [1; 16]).is_err());
    }
    #[test]
    fn cancelling_before_started_retains_exact_retirement_obligation() {
        let mut j = Job::new([1; 16], 9, 1).unwrap();
        j.cancel();
        assert_eq!(j.status(), Status::CancelWaiting);
        let id = Identity { row: 43, task: 102 };
        j.started(9, id).unwrap();
        assert_eq!(j.status(), Status::Cancelling(id));
        j.retired(9, id, Some(0)).unwrap();
        assert_eq!(j.status(), Status::Failed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Clap,
    Vst3,
}
impl Format {
    pub fn token(self) -> &'static [u8] {
        match self {
            Self::Clap => b"clap",
            Self::Vst3 => b"vst3",
        }
    }
}

pub(crate) fn buffer<T>(count: usize) -> Result<Vec<T>, Error> {
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| Error::Memory)?;
    Ok(result)
}
