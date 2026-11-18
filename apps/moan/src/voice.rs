// SPDX-License-Identifier: GPL-3.0-only
//! Checked import of one immutable voice-job result into Moan's 48 kHz timeline.
use alloc::vec::Vec;
use moan_core::{Frame, RATE};
use sex_voice::ResultAsset;
use sha2::{Digest, Sha256};

pub const MODEL_MAX: usize = 96;

/// Project metadata, never a filesystem or network grant. PCM is stored in the
/// editable project as usual, so reopening never calls the model again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub job_id: u64,
    pub asset_id: u64,
    pub seed: u64,
    pub content_hash: [u8; 32],
    pub source_rate: u32,
    pub source_channels: u8,
    pub model: [u8; MODEL_MAX],
    pub model_len: u8,
}
impl Provenance {
    pub fn model(&self) -> &str {
        core::str::from_utf8(&self.model[..self.model_len as usize]).unwrap_or("")
    }
}

fn word(data: &[u8], at: usize) -> Result<u32, &'static str> {
    Ok(u32::from_le_bytes(data.get(at..at + 4).ok_or("Truncated voice WAV")?.try_into().unwrap()))
}

/// Verify the exact job asset, then decode and linearly resample PCM16 mono or
/// stereo into the bounded Moan timeline. Unsupported formats fail explicitly.
pub fn import_result(job_id: u64, result: &ResultAsset<'_>, wav: &[u8]) -> Result<(Vec<Frame>, Provenance), &'static str> {
    if job_id == 0 || result.asset_id == 0 || result.model.len() > MODEL_MAX
        || Sha256::digest(wav).as_slice() != result.content_hash { return Err("Voice asset hash mismatch"); }
    if wav.get(..4) != Some(b"RIFF") || wav.get(8..12) != Some(b"WAVE") { return Err("Voice output is not WAV"); }
    let end = 8usize.checked_add(word(wav, 4)? as usize).ok_or("Voice RIFF size overflow")?;
    if end > wav.len() { return Err("Truncated voice WAV"); }
    let (mut at, mut format, mut pcm) = (12usize, None, None);
    while at < end {
        if end - at < 8 { return Err("Truncated voice chunk"); }
        let len = word(wav, at + 4)? as usize;
        let start = at + 8;
        let finish = start.checked_add(len).filter(|n| *n <= end).ok_or("Invalid voice chunk")?;
        match &wav[at..at + 4] {
            b"fmt " => {
                if format.is_some() || len < 16 { return Err("Invalid voice format"); }
                let tag = u16::from_le_bytes(wav[start..start + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(wav[start + 2..start + 4].try_into().unwrap());
                let rate = word(wav, start + 4)?;
                let bits = u16::from_le_bytes(wav[start + 14..start + 16].try_into().unwrap());
                if tag != 1 || !(1..=2).contains(&channels) || !(8_000..=192_000).contains(&rate) || bits != 16
                    || word(wav, start + 8)? != rate * channels as u32 * 2 { return Err("Voice WAV must be PCM16 mono or stereo"); }
                format = Some((channels as usize, rate));
            }
            b"data" => { if pcm.replace(&wav[start..finish]).is_some() { return Err("Multiple voice data chunks"); } }
            _ => {}
        }
        at = finish.checked_add(len & 1).filter(|n| *n <= end).ok_or("Invalid voice chunk padding")?;
    }
    let (channels, rate) = format.ok_or("Missing voice WAV format")?;
    let data = pcm.ok_or("Missing voice WAV audio")?;
    if data.is_empty() || data.len() % (channels * 2) != 0 { return Err("Invalid voice samples"); }
    let input_frames = data.len() / (channels * 2);
    if result.sample_rate != rate || result.channels as usize != channels || result.frames != input_frames as u64 {
        return Err("Voice result metadata differs from WAV");
    }
    let frames = ((input_frames as u64) * RATE as u64).div_ceil(rate as u64) as usize;
    if frames == 0 || frames > crate::MAX_FRAMES { return Err("Voice take exceeds Moan's one-minute limit"); }
    let sample = |frame: usize, channel: usize| -> i32 {
        let at = (frame * channels + channel.min(channels - 1)) * 2;
        i16::from_le_bytes([data[at], data[at + 1]]) as i32
    };
    let mut output = Vec::new();
    output.try_reserve_exact(frames).map_err(|_| "Not enough memory for voice take")?;
    for i in 0..frames {
        let position = i as u64 * rate as u64;
        let base = (position / RATE as u64) as usize;
        let fraction = (position % RATE as u64) as i64;
        let next = (base + 1).min(input_frames - 1);
        let mut out = [0i16; 2];
        for (ch, value) in out.iter_mut().enumerate() {
            let a = sample(base.min(input_frames - 1), ch) as i64;
            let b = sample(next, ch) as i64;
            *value = ((a * (RATE as i64 - fraction) + b * fraction) / RATE as i64) as i16;
        }
        output.push(out);
    }
    let mut model = [0; MODEL_MAX];
    model[..result.model.len()].copy_from_slice(result.model.as_bytes());
    Ok((output, Provenance { job_id, asset_id: result.asset_id, seed: result.seed,
        content_hash: result.content_hash, source_rate: rate, source_channels: channels as u8,
        model, model_len: result.model.len() as u8 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    const WAV: [u8; 52] = [82,73,70,70,44,0,0,0,87,65,86,69,102,109,116,32,16,0,0,0,1,0,1,0,64,31,0,0,128,62,0,0,2,0,16,0,100,97,116,97,8,0,0,0,0,0,232,3,24,252,0,0];
    #[test]
    fn fixture_resamples_and_verifies() {
        let hash: [u8; 32] = Sha256::digest(WAV).into();
        let result = ResultAsset { asset_id: 7, sample_rate: 8000, channels: 1, frames: 4,
            content_hash: hash, seed: 42, model: "fixture-v1" };
        let (frames, provenance) = import_result(3, &result, &WAV).unwrap();
        assert_eq!(frames.len(), 24);
        assert_eq!(frames[0], [0, 0]);
        assert_eq!(provenance.model(), "fixture-v1");
        let mut altered = WAV;
        altered[44] = 1;
        assert_eq!(import_result(3, &result, &altered).err(), Some("Voice asset hash mismatch"));
    }
}
