// SPDX-License-Identifier: GPL-3.0-only
//! A bounded voice-job message contract. Job and asset IDs are data, never authority.
//! A transport must authenticate the caller and enforce access to referenced assets.
#![no_std]

pub const VERSION: u8 = 1;
pub const MAX_FRAME: usize = 4096;
pub const MAX_TEXT: usize = 3072;
pub const MAX_MODEL: usize = 96;
pub const MAX_LANGUAGE: usize = 24;
const MAGIC: &[u8; 4] = b"SXVC";
const HEADER: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, TooLong, Magic, Version, Kind, Value, Trailing }

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation { CreateVoice = 1, Synthesize = 2, Continue = 3 }
impl Operation {
    fn parse(n: u8) -> Result<Self, Error> {
        match n { 1 => Ok(Self::CreateVoice), 2 => Ok(Self::Synthesize), 3 => Ok(Self::Continue), _ => Err(Error::Value) }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState { Queued = 1, Running = 2, Complete = 3, Cancelled = 4, Failed = 5 }
impl JobState {
    fn parse(n: u8) -> Result<Self, Error> {
        match n { 1 => Ok(Self::Queued), 2 => Ok(Self::Running), 3 => Ok(Self::Complete), 4 => Ok(Self::Cancelled), 5 => Ok(Self::Failed), _ => Err(Error::Value) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submit<'a> {
    pub operation: Operation,
    pub seed: u64,
    pub voice_id: u64,
    pub reference_asset_id: u64,
    pub max_frames: u32,
    pub language: &'a str,
    pub model: &'a str,
    pub text: &'a str,
}
impl Submit<'_> {
    fn validate(&self) -> Result<(), Error> {
        if self.language.len() > MAX_LANGUAGE || self.model.len() > MAX_MODEL
            || self.text.len() > MAX_TEXT { return Err(Error::TooLong); }
        if self.model.is_empty() || self.max_frames == 0 { return Err(Error::Value); }
        match self.operation {
            Operation::CreateVoice if self.reference_asset_id == 0 => Err(Error::Value),
            Operation::Synthesize if self.text.is_empty() => Err(Error::Value),
            Operation::Continue if self.text.is_empty() || self.reference_asset_id == 0 => Err(Error::Value),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status { pub state: JobState, pub progress_bps: u16, pub error_code: u16 }
impl Status {
    fn validate(&self) -> Result<(), Error> {
        if self.progress_bps > 10_000 || (self.state != JobState::Failed && self.error_code != 0) { Err(Error::Value) } else { Ok(()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultAsset<'a> {
    pub asset_id: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: u64,
    pub content_hash: [u8; 32],
    pub seed: u64,
    pub model: &'a str,
}
impl ResultAsset<'_> {
    fn validate(&self) -> Result<(), Error> {
        if self.model.len() > MAX_MODEL { return Err(Error::TooLong); }
        if self.asset_id == 0 || !(8_000..=192_000).contains(&self.sample_rate)
            || !(1..=8).contains(&self.channels) || self.frames == 0
            || self.model.is_empty() { Err(Error::Value) } else { Ok(()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message<'a> {
    Submit { job_id: u64, data: Submit<'a> },
    Cancel { job_id: u64 },
    GetStatus { job_id: u64 },
    Status { job_id: u64, data: Status },
    Result { job_id: u64, data: ResultAsset<'a> },
}
impl Message<'_> {
    pub fn job_id(&self) -> u64 {
        match self { Self::Submit { job_id, .. } | Self::Cancel { job_id }
            | Self::GetStatus { job_id } | Self::Status { job_id, .. }
            | Self::Result { job_id, .. } => *job_id }
    }
    fn kind(&self) -> u8 {
        match self { Self::Submit { .. } => 1, Self::Cancel { .. } => 2,
            Self::GetStatus { .. } => 3, Self::Status { .. } => 4, Self::Result { .. } => 5 }
    }
}

fn take<'a>(src: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if src.len() < n { return Err(Error::Short); }
    let (head, tail) = src.split_at(n);
    *src = tail;
    Ok(head)
}
fn u16_from(src: &mut &[u8]) -> Result<u16, Error> { Ok(u16::from_le_bytes(take(src, 2)?.try_into().unwrap())) }
fn u32_from(src: &mut &[u8]) -> Result<u32, Error> { Ok(u32::from_le_bytes(take(src, 4)?.try_into().unwrap())) }
fn u64_from(src: &mut &[u8]) -> Result<u64, Error> { Ok(u64::from_le_bytes(take(src, 8)?.try_into().unwrap())) }
fn utf8<'a>(src: &mut &'a [u8], n: usize, max: usize) -> Result<&'a str, Error> {
    if n > max { return Err(Error::TooLong); }
    core::str::from_utf8(take(src, n)?).map_err(|_| Error::Value)
}

/// Decode exactly one frame. Unknown versions, tags and trailing bytes are rejected.
pub fn decode(frame: &[u8]) -> Result<Message<'_>, Error> {
    if frame.len() < HEADER { return Err(Error::Short); }
    if frame.len() > MAX_FRAME { return Err(Error::TooLong); }
    if &frame[..4] != MAGIC { return Err(Error::Magic); }
    if frame[4] != VERSION { return Err(Error::Version); }
    if frame[6] != 0 || frame[7] != 0 { return Err(Error::Value); }
    let mut h = &frame[8..];
    let job_id = u64_from(&mut h)?;
    if job_id == 0 { return Err(Error::Value); }
    let mut p = &frame[HEADER..];
    let message = match frame[5] {
        1 => {
            let operation = Operation::parse(take(&mut p, 1)?[0])?;
            let reserved = take(&mut p, 3)?;
            if reserved != [0, 0, 0] { return Err(Error::Value); }
            let seed = u64_from(&mut p)?;
            let voice_id = u64_from(&mut p)?;
            let reference_asset_id = u64_from(&mut p)?;
            let max_frames = u32_from(&mut p)?;
            let language_len = take(&mut p, 1)?[0] as usize;
            let model_len = take(&mut p, 1)?[0] as usize;
            let text_len = u16_from(&mut p)? as usize;
            let language = utf8(&mut p, language_len, MAX_LANGUAGE)?;
            let model = utf8(&mut p, model_len, MAX_MODEL)?;
            let text = utf8(&mut p, text_len, MAX_TEXT)?;
            let data = Submit { operation, seed, voice_id, reference_asset_id, max_frames, language, model, text };
            data.validate()?;
            Message::Submit { job_id, data }
        }
        2 => Message::Cancel { job_id },
        3 => Message::GetStatus { job_id },
        4 => {
            let state = JobState::parse(take(&mut p, 1)?[0])?;
            let reserved = take(&mut p, 1)?[0];
            if reserved != 0 { return Err(Error::Value); }
            let data = Status { state, progress_bps: u16_from(&mut p)?, error_code: u16_from(&mut p)? };
            data.validate()?;
            Message::Status { job_id, data }
        }
        5 => {
            let asset_id = u64_from(&mut p)?;
            let sample_rate = u32_from(&mut p)?;
            let channels = take(&mut p, 1)?[0];
            let reserved = take(&mut p, 3)?;
            if reserved != [0, 0, 0] { return Err(Error::Value); }
            let frames = u64_from(&mut p)?;
            let content_hash = take(&mut p, 32)?.try_into().unwrap();
            let seed = u64_from(&mut p)?;
            let model_len = take(&mut p, 1)?[0] as usize;
            let model = utf8(&mut p, model_len, MAX_MODEL)?;
            let data = ResultAsset { asset_id, sample_rate, channels, frames, content_hash, seed, model };
            data.validate()?;
            Message::Result { job_id, data }
        }
        _ => return Err(Error::Kind),
    };
    if !p.is_empty() { return Err(Error::Trailing); }
    Ok(message)
}

struct Writer<'a> { out: &'a mut [u8], at: usize }
impl Writer<'_> {
    fn add(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.at.checked_add(bytes.len()).filter(|n| *n <= self.out.len() && *n <= MAX_FRAME).is_none() { return Err(Error::TooLong); }
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
        Ok(())
    }
    fn u8(&mut self, n: u8) -> Result<(), Error> { self.add(&[n]) }
    fn u16(&mut self, n: u16) -> Result<(), Error> { self.add(&n.to_le_bytes()) }
    fn u32(&mut self, n: u32) -> Result<(), Error> { self.add(&n.to_le_bytes()) }
    fn u64(&mut self, n: u64) -> Result<(), Error> { self.add(&n.to_le_bytes()) }
}

/// Encode one message into caller-owned memory; returns the used prefix length.
pub fn encode(message: Message<'_>, out: &mut [u8]) -> Result<usize, Error> {
    if message.job_id() == 0 { return Err(Error::Value); }
    let mut w = Writer { out, at: 0 };
    w.add(MAGIC)?;
    w.u8(VERSION)?;
    w.u8(message.kind())?;
    w.u16(0)?;
    w.u64(message.job_id())?;
    match message {
        Message::Submit { data, .. } => {
            data.validate()?;
            w.u8(data.operation as u8)?;
            w.add(&[0; 3])?;
            w.u64(data.seed)?;
            w.u64(data.voice_id)?;
            w.u64(data.reference_asset_id)?;
            w.u32(data.max_frames)?;
            w.u8(data.language.len() as u8)?;
            w.u8(data.model.len() as u8)?;
            w.u16(data.text.len() as u16)?;
            w.add(data.language.as_bytes())?;
            w.add(data.model.as_bytes())?;
            w.add(data.text.as_bytes())?;
        }
        Message::Status { data, .. } => {
            data.validate()?;
            w.u8(data.state as u8)?;
            w.u8(0)?;
            w.u16(data.progress_bps)?;
            w.u16(data.error_code)?;
        }
        Message::Result { data, .. } => {
            data.validate()?;
            w.u64(data.asset_id)?;
            w.u32(data.sample_rate)?;
            w.u8(data.channels)?;
            w.add(&[0; 3])?;
            w.u64(data.frames)?;
            w.add(&data.content_hash)?;
            w.u64(data.seed)?;
            w.u8(data.model.len() as u8)?;
            w.add(data.model.as_bytes())?;
        }
        Message::Cancel { .. } | Message::GetStatus { .. } => {}
    }
    Ok(w.at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: Message<'_>) {
        let mut out = [0; MAX_FRAME];
        let used = encode(message, &mut out).unwrap();
        assert_eq!(decode(&out[..used]).unwrap(), message);
    }

    #[test]
    fn every_message_round_trips() {
        round_trip(Message::Submit { job_id: 42, data: Submit { operation: Operation::Synthesize,
            seed: 7, voice_id: 9, reference_asset_id: 0, max_frames: 48_000,
            language: "en-US", model: "MOSS-TTS-v1.5", text: "Hello, SexOS." } });
        round_trip(Message::Cancel { job_id: 42 });
        round_trip(Message::GetStatus { job_id: 42 });
        round_trip(Message::Status { job_id: 42, data: Status { state: JobState::Running, progress_bps: 5_000, error_code: 0 } });
        round_trip(Message::Result { job_id: 42, data: ResultAsset { asset_id: 25, sample_rate: 48_000,
            channels: 1, frames: 96_000, content_hash: [0x5a; 32], seed: 7, model: "MOSS-TTS-v1.5" } });
    }

    #[test]
    fn rejects_version_trailing_and_invalid_asset() {
        let mut out = [0; MAX_FRAME];
        let n = encode(Message::Cancel { job_id: 1 }, &mut out).unwrap();
        out[4] = 2;
        assert_eq!(decode(&out[..n]), Err(Error::Version));
        out[4] = VERSION;
        assert_eq!(decode(&out[..n + 1]), Err(Error::Trailing));
        assert_eq!(encode(Message::Result { job_id: 1, data: ResultAsset { asset_id: 0,
            sample_rate: 48_000, channels: 1, frames: 1, content_hash: [0; 32], seed: 1,
            model: "m" } }, &mut out), Err(Error::Value));
    }

    #[test]
    fn bounded_text_and_continuation_reference_are_required() {
        let text = [b'x'; MAX_TEXT + 1];
        let too_long = core::str::from_utf8(&text).unwrap();
        let data = Submit { operation: Operation::Synthesize, seed: 0, voice_id: 1,
            reference_asset_id: 0, max_frames: 10, language: "en", model: "m", text: too_long };
        let mut out = [0; MAX_FRAME];
        assert_eq!(encode(Message::Submit { job_id: 1, data }, &mut out), Err(Error::TooLong));
        let missing_reference = Submit { operation: Operation::Continue, text: "hello", voice_id: 0, ..data };
        assert_eq!(encode(Message::Submit { job_id: 1, data: missing_reference }, &mut out), Err(Error::Value));
    }
}
