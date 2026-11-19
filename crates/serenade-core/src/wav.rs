//! Bounds-checked RIFF/WAVE encoder and decoder supporting PCM16, PCM24,
//! IEEE Float32, cue markers (`cue ` + `LIST`/`adtl`/`labl`), and `INFO` metadata.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::convert::TryInto;

use crate::edit::{AudioClip, Marker};
use crate::SerenadeError;

/// Target audio sample encoding format for WAV files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    /// 16-bit linear signed integer PCM.
    Pcm16,
    /// 24-bit linear signed integer PCM (3 bytes per sample).
    Pcm24,
    /// 32-bit IEEE-754 floating point PCM.
    Float32,
}

/// Metadata stored in RIFF `LIST/INFO` chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WavMetadata {
    /// Title / clip name (`INAM`).
    pub title: Option<String>,
    /// Artist / author name (`IART`).
    pub artist: Option<String>,
    /// Creation date / timestamp string (`ICRD`).
    pub date: Option<String>,
    /// Comment / remarks (`ICMT`).
    pub comment: Option<String>,
    /// Genre classification (`IGNR`).
    pub genre: Option<String>,
    /// Software encoder tag (`ISFT`).
    pub software: Option<String>,
}

/// Decoded WAV container containing audio clip, format, and metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedWav {
    /// Audio clip containing PCM samples and markers.
    pub clip: AudioClip,
    /// Format found in WAV header.
    pub format: WavFormat,
    /// Metadata tags recovered from `LIST/INFO`.
    pub metadata: WavMetadata,
}

/// Specific error types encountered while decoding WAV byte streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavError {
    /// Header does not start with `"RIFF"` and `"WAVE"`.
    InvalidRiffHeader,
    /// Format is not supported (only PCM 16/24 and IEEE Float 32).
    UnsupportedFormat(&'static str),
    /// Missing mandatory `"fmt "` chunk before audio data.
    MissingFmtChunk,
    /// Missing mandatory `"data"` chunk.
    MissingDataChunk,
    /// Unexpected end of input or truncated chunk payload.
    TruncatedData,
    /// Arithmetic overflow in chunk pointer or size calculations.
    IntegerOverflow,
    /// Sample rate is zero.
    InvalidSampleRate,
    /// Channel count is unsupported (must be 1 or 2).
    InvalidChannelCount,
    /// Memory allocation failure during decode.
    AllocationFailed,
}

impl From<WavError> for SerenadeError {
    fn from(e: WavError) -> Self {
        SerenadeError::Wav(e)
    }
}

/// Encodes an `AudioClip` into a standard RIFF/WAVE byte buffer.
pub fn encode_wav(
    clip: &AudioClip,
    format: WavFormat,
    metadata: &WavMetadata,
) -> Result<Vec<u8>, SerenadeError> {
    let ch = clip.channels;
    if ch != 1 && ch != 2 {
        return Err(SerenadeError::Wav(WavError::InvalidChannelCount));
    }
    if clip.sample_rate == 0 {
        return Err(SerenadeError::Wav(WavError::InvalidSampleRate));
    }

    let (audio_format, bits_per_sample) = match format {
        WavFormat::Pcm16 => (1u16, 16u16),
        WavFormat::Pcm24 => (1u16, 24u16),
        WavFormat::Float32 => (3u16, 32u16),
    };

    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let block_align = ch * (bits_per_sample / 8);
    let byte_rate = clip
        .sample_rate
        .checked_mul(block_align as u32)
        .ok_or(WavError::IntegerOverflow)?;
    let num_samples = clip.samples.len();

    let data_bytes_len = num_samples
        .checked_mul(bytes_per_sample)
        .ok_or(WavError::IntegerOverflow)?;

    // Estimated capacity for RIFF container
    let mut out = Vec::new();
    let estimated_total = data_bytes_len
        .checked_add(1024)
        .ok_or(WavError::IntegerOverflow)?;
    out.try_reserve(estimated_total)
        .map_err(|_| SerenadeError::AllocationFailed)?;

    // 1. RIFF Header placeholder (we write length at the end)
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // placeholder
    out.extend_from_slice(b"WAVE");

    // 2. fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk size = 16
    out.extend_from_slice(&audio_format.to_le_bytes());
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&clip.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());

    // 3. data chunk
    out.extend_from_slice(b"data");
    let data_len_u32: u32 = data_bytes_len
        .try_into()
        .map_err(|_| SerenadeError::Wav(WavError::IntegerOverflow))?;
    out.extend_from_slice(&data_len_u32.to_le_bytes());

    match format {
        WavFormat::Pcm16 => {
            for &s in &clip.samples {
                let clamped = if s < -1.0 {
                    -1.0
                } else if s > 1.0 {
                    1.0
                } else {
                    s
                };
                let val = (clamped * 32767.0) as i16;
                out.extend_from_slice(&val.to_le_bytes());
            }
        }
        WavFormat::Pcm24 => {
            for &s in &clip.samples {
                let clamped = if s < -1.0 {
                    -1.0
                } else if s > 1.0 {
                    1.0
                } else {
                    s
                };
                let val = (clamped * 8388607.0) as i32;
                let b0 = val as u8;
                let b1 = (val >> 8) as u8;
                let b2 = (val >> 16) as u8;
                out.push(b0);
                out.push(b1);
                out.push(b2);
            }
        }
        WavFormat::Float32 => {
            for &s in &clip.samples {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
    }

    // Pad data chunk to even boundary if needed
    if data_bytes_len % 2 != 0 {
        out.push(0);
    }

    // 4. cue chunk (if markers present)
    if !clip.markers.is_empty() {
        out.extend_from_slice(b"cue ");
        let cue_count = clip.markers.len() as u32;
        let cue_chunk_size = 4 + cue_count * 24;
        out.extend_from_slice(&cue_chunk_size.to_le_bytes());
        out.extend_from_slice(&cue_count.to_le_bytes());

        for m in &clip.markers {
            let id = m.id;
            let pos = m.frame_index as u32;
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&pos.to_le_bytes());
            out.extend_from_slice(b"data");
            out.extend_from_slice(&0u32.to_le_bytes()); // chunk start
            out.extend_from_slice(&0u32.to_le_bytes()); // block start
            out.extend_from_slice(&pos.to_le_bytes()); // sample offset
        }

        // 5. LIST / adtl chunk for marker text labels
        let mut adtl_payload = Vec::new();
        adtl_payload.extend_from_slice(b"adtl");
        for m in &clip.markers {
            adtl_payload.extend_from_slice(b"labl");
            let name_bytes = m.name.as_bytes();
            let label_len = 4 + name_bytes.len() + 1; // dwName (4) + text + null
            let label_len_u32 = label_len as u32;
            adtl_payload.extend_from_slice(&label_len_u32.to_le_bytes());
            adtl_payload.extend_from_slice(&m.id.to_le_bytes());
            adtl_payload.extend_from_slice(name_bytes);
            adtl_payload.push(0); // null terminator
            if label_len % 2 != 0 {
                adtl_payload.push(0); // padding
            }
        }

        out.extend_from_slice(b"LIST");
        let adtl_len_u32 = adtl_payload.len() as u32;
        out.extend_from_slice(&adtl_len_u32.to_le_bytes());
        out.extend_from_slice(&adtl_payload);
        if adtl_payload.len() % 2 != 0 {
            out.push(0);
        }
    }

    // 6. LIST / INFO chunk for metadata
    let mut info_payload = Vec::new();
    let has_info = metadata.title.is_some()
        || metadata.artist.is_some()
        || metadata.date.is_some()
        || metadata.comment.is_some()
        || metadata.genre.is_some()
        || metadata.software.is_some();

    if has_info {
        info_payload.extend_from_slice(b"INFO");
        let write_info_field = |tag: &[u8; 4], text: &str, target: &mut Vec<u8>| {
            target.extend_from_slice(tag);
            let bytes = text.as_bytes();
            let len = bytes.len() + 1; // text + null
            target.extend_from_slice(&(len as u32).to_le_bytes());
            target.extend_from_slice(bytes);
            target.push(0);
            if len % 2 != 0 {
                target.push(0);
            }
        };

        if let Some(ref t) = metadata.title {
            write_info_field(b"INAM", t, &mut info_payload);
        }
        if let Some(ref a) = metadata.artist {
            write_info_field(b"IART", a, &mut info_payload);
        }
        if let Some(ref d) = metadata.date {
            write_info_field(b"ICRD", d, &mut info_payload);
        }
        if let Some(ref c) = metadata.comment {
            write_info_field(b"ICMT", c, &mut info_payload);
        }
        if let Some(ref g) = metadata.genre {
            write_info_field(b"IGNR", g, &mut info_payload);
        }
        let soft = metadata
            .software
            .as_deref()
            .unwrap_or("SexOS Serenade 1.0");
        write_info_field(b"ISFT", soft, &mut info_payload);

        out.extend_from_slice(b"LIST");
        let info_len_u32 = info_payload.len() as u32;
        out.extend_from_slice(&info_len_u32.to_le_bytes());
        out.extend_from_slice(&info_payload);
        if info_payload.len() % 2 != 0 {
            out.push(0);
        }
    }

    // Finalize RIFF file length (total bytes - 8)
    let total_len = out.len();
    let riff_size_u32 = (total_len - 8) as u32;
    out[4..8].copy_from_slice(&riff_size_u32.to_le_bytes());

    Ok(out)
}

/// Maximum number of samples allowed to be decoded into memory in a single WAV file (10 minutes stereo 48 kHz).
pub const MAX_WAV_DECODE_SAMPLES: usize = 57_600_000;

/// Decodes a byte buffer containing a standard RIFF/WAVE container.
pub fn decode_wav(bytes: &[u8]) -> Result<DecodedWav, SerenadeError> {
    if bytes.len() < 12 {
        return Err(SerenadeError::Wav(WavError::InvalidRiffHeader));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(SerenadeError::Wav(WavError::InvalidRiffHeader));
    }

    let mut fmt_info: Option<(u16, u16, u32, u16)> = None; // (format, channels, sample_rate, bits_per_sample)
    let mut raw_samples_slice: Option<&[u8]> = None;
    let mut raw_cue_points: Vec<(u32, u32)> = Vec::new(); // (id, frame_offset)
    let mut marker_labels: Vec<(u32, String)> = Vec::new();
    let mut metadata = WavMetadata::default();

    let mut offset = 12usize;
    while let Some(chunk_header_end) = offset.checked_add(8) {
        if chunk_header_end > bytes.len() {
            break;
        }
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(bytes[offset + 4..chunk_header_end].try_into().unwrap()) as usize;
        let payload_start = chunk_header_end;

        let payload_end = payload_start
            .checked_add(chunk_size)
            .ok_or(WavError::IntegerOverflow)?;

        if payload_end > bytes.len() {
            return Err(SerenadeError::Wav(WavError::TruncatedData));
        }

        let chunk_data = &bytes[payload_start..payload_end];

        if chunk_id == b"fmt " {
            if chunk_data.len() < 16 {
                return Err(SerenadeError::Wav(WavError::TruncatedData));
            }
            let audio_format = u16::from_le_bytes(chunk_data[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(chunk_data[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(chunk_data[4..8].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(chunk_data[14..16].try_into().unwrap());

            if channels != 1 && channels != 2 {
                return Err(SerenadeError::Wav(WavError::InvalidChannelCount));
            }
            if sample_rate == 0 {
                return Err(SerenadeError::Wav(WavError::InvalidSampleRate));
            }

            fmt_info = Some((audio_format, channels, sample_rate, bits_per_sample));
        } else if chunk_id == b"data" {
            raw_samples_slice = Some(chunk_data);
        } else if chunk_id == b"cue " {
            if chunk_data.len() >= 4 {
                let cue_count = u32::from_le_bytes(chunk_data[0..4].try_into().unwrap()) as usize;
                let mut cue_off = 4usize;
                for _ in 0..cue_count {
                    if cue_off + 24 > chunk_data.len() {
                        break;
                    }
                    let id = u32::from_le_bytes(chunk_data[cue_off..cue_off + 4].try_into().unwrap());
                    let sample_off = u32::from_le_bytes(
                        chunk_data[cue_off + 20..cue_off + 24]
                            .try_into()
                            .unwrap(),
                    );
                    raw_cue_points.push((id, sample_off));
                    cue_off += 24;
                }
            }
        } else if chunk_id == b"LIST" {
            if chunk_data.len() >= 4 {
                let list_type = &chunk_data[0..4];
                if list_type == b"adtl" {
                    // Associated Data List containing labels
                    let mut sub_off = 4usize;
                    while let Some(sub_header_end) = sub_off.checked_add(8) {
                        if sub_header_end > chunk_data.len() {
                            break;
                        }
                        let sub_id = &chunk_data[sub_off..sub_off + 4];
                        let sub_size = u32::from_le_bytes(
                            chunk_data[sub_off + 4..sub_header_end].try_into().unwrap(),
                        ) as usize;
                        let sub_payload_start = sub_header_end;
                        let sub_payload_end = sub_payload_start
                            .checked_add(sub_size)
                            .ok_or(WavError::IntegerOverflow)?;

                        if sub_payload_end > chunk_data.len() {
                            break;
                        }

                        let sub_payload = &chunk_data[sub_payload_start..sub_payload_end];
                        if sub_id == b"labl" && sub_payload.len() >= 4 {
                            let cue_id = u32::from_le_bytes(sub_payload[0..4].try_into().unwrap());
                            let mut text_bytes = &sub_payload[4..];
                            // Strip trailing null if present
                            if let Some(pos) = text_bytes.iter().position(|&b| b == 0) {
                                text_bytes = &text_bytes[..pos];
                            }
                            let label = String::from_utf8_lossy(text_bytes).to_string();
                            marker_labels.push((cue_id, label));
                        }

                        let padding = sub_size & 1;
                        let padded_sub = sub_size
                            .checked_add(padding)
                            .ok_or(WavError::IntegerOverflow)?;
                        sub_off = sub_payload_start
                            .checked_add(padded_sub)
                            .ok_or(WavError::IntegerOverflow)?;
                    }
                } else if list_type == b"INFO" {
                    let mut sub_off = 4usize;
                    while let Some(sub_header_end) = sub_off.checked_add(8) {
                        if sub_header_end > chunk_data.len() {
                            break;
                        }
                        let sub_id = &chunk_data[sub_off..sub_off + 4];
                        let sub_size = u32::from_le_bytes(
                            chunk_data[sub_off + 4..sub_header_end].try_into().unwrap(),
                        ) as usize;
                        let sub_payload_start = sub_header_end;
                        let sub_payload_end = sub_payload_start
                            .checked_add(sub_size)
                            .ok_or(WavError::IntegerOverflow)?;

                        if sub_payload_end > chunk_data.len() {
                            break;
                        }

                        let mut text_bytes = &chunk_data[sub_payload_start..sub_payload_end];
                        if let Some(pos) = text_bytes.iter().position(|&b| b == 0) {
                            text_bytes = &text_bytes[..pos];
                        }
                        let text = String::from_utf8_lossy(text_bytes).to_string();

                        match sub_id {
                            b"INAM" => metadata.title = Some(text),
                            b"IART" => metadata.artist = Some(text),
                            b"ICRD" => metadata.date = Some(text),
                            b"ICMT" => metadata.comment = Some(text),
                            b"IGNR" => metadata.genre = Some(text),
                            b"ISFT" => metadata.software = Some(text),
                            _ => {}
                        }

                        let padding = sub_size & 1;
                        let padded_sub = sub_size
                            .checked_add(padding)
                            .ok_or(WavError::IntegerOverflow)?;
                        sub_off = sub_payload_start
                            .checked_add(padded_sub)
                            .ok_or(WavError::IntegerOverflow)?;
                    }
                }
            }
        }

        // Advance to next chunk (aligned to 2-byte boundary)
        let padding = chunk_size & 1;
        let padded_size = chunk_size
            .checked_add(padding)
            .ok_or(WavError::IntegerOverflow)?;

        offset = payload_start
            .checked_add(padded_size)
            .ok_or(WavError::IntegerOverflow)?;
    }

    let (format_tag, channels, sample_rate, bits_per_sample) =
        fmt_info.ok_or(SerenadeError::Wav(WavError::MissingFmtChunk))?;
    let raw_data = raw_samples_slice.ok_or(SerenadeError::Wav(WavError::MissingDataChunk))?;

    let wav_format = match (format_tag, bits_per_sample) {
        (1, 16) => WavFormat::Pcm16,
        (1, 24) => WavFormat::Pcm24,
        (3, 32) => WavFormat::Float32,
        _ => return Err(SerenadeError::Wav(WavError::UnsupportedFormat("only PCM 16/24 and IEEE Float 32 supported"))),
    };

    let bytes_per_sample = (bits_per_sample / 8) as usize;
    if bytes_per_sample == 0 {
        return Err(SerenadeError::Wav(WavError::UnsupportedFormat("bits per sample must be >= 8")));
    }
    let block_align = (channels as usize)
        .checked_mul(bytes_per_sample)
        .ok_or(WavError::IntegerOverflow)?;
    if block_align == 0 {
        return Err(SerenadeError::Wav(WavError::InvalidChannelCount));
    }
    let frame_count = raw_data.len() / block_align;
    let total_samples = frame_count
        .checked_mul(channels as usize)
        .ok_or(WavError::IntegerOverflow)?;

    if total_samples > MAX_WAV_DECODE_SAMPLES {
        return Err(SerenadeError::BufferFull {
            current_samples: 0,
            max_samples: MAX_WAV_DECODE_SAMPLES,
        });
    }

    let mut samples = Vec::new();
    samples
        .try_reserve_exact(total_samples)
        .map_err(|_| SerenadeError::AllocationFailed)?;

    match wav_format {
        WavFormat::Pcm16 => {
            let mut i = 0;
            while i + 2 <= raw_data.len() && samples.len() < total_samples {
                let v = i16::from_le_bytes(raw_data[i..i + 2].try_into().unwrap());
                samples.push(v as f32 / 32768.0);
                i += 2;
            }
        }
        WavFormat::Pcm24 => {
            let mut i = 0;
            while i + 3 <= raw_data.len() && samples.len() < total_samples {
                let b0 = raw_data[i] as i32;
                let b1 = (raw_data[i + 1] as i32) << 8;
                let b2 = (raw_data[i + 2] as i8 as i32) << 16;
                let val = b0 | b1 | b2;
                samples.push(val as f32 / 8388608.0);
                i += 3;
            }
        }
        WavFormat::Float32 => {
            let mut i = 0;
            while i + 4 <= raw_data.len() && samples.len() < total_samples {
                let val = f32::from_le_bytes(raw_data[i..i + 4].try_into().unwrap());
                samples.push(val);
                i += 4;
            }
        }
    }

    // Reconstruct markers from cue points and labels
    let mut markers = Vec::new();
    for (id, frame_idx) in raw_cue_points {
        let name = marker_labels
            .iter()
            .find(|(lbl_id, _)| *lbl_id == id)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| {
                let mut s = String::from("Marker ");
                let mut n = id;
                let mut digits = [0u8; 10];
                let mut d_idx = 0;
                if n == 0 {
                    s.push('0');
                } else {
                    while n > 0 {
                        digits[d_idx] = b'0' + (n % 10) as u8;
                        n /= 10;
                        d_idx += 1;
                    }
                    while d_idx > 0 {
                        d_idx -= 1;
                        s.push(digits[d_idx] as char);
                    }
                }
                s
            });

        let ts_ms = (frame_idx as u64 * 1000) / (sample_rate as u64);
        markers.push(Marker {
            id,
            name,
            frame_index: frame_idx as u64,
            timestamp_ms: ts_ms,
        });
    }

    let clip = AudioClip::new(channels, sample_rate, samples, markers)?;

    Ok(DecodedWav {
        clip,
        format: wav_format,
        metadata,
    })
}
