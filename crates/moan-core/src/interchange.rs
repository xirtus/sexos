// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust Timeline Interchange (`InterchangeTimeline`) for OpenTimelineIO (`.otio` JSON),
//! CMX 3600 `.edl`, and AAF-XML summary round-trip delivery across Moan, Splicy, and Cabaret.
//! Preserves clip trims, fades, gain, markers, loudness metadata, and reference video sync offset.
//! Pure `#![no_std]` + `alloc`, zero `unsafe`, zero external crates.

use crate::{dsp_math::round_f32, Track, RATE};
use alloc::{
    format,
    string::String,
    vec::Vec,
};

const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 32;
const MAX_TRACKS: usize = 256;
const MAX_CLIPS_PER_TRACK: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterchangeError {
    InputTooLarge,
    InvalidJson,
    InvalidOtioSchema,
    InvalidEdl,
    InvalidXml,
    InvalidParameter,
    NestingTooDeep,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InterchangeLoudness {
    pub integrated_lufs: Option<f32>,
    pub true_peak_dbtp: Option<f32>,
    pub loudness_range_lu: Option<f32>,
    pub target_lufs: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterchangeMarker {
    pub name: String,
    pub start_frame: u64,
    pub duration_frames: u64,
    pub color: String,
    pub comment: String,
}

impl InterchangeMarker {
    pub fn new(name: &str, start_frame: u64) -> Self {
        Self {
            name: String::from(name),
            start_frame,
            duration_frames: 0,
            color: String::from("GREEN"),
            comment: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterchangeClip {
    pub name: String,
    pub media_url: String,
    pub timeline_start_frame: u64,
    pub source_start_frame: u64,
    pub duration_frames: u64,
    pub fade_in_frames: u64,
    pub fade_out_frames: u64,
    pub gain_percent: u16,
    pub muted: bool,
    pub markers: Vec<InterchangeMarker>,
}

impl InterchangeClip {
    pub fn new(
        name: &str,
        media_url: &str,
        timeline_start_frame: u64,
        source_start_frame: u64,
        duration_frames: u64,
    ) -> Self {
        Self {
            name: String::from(name),
            media_url: String::from(media_url),
            timeline_start_frame,
            source_start_frame,
            duration_frames,
            fade_in_frames: 0,
            fade_out_frames: 0,
            gain_percent: 100,
            muted: false,
            markers: Vec::new(),
        }
    }

    pub fn timeline_end_frame(&self) -> u64 {
        self.timeline_start_frame.saturating_add(self.duration_frames)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterchangeTrackKind {
    Audio,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterchangeTrack {
    pub name: String,
    pub kind: InterchangeTrackKind,
    pub volume_percent: u16,
    pub pan: i16,
    pub muted: bool,
    pub solo: bool,
    pub clips: Vec<InterchangeClip>,
    pub markers: Vec<InterchangeMarker>,
}

impl InterchangeTrack {
    pub fn audio(name: &str) -> Self {
        Self {
            name: String::from(name),
            kind: InterchangeTrackKind::Audio,
            volume_percent: 100,
            pan: 0,
            muted: false,
            solo: false,
            clips: Vec::new(),
            markers: Vec::new(),
        }
    }

    pub fn video(name: &str) -> Self {
        Self {
            name: String::from(name),
            kind: InterchangeTrackKind::Video,
            volume_percent: 100,
            pan: 0,
            muted: false,
            solo: false,
            clips: Vec::new(),
            markers: Vec::new(),
        }
    }
}

/// Cross-app timeline interchange model bridging Moan, Splicy, and Cabaret.
#[derive(Clone, Debug, PartialEq)]
pub struct InterchangeTimeline {
    pub name: String,
    pub sample_rate: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// Offset in audio frames relative to reference video timecode zero (`+` delays audio).
    pub video_sync_offset_frames: i64,
    pub loudness: InterchangeLoudness,
    pub tracks: Vec<InterchangeTrack>,
    pub markers: Vec<InterchangeMarker>,
}

impl InterchangeTimeline {
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            sample_rate: RATE,
            fps_num: 30,
            fps_den: 1,
            video_sync_offset_frames: 0,
            loudness: InterchangeLoudness::default(),
            tracks: Vec::new(),
            markers: Vec::new(),
        }
    }

    /// Video sync offset in milliseconds at `sample_rate`.
    pub fn video_sync_offset_ms(&self) -> f32 {
        self.video_sync_offset_frames as f32 * 1000.0 / (self.sample_rate.max(1) as f32)
    }

    /// Builds an [`InterchangeTimeline`] snapshot from Moan [`Track`]s, preserving trims, fades,
    /// gain, volume, pan, mute, and solo metadata.
    pub fn from_tracks(name: &str, tracks: &[Track<'_>]) -> Self {
        let mut tl = Self::new(name);
        for (ti, track) in tracks.iter().enumerate() {
            let mut it = InterchangeTrack::audio(&format!("Audio {}", ti + 1));
            it.volume_percent = track.volume();
            it.pan = track.pan();
            it.muted = track.muted;
            it.solo = track.solo;
            for (ci, clip) in track.clips.iter().enumerate() {
                let (fi, fo) = clip.fades();
                let cname = if clip.vocal_clip().is_some() {
                    format!("vocal_{}_{}", ti + 1, ci + 1)
                } else {
                    format!("clip_{}_{}", ti + 1, ci + 1)
                };
                let url = if clip.midi_pattern().is_some() {
                    format!("moan://midi/track{}/clip{}", ti + 1, ci + 1)
                } else if clip.vocal_clip().is_some() {
                    format!("moan://vocal/track{}/clip{}", ti + 1, ci + 1)
                } else {
                    format!("moan://pcm/track{}/clip{}.wav", ti + 1, ci + 1)
                };
                let mut ic = InterchangeClip::new(
                    &cname,
                    &url,
                    clip.start() as u64,
                    clip.source_offset() as u64,
                    clip.len() as u64,
                );
                ic.fade_in_frames = fi as u64;
                ic.fade_out_frames = fo as u64;
                ic.gain_percent = clip.gain();
                it.clips.push(ic);
            }
            tl.tracks.push(it);
        }
        tl
    }

    // -------------------------------------------------------------------------
    // OpenTimelineIO (.otio JSON) Export & Import
    // -------------------------------------------------------------------------

    pub fn to_otio_json(&self) -> String {
        let sr = self.sample_rate.max(1);
        let mut tracks_json = String::new();

        for (ti, track) in self.tracks.iter().enumerate() {
            if ti > 0 {
                tracks_json.push(',');
            }
            let mut sorted = track.clips.clone();
            sorted.sort_by_key(|c| c.timeline_start_frame);

            let mut children = String::new();
            let mut cursor = 0u64;
            let mut first_child = true;

            for clip in &sorted {
                if clip.timeline_start_frame > cursor {
                    let gap_dur = clip.timeline_start_frame - cursor;
                    if !first_child {
                        children.push(',');
                    }
                    first_child = false;
                    children.push_str(&format!(
                        "{{\"OTIO_SCHEMA\":\"Gap.1\",\"name\":\"\",\"source_range\":{}}}",
                        otio_time_range(0, gap_dur, sr)
                    ));
                }
                if !first_child {
                    children.push(',');
                }
                first_child = false;

                let mut markers_json = String::new();
                for (mi, m) in clip.markers.iter().enumerate() {
                    if mi > 0 {
                        markers_json.push(',');
                    }
                    markers_json.push_str(&otio_marker_json(m, sr));
                }

                let mut effects_json = String::new();
                let mut has_eff = false;
                if clip.fade_in_frames > 0 {
                    effects_json.push_str(&format!(
                        "{{\"OTIO_SCHEMA\":\"Effect.1\",\"name\":\"FadeIn\",\"effect_name\":\"FadeIn\",\"metadata\":{{\"frames\":{}}}}}",
                        clip.fade_in_frames
                    ));
                    has_eff = true;
                }
                if clip.fade_out_frames > 0 {
                    if has_eff {
                        effects_json.push(',');
                    }
                    effects_json.push_str(&format!(
                        "{{\"OTIO_SCHEMA\":\"Effect.1\",\"name\":\"FadeOut\",\"effect_name\":\"FadeOut\",\"metadata\":{{\"frames\":{}}}}}",
                        clip.fade_out_frames
                    ));
                }

                children.push_str(&format!(
                    "{{\"OTIO_SCHEMA\":\"Clip.2\",\"name\":\"{}\",\"source_range\":{},\"media_references\":{{\"DEFAULT_MEDIA\":{{\"OTIO_SCHEMA\":\"ExternalReference.1\",\"target_url\":\"{}\"}}}},\"active_media_reference_key\":\"DEFAULT_MEDIA\",\"effects\":[{}],\"markers\":[{}],\"metadata\":{{\"moan_interchange\":{{\"timeline_start_frame\":{},\"fade_in_frames\":{},\"fade_out_frames\":{},\"gain_percent\":{},\"muted\":{}}}}}}}",
                    json_escape(&clip.name),
                    otio_time_range(clip.source_start_frame, clip.duration_frames, sr),
                    json_escape(&clip.media_url),
                    effects_json,
                    markers_json,
                    clip.timeline_start_frame,
                    clip.fade_in_frames,
                    clip.fade_out_frames,
                    clip.gain_percent,
                    clip.muted
                ));
                cursor = clip
                    .timeline_start_frame
                    .saturating_add(clip.duration_frames);
            }

            let mut trk_markers = String::new();
            for (mi, m) in track.markers.iter().enumerate() {
                if mi > 0 {
                    trk_markers.push(',');
                }
                trk_markers.push_str(&otio_marker_json(m, sr));
            }
            let kind = match track.kind {
                InterchangeTrackKind::Audio => "Audio",
                InterchangeTrackKind::Video => "Video",
            };
            tracks_json.push_str(&format!(
                "{{\"OTIO_SCHEMA\":\"Track.1\",\"name\":\"{}\",\"kind\":\"{}\",\"children\":[{}],\"markers\":[{}],\"metadata\":{{\"moan_interchange\":{{\"volume_percent\":{},\"pan\":{},\"muted\":{},\"solo\":{}}}}}}}",
                json_escape(&track.name),
                kind,
                children,
                trk_markers,
                track.volume_percent,
                track.pan,
                track.muted,
                track.solo
            ));
        }

        let mut stack_markers = String::new();
        for (mi, m) in self.markers.iter().enumerate() {
            if mi > 0 {
                stack_markers.push(',');
            }
            stack_markers.push_str(&otio_marker_json(m, sr));
        }

        let loudness_json = format!(
            "{{\"integrated_lufs\":{},\"true_peak_dbtp\":{},\"loudness_range_lu\":{},\"target_lufs\":{}}}",
            json_opt_f32(self.loudness.integrated_lufs),
            json_opt_f32(self.loudness.true_peak_dbtp),
            json_opt_f32(self.loudness.loudness_range_lu),
            json_opt_f32(self.loudness.target_lufs)
        );

        format!(
            "{{\"OTIO_SCHEMA\":\"Timeline.1\",\"name\":\"{}\",\"global_start_time\":{{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":{}.0,\"value\":{}.0}},\"metadata\":{{\"moan_interchange\":{{\"sample_rate\":{},\"fps_num\":{},\"fps_den\":{},\"video_sync_offset_frames\":{},\"loudness\":{}}}}},\"tracks\":{{\"OTIO_SCHEMA\":\"Stack.1\",\"name\":\"tracks\",\"children\":[{}],\"markers\":[{}]}}}}",
            json_escape(&self.name),
            sr,
            self.video_sync_offset_frames,
            sr,
            self.fps_num.max(1),
            self.fps_den.max(1),
            self.video_sync_offset_frames,
            loudness_json,
            tracks_json,
            stack_markers
        )
    }

    pub fn from_otio_json(json: &str) -> Result<Self, InterchangeError> {
        let root = parse_json(json)?;
        let obj = root.as_object().ok_or(InterchangeError::InvalidOtioSchema)?;
        if let Some(schema) = json_str(obj, "OTIO_SCHEMA") {
            if !schema.starts_with("Timeline") {
                return Err(InterchangeError::InvalidOtioSchema);
            }
        }
        let name = json_str(obj, "name").unwrap_or("Untitled");
        let mut tl = Self::new(name);

        if let Some(meta) = json_obj(obj, "metadata").and_then(|m| json_obj(m, "moan_interchange"))
        {
            if let Some(sr) = json_f64(meta, "sample_rate") {
                if sr >= 8000.0 && sr <= 192_000.0 {
                    tl.sample_rate = sr as u32;
                }
            }
            if let Some(fn_) = json_f64(meta, "fps_num") {
                if fn_ >= 1.0 && fn_ <= 240_000.0 {
                    tl.fps_num = fn_ as u32;
                }
            }
            if let Some(fd) = json_f64(meta, "fps_den") {
                if fd >= 1.0 && fd <= 10_000.0 {
                    tl.fps_den = fd as u32;
                }
            }
            if let Some(off) = json_f64(meta, "video_sync_offset_frames") {
                tl.video_sync_offset_frames = off as i64;
            }
            if let Some(lobj) = json_obj(meta, "loudness") {
                tl.loudness = InterchangeLoudness {
                    integrated_lufs: json_f64(lobj, "integrated_lufs").map(|v| v as f32),
                    true_peak_dbtp: json_f64(lobj, "true_peak_dbtp").map(|v| v as f32),
                    loudness_range_lu: json_f64(lobj, "loudness_range_lu").map(|v| v as f32),
                    target_lufs: json_f64(lobj, "target_lufs").map(|v| v as f32),
                };
            }
        } else if let Some(gst) = json_obj(obj, "global_start_time") {
            if let Some((val, rate)) = parse_rational_time(gst) {
                tl.video_sync_offset_frames =
                    rational_to_signed_frames(val, rate, tl.sample_rate);
            }
        }

        let stack = json_obj(obj, "tracks").ok_or(InterchangeError::InvalidOtioSchema)?;
        if let Some(markers) = json_arr(stack, "markers") {
            for mv in markers {
                if let Some(m) = parse_otio_marker(mv, tl.sample_rate) {
                    tl.markers.push(m);
                }
            }
        }

        let track_vals = json_arr(stack, "children").ok_or(InterchangeError::InvalidOtioSchema)?;
        if track_vals.len() > MAX_TRACKS {
            return Err(InterchangeError::InvalidParameter);
        }

        for tv in track_vals {
            let tobj = tv.as_object().ok_or(InterchangeError::InvalidOtioSchema)?;
            let tname = json_str(tobj, "name").unwrap_or("Track");
            let kind = match json_str(tobj, "kind").unwrap_or("Audio") {
                "Video" | "video" => InterchangeTrackKind::Video,
                _ => InterchangeTrackKind::Audio,
            };
            let mut track = InterchangeTrack {
                name: String::from(tname),
                kind,
                volume_percent: 100,
                pan: 0,
                muted: false,
                solo: false,
                clips: Vec::new(),
                markers: Vec::new(),
            };
            if let Some(tmeta) =
                json_obj(tobj, "metadata").and_then(|m| json_obj(m, "moan_interchange"))
            {
                if let Some(v) = json_f64(tmeta, "volume_percent") {
                    track.volume_percent = (v as i64).clamp(0, 400) as u16;
                }
                if let Some(p) = json_f64(tmeta, "pan") {
                    track.pan = (p as i64).clamp(-100, 100) as i16;
                }
                if let Some(m) = json_bool(tmeta, "muted") {
                    track.muted = m;
                }
                if let Some(s) = json_bool(tmeta, "solo") {
                    track.solo = s;
                }
            }
            if let Some(markers) = json_arr(tobj, "markers") {
                for mv in markers {
                    if let Some(m) = parse_otio_marker(mv, tl.sample_rate) {
                        track.markers.push(m);
                    }
                }
            }

            let mut cursor = 0u64;
            if let Some(children) = json_arr(tobj, "children") {
                if children.len() > MAX_CLIPS_PER_TRACK {
                    return Err(InterchangeError::InvalidParameter);
                }
                for cv in children {
                    let cobj = cv.as_object().ok_or(InterchangeError::InvalidOtioSchema)?;
                    let schema = json_str(cobj, "OTIO_SCHEMA").unwrap_or("");
                    let (src_start, dur) = json_obj(cobj, "source_range")
                        .map(|sr| parse_time_range(sr, tl.sample_rate))
                        .unwrap_or((0, 0));

                    if schema.starts_with("Gap") {
                        cursor = cursor.saturating_add(dur);
                        continue;
                    }
                    if schema.starts_with("Clip") || schema.is_empty() {
                        let cname = json_str(cobj, "name").unwrap_or("Clip");
                        let url = extract_media_url(cobj).unwrap_or("");
                        let mut tl_start = cursor;
                        let mut fade_in = 0u64;
                        let mut fade_out = 0u64;
                        let mut gain = 100u16;
                        let mut muted = false;

                        if let Some(effects) = json_arr(cobj, "effects") {
                            for ev in effects {
                                if let Some(eo) = ev.as_object() {
                                    let ename = json_str(eo, "effect_name")
                                        .or_else(|| json_str(eo, "name"))
                                        .unwrap_or("");
                                    let frames = json_obj(eo, "metadata")
                                        .and_then(|m| json_f64(m, "frames"))
                                        .map(|f| f.max(0.0) as u64)
                                        .unwrap_or(0);
                                    if ename == "FadeIn" {
                                        fade_in = frames;
                                    } else if ename == "FadeOut" {
                                        fade_out = frames;
                                    }
                                }
                            }
                        }

                        if let Some(cmeta) =
                            json_obj(cobj, "metadata").and_then(|m| json_obj(m, "moan_interchange"))
                        {
                            if let Some(ts) = json_f64(cmeta, "timeline_start_frame") {
                                tl_start = ts.max(0.0) as u64;
                            }
                            if let Some(fi) = json_f64(cmeta, "fade_in_frames") {
                                fade_in = fi.max(0.0) as u64;
                            }
                            if let Some(fo) = json_f64(cmeta, "fade_out_frames") {
                                fade_out = fo.max(0.0) as u64;
                            }
                            if let Some(g) = json_f64(cmeta, "gain_percent") {
                                gain = (g as i64).clamp(0, 400) as u16;
                            }
                            if let Some(m) = json_bool(cmeta, "muted") {
                                muted = m;
                            }
                        }

                        let mut markers = Vec::new();
                        if let Some(marr) = json_arr(cobj, "markers") {
                            for mv in marr {
                                if let Some(m) = parse_otio_marker(mv, tl.sample_rate) {
                                    markers.push(m);
                                }
                            }
                        }

                        track.clips.push(InterchangeClip {
                            name: String::from(cname),
                            media_url: String::from(url),
                            timeline_start_frame: tl_start,
                            source_start_frame: src_start,
                            duration_frames: dur,
                            fade_in_frames: fade_in,
                            fade_out_frames: fade_out,
                            gain_percent: gain,
                            muted,
                            markers,
                        });
                        cursor = tl_start.saturating_add(dur);
                    }
                }
            }
            tl.tracks.push(track);
        }

        Ok(tl)
    }

    // -------------------------------------------------------------------------
    // CMX 3600 .edl Export & Import
    // -------------------------------------------------------------------------

    pub fn to_cmx3600_edl(&self) -> String {
        let sr = self.sample_rate.max(1);
        let fps = smpte_fps(self.fps_num, self.fps_den);
        let mut out = String::new();
        out.push_str(&format!("TITLE: {}\n", sanitize_line(&self.name)));
        out.push_str("FCM: NON-DROP FRAME\n");
        out.push_str(&format!(
            "* SAMPLE_RATE: {} FPS: {}/{}\n",
            sr,
            self.fps_num.max(1),
            self.fps_den.max(1)
        ));
        out.push_str(&format!(
            "* VIDEO_SYNC_OFFSET_FRAMES: {}\n",
            self.video_sync_offset_frames
        ));

        if self.loudness.integrated_lufs.is_some()
            || self.loudness.true_peak_dbtp.is_some()
            || self.loudness.loudness_range_lu.is_some()
            || self.loudness.target_lufs.is_some()
        {
            out.push_str(&format!(
                "* LOUDNESS: INTEGRATED={} TRUE_PEAK={} LRA={} TARGET={}\n",
                opt_f32_str(self.loudness.integrated_lufs),
                opt_f32_str(self.loudness.true_peak_dbtp),
                opt_f32_str(self.loudness.loudness_range_lu),
                opt_f32_str(self.loudness.target_lufs)
            ));
        }

        for m in &self.markers {
            out.push_str(&format!(
                "* MARKER: {} {} {}\n",
                frames_to_smpte(m.start_frame, sr, fps),
                sanitize_token(&m.color, "GREEN"),
                sanitize_line(&m.name)
            ));
        }

        let mut event_idx = 1usize;
        for (ti, track) in self.tracks.iter().enumerate() {
            let chan = match track.kind {
                InterchangeTrackKind::Video => "V",
                InterchangeTrackKind::Audio => "AA",
            };
            for clip in &track.clips {
                let reel = make_reel_id(&clip.name, event_idx);
                let src_in = frames_to_smpte(clip.source_start_frame, sr, fps);
                let src_out = frames_to_smpte(
                    clip.source_start_frame.saturating_add(clip.duration_frames),
                    sr,
                    fps,
                );
                let rec_in = frames_to_smpte(clip.timeline_start_frame, sr, fps);
                let rec_out = frames_to_smpte(
                    clip.timeline_start_frame.saturating_add(clip.duration_frames),
                    sr,
                    fps,
                );
                out.push_str(&format!(
                    "{:03}  {:<8} {:<4}  C        {} {} {} {}\n",
                    event_idx, reel, chan, src_in, src_out, rec_in, rec_out
                ));
                out.push_str(&format!("* FROM CLIP NAME: {}\n", sanitize_line(&clip.name)));
                if !clip.media_url.is_empty() {
                    out.push_str(&format!(
                        "* SOURCE FILE: {}\n",
                        sanitize_line(&clip.media_url)
                    ));
                }
                out.push_str(&format!(
                    "* TRACK: {} {} VOL={} PAN={} MUTED={}\n",
                    ti,
                    sanitize_token(&track.name, "Audio"),
                    track.volume_percent,
                    track.pan,
                    track.muted as u8
                ));
                out.push_str(&format!(
                    "* FADES: IN={} OUT={} GAIN={} CLIP_MUTED={}\n",
                    clip.fade_in_frames,
                    clip.fade_out_frames,
                    clip.gain_percent,
                    clip.muted as u8
                ));
                for m in &clip.markers {
                    out.push_str(&format!(
                        "* CLIP MARKER: {} {} {}\n",
                        frames_to_smpte(m.start_frame, sr, fps),
                        sanitize_token(&m.color, "GREEN"),
                        sanitize_line(&m.name)
                    ));
                }
                event_idx += 1;
            }
        }

        out
    }

    pub fn from_cmx3600_edl(
        edl: &str,
        sample_rate: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<Self, InterchangeError> {
        if edl.len() > MAX_INPUT_BYTES {
            return Err(InterchangeError::InputTooLarge);
        }
        let mut tl = Self::new("EDL Import");
        tl.sample_rate = sample_rate.clamp(8000, 192_000);
        tl.fps_num = fps_num.max(1);
        tl.fps_den = fps_den.max(1);

        let mut last_loc: Option<(usize, usize)> = None;
        let mut saw_any = false;

        for raw_line in edl.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(title) = line.strip_prefix("TITLE:") {
                tl.name = String::from(title.trim());
                saw_any = true;
                continue;
            }
            if line.starts_with("FCM:") {
                saw_any = true;
                continue;
            }
            if let Some(comment) = line.strip_prefix('*') {
                let c = comment.trim();
                if let Some(rest) = c.strip_prefix("SAMPLE_RATE:") {
                    for tok in rest.split_whitespace() {
                        if let Ok(sr) = tok.parse::<u32>() {
                            if (8000..=192_000).contains(&sr) {
                                tl.sample_rate = sr;
                            }
                            break;
                        }
                    }
                    if let Some(idx) = rest.find("FPS:") {
                        let fps_str = rest[idx + 4..].trim();
                        if let Some((n, d)) = fps_str.split_once('/') {
                            if let (Ok(nv), Ok(dv)) = (n.trim().parse::<u32>(), d.trim().parse::<u32>())
                            {
                                if nv > 0 && dv > 0 {
                                    tl.fps_num = nv;
                                    tl.fps_den = dv;
                                }
                            }
                        }
                    }
                } else if let Some(rest) = c.strip_prefix("VIDEO_SYNC_OFFSET_FRAMES:") {
                    if let Ok(off) = rest.trim().parse::<i64>() {
                        tl.video_sync_offset_frames = off;
                    }
                } else if let Some(rest) = c.strip_prefix("LOUDNESS:") {
                    for tok in rest.split_whitespace() {
                        if let Some((k, v)) = tok.split_once('=') {
                            let val = if v == "NONE" {
                                None
                            } else {
                                v.parse::<f32>().ok()
                            };
                            match k {
                                "INTEGRATED" => tl.loudness.integrated_lufs = val,
                                "TRUE_PEAK" => tl.loudness.true_peak_dbtp = val,
                                "LRA" => tl.loudness.loudness_range_lu = val,
                                "TARGET" => tl.loudness.target_lufs = val,
                                _ => {}
                            }
                        }
                    }
                } else if let Some(rest) = c.strip_prefix("MARKER:") {
                    let fps = smpte_fps(tl.fps_num, tl.fps_den);
                    if let Some(m) = parse_edl_marker(rest, tl.sample_rate, fps) {
                        tl.markers.push(m);
                    }
                } else if let Some(rest) = c.strip_prefix("FROM CLIP NAME:") {
                    if let Some((ti, ci)) = last_loc {
                        tl.tracks[ti].clips[ci].name = String::from(rest.trim());
                    }
                } else if let Some(rest) = c.strip_prefix("SOURCE FILE:") {
                    if let Some((ti, ci)) = last_loc {
                        tl.tracks[ti].clips[ci].media_url = String::from(rest.trim());
                    }
                } else if let Some(rest) = c.strip_prefix("FADES:") {
                    if let Some((ti, ci)) = last_loc {
                        let clip = &mut tl.tracks[ti].clips[ci];
                        for tok in rest.split_whitespace() {
                            if let Some((k, v)) = tok.split_once('=') {
                                match k {
                                    "IN" => {
                                        if let Ok(n) = v.parse::<u64>() {
                                            clip.fade_in_frames = n;
                                        }
                                    }
                                    "OUT" => {
                                        if let Ok(n) = v.parse::<u64>() {
                                            clip.fade_out_frames = n;
                                        }
                                    }
                                    "GAIN" => {
                                        if let Ok(g) = v.parse::<u16>() {
                                            clip.gain_percent = g.min(400);
                                        }
                                    }
                                    "CLIP_MUTED" => clip.muted = v == "1" || v == "true",
                                    _ => {}
                                }
                            }
                        }
                    }
                } else if let Some(rest) = c.strip_prefix("TRACK:") {
                    if let Some((ti, _ci)) = last_loc {
                        for tok in rest.split_whitespace() {
                            if let Some((k, v)) = tok.split_once('=') {
                                match k {
                                    "VOL" => {
                                        if let Ok(val) = v.parse::<u16>() {
                                            tl.tracks[ti].volume_percent = val.min(400);
                                        }
                                    }
                                    "PAN" => {
                                        if let Ok(val) = v.parse::<i16>() {
                                            tl.tracks[ti].pan = val.clamp(-100, 100);
                                        }
                                    }
                                    "MUTED" => tl.tracks[ti].muted = v == "1" || v == "true",
                                    _ => {}
                                }
                            }
                        }
                    }
                } else if let Some(rest) = c.strip_prefix("CLIP MARKER:") {
                    if let Some((ti, ci)) = last_loc {
                        let fps = smpte_fps(tl.fps_num, tl.fps_den);
                        if let Some(m) = parse_edl_marker(rest, tl.sample_rate, fps) {
                            tl.tracks[ti].clips[ci].markers.push(m);
                        }
                    }
                }
                continue;
            }

            // Parse CMX 3600 event row: `<num> <reel> <chan> <edit> <src_in> <src_out> <rec_in> <rec_out>`
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 8 && cols[0].chars().all(|ch| ch.is_ascii_digit()) {
                saw_any = true;
                let fps = smpte_fps(tl.fps_num, tl.fps_den);
                let src_in = smpte_to_frames(cols[4], tl.sample_rate, fps)
                    .ok_or(InterchangeError::InvalidEdl)?;
                let src_out = smpte_to_frames(cols[5], tl.sample_rate, fps)
                    .ok_or(InterchangeError::InvalidEdl)?;
                let rec_in = smpte_to_frames(cols[6], tl.sample_rate, fps)
                    .ok_or(InterchangeError::InvalidEdl)?;
                let rec_out = smpte_to_frames(cols[7], tl.sample_rate, fps)
                    .ok_or(InterchangeError::InvalidEdl)?;
                let kind = if cols[2].starts_with('V') {
                    InterchangeTrackKind::Video
                } else {
                    InterchangeTrackKind::Audio
                };
                let trk_idx = match tl.tracks.iter().position(|t| t.kind == kind) {
                    Some(idx) => idx,
                    None => {
                        let name = if kind == InterchangeTrackKind::Video {
                            "Video 1"
                        } else {
                            "Audio 1"
                        };
                        tl.tracks.push(InterchangeTrack {
                            name: String::from(name),
                            kind,
                            volume_percent: 100,
                            pan: 0,
                            muted: false,
                            solo: false,
                            clips: Vec::new(),
                            markers: Vec::new(),
                        });
                        tl.tracks.len() - 1
                    }
                };
                let dur = rec_out
                    .saturating_sub(rec_in)
                    .max(src_out.saturating_sub(src_in));
                tl.tracks[trk_idx].clips.push(InterchangeClip::new(
                    cols[1],
                    "",
                    rec_in,
                    src_in,
                    dur,
                ));
                last_loc = Some((trk_idx, tl.tracks[trk_idx].clips.len() - 1));
            }
        }

        if !saw_any {
            return Err(InterchangeError::InvalidEdl);
        }
        Ok(tl)
    }

    // -------------------------------------------------------------------------
    // AAF-XML Summary Export & Import
    // -------------------------------------------------------------------------

    pub fn to_aaf_xml(&self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str(&format!(
            "<AAFExchangeSummary version=\"1.0\" name=\"{}\" sampleRate=\"{}\" fpsNum=\"{}\" fpsDen=\"{}\" videoSyncOffsetFrames=\"{}\">\n",
            xml_escape(&self.name),
            self.sample_rate.max(1),
            self.fps_num.max(1),
            self.fps_den.max(1),
            self.video_sync_offset_frames
        ));
        out.push_str(&format!(
            "  <Loudness integratedLufs=\"{}\" truePeakDbtp=\"{}\" loudnessRangeLu=\"{}\" targetLufs=\"{}\"/>\n",
            opt_f32_str(self.loudness.integrated_lufs),
            opt_f32_str(self.loudness.true_peak_dbtp),
            opt_f32_str(self.loudness.loudness_range_lu),
            opt_f32_str(self.loudness.target_lufs)
        ));
        out.push_str("  <Markers>\n");
        for m in &self.markers {
            out.push_str(&format!(
                "    <Marker name=\"{}\" startFrame=\"{}\" durationFrames=\"{}\" color=\"{}\" comment=\"{}\"/>\n",
                xml_escape(&m.name),
                m.start_frame,
                m.duration_frames,
                xml_escape(&m.color),
                xml_escape(&m.comment)
            ));
        }
        out.push_str("  </Markers>\n");
        out.push_str("  <Tracks>\n");
        for track in &self.tracks {
            let kind = match track.kind {
                InterchangeTrackKind::Audio => "Audio",
                InterchangeTrackKind::Video => "Video",
            };
            out.push_str(&format!(
                "    <Track name=\"{}\" kind=\"{}\" volumePercent=\"{}\" pan=\"{}\" muted=\"{}\" solo=\"{}\">\n",
                xml_escape(&track.name),
                kind,
                track.volume_percent,
                track.pan,
                track.muted,
                track.solo
            ));
            for clip in &track.clips {
                out.push_str(&format!(
                    "      <Clip name=\"{}\" mediaUrl=\"{}\" timelineStartFrame=\"{}\" sourceStartFrame=\"{}\" durationFrames=\"{}\" fadeInFrames=\"{}\" fadeOutFrames=\"{}\" gainPercent=\"{}\" muted=\"{}\">\n",
                    xml_escape(&clip.name),
                    xml_escape(&clip.media_url),
                    clip.timeline_start_frame,
                    clip.source_start_frame,
                    clip.duration_frames,
                    clip.fade_in_frames,
                    clip.fade_out_frames,
                    clip.gain_percent,
                    clip.muted
                ));
                for m in &clip.markers {
                    out.push_str(&format!(
                        "        <Marker name=\"{}\" startFrame=\"{}\" durationFrames=\"{}\" color=\"{}\" comment=\"{}\"/>\n",
                        xml_escape(&m.name),
                        m.start_frame,
                        m.duration_frames,
                        xml_escape(&m.color),
                        xml_escape(&m.comment)
                    ));
                }
                out.push_str("      </Clip>\n");
            }
            out.push_str("    </Track>\n");
        }
        out.push_str("  </Tracks>\n");
        out.push_str("</AAFExchangeSummary>\n");
        out
    }

    pub fn from_aaf_xml(xml: &str) -> Result<Self, InterchangeError> {
        if xml.len() > MAX_INPUT_BYTES {
            return Err(InterchangeError::InputTooLarge);
        }
        let mut tl = Self::new("AAF Import");
        let mut found_root = false;
        let mut cur_track: Option<InterchangeTrack> = None;
        let mut cur_clip: Option<InterchangeClip> = None;

        let bytes = xml.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                i += 1;
                continue;
            }
            let Some(end_rel) = xml[i + 1..].find('>') else {
                return Err(InterchangeError::InvalidXml);
            };
            let tag = xml[i + 1..i + 1 + end_rel].trim();
            i += end_rel + 2;

            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if tag.starts_with('/') {
                let name = tag[1..].trim();
                if name == "Clip" {
                    if let (Some(c), Some(t)) = (cur_clip.take(), cur_track.as_mut()) {
                        t.clips.push(c);
                    }
                } else if name == "Track" {
                    if let Some(c) = cur_clip.take() {
                        if let Some(t) = cur_track.as_mut() {
                            t.clips.push(c);
                        }
                    }
                    if let Some(t) = cur_track.take() {
                        tl.tracks.push(t);
                    }
                }
                continue;
            }

            let self_closing = tag.ends_with('/');
            let tag_inner = tag.trim_end_matches('/').trim();
            let (tag_name, attr_str) = match tag_inner.find(|c: char| c.is_ascii_whitespace()) {
                Some(idx) => (&tag_inner[..idx], &tag_inner[idx..]),
                None => (tag_inner, ""),
            };
            let attrs = parse_xml_attrs(attr_str)?;

            match tag_name {
                "AAFExchangeSummary" => {
                    found_root = true;
                    if let Some(n) = xml_attr(&attrs, "name") {
                        tl.name = n;
                    }
                    if let Some(sr) = xml_attr(&attrs, "sampleRate").and_then(|s| s.parse::<u32>().ok()) {
                        tl.sample_rate = sr.clamp(8000, 192_000);
                    }
                    if let Some(fn_) = xml_attr(&attrs, "fpsNum").and_then(|s| s.parse::<u32>().ok()) {
                        tl.fps_num = fn_.max(1);
                    }
                    if let Some(fd) = xml_attr(&attrs, "fpsDen").and_then(|s| s.parse::<u32>().ok()) {
                        tl.fps_den = fd.max(1);
                    }
                    if let Some(off) =
                        xml_attr(&attrs, "videoSyncOffsetFrames").and_then(|s| s.parse::<i64>().ok())
                    {
                        tl.video_sync_offset_frames = off;
                    }
                }
                "Loudness" => {
                    tl.loudness = InterchangeLoudness {
                        integrated_lufs: xml_attr(&attrs, "integratedLufs").and_then(parse_opt_f32),
                        true_peak_dbtp: xml_attr(&attrs, "truePeakDbtp").and_then(parse_opt_f32),
                        loudness_range_lu: xml_attr(&attrs, "loudnessRangeLu").and_then(parse_opt_f32),
                        target_lufs: xml_attr(&attrs, "targetLufs").and_then(parse_opt_f32),
                    };
                }
                "Track" => {
                    let tname = xml_attr(&attrs, "name").unwrap_or_else(|| String::from("Track"));
                    let kind = match xml_attr(&attrs, "kind").as_deref() {
                        Some("Video") => InterchangeTrackKind::Video,
                        _ => InterchangeTrackKind::Audio,
                    };
                    let trk = InterchangeTrack {
                        name: tname,
                        kind,
                        volume_percent: xml_attr(&attrs, "volumePercent")
                            .and_then(|s| s.parse::<u16>().ok())
                            .unwrap_or(100)
                            .min(400),
                        pan: xml_attr(&attrs, "pan")
                            .and_then(|s| s.parse::<i16>().ok())
                            .unwrap_or(0)
                            .clamp(-100, 100),
                        muted: xml_attr(&attrs, "muted").is_some_and(|s| s == "true"),
                        solo: xml_attr(&attrs, "solo").is_some_and(|s| s == "true"),
                        clips: Vec::new(),
                        markers: Vec::new(),
                    };
                    if self_closing {
                        tl.tracks.push(trk);
                    } else {
                        cur_track = Some(trk);
                    }
                }
                "Clip" => {
                    let c = InterchangeClip {
                        name: xml_attr(&attrs, "name").unwrap_or_default(),
                        media_url: xml_attr(&attrs, "mediaUrl").unwrap_or_default(),
                        timeline_start_frame: xml_attr(&attrs, "timelineStartFrame")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        source_start_frame: xml_attr(&attrs, "sourceStartFrame")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        duration_frames: xml_attr(&attrs, "durationFrames")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        fade_in_frames: xml_attr(&attrs, "fadeInFrames")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        fade_out_frames: xml_attr(&attrs, "fadeOutFrames")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        gain_percent: xml_attr(&attrs, "gainPercent")
                            .and_then(|s| s.parse::<u16>().ok())
                            .unwrap_or(100)
                            .min(400),
                        muted: xml_attr(&attrs, "muted").is_some_and(|s| s == "true"),
                        markers: Vec::new(),
                    };
                    if self_closing {
                        if let Some(t) = cur_track.as_mut() {
                            t.clips.push(c);
                        }
                    } else {
                        cur_clip = Some(c);
                    }
                }
                "Marker" => {
                    let m = InterchangeMarker {
                        name: xml_attr(&attrs, "name").unwrap_or_default(),
                        start_frame: xml_attr(&attrs, "startFrame")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        duration_frames: xml_attr(&attrs, "durationFrames")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        color: xml_attr(&attrs, "color").unwrap_or_else(|| String::from("GREEN")),
                        comment: xml_attr(&attrs, "comment").unwrap_or_default(),
                    };
                    if let Some(c) = cur_clip.as_mut() {
                        c.markers.push(m);
                    } else if let Some(t) = cur_track.as_mut() {
                        t.markers.push(m);
                    } else {
                        tl.markers.push(m);
                    }
                }
                _ => {}
            }
        }

        if !found_root {
            return Err(InterchangeError::InvalidXml);
        }
        Ok(tl)
    }
}

// --- OTIO / SMPTE / Formatting helpers ---

fn otio_time_range(start: u64, dur: u64, sample_rate: u32) -> String {
    format!(
        "{{\"OTIO_SCHEMA\":\"TimeRange.1\",\"start_time\":{{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":{}.0,\"value\":{}.0}},\"duration\":{{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":{}.0,\"value\":{}.0}}}}",
        sample_rate, start, sample_rate, dur
    )
}

fn otio_marker_json(m: &InterchangeMarker, sample_rate: u32) -> String {
    format!(
        "{{\"OTIO_SCHEMA\":\"Marker.2\",\"name\":\"{}\",\"color\":\"{}\",\"marked_range\":{},\"metadata\":{{\"comment\":\"{}\"}}}}",
        json_escape(&m.name),
        json_escape(if m.color.is_empty() { "GREEN" } else { &m.color }),
        otio_time_range(m.start_frame, m.duration_frames, sample_rate),
        json_escape(&m.comment)
    )
}

fn json_opt_f32(v: Option<f32>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{:.3}", x),
        _ => String::from("null"),
    }
}

fn opt_f32_str(v: Option<f32>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{:.3}", x),
        _ => String::from("NONE"),
    }
}

fn parse_opt_f32(s: String) -> Option<f32> {
    if s == "NONE" || s.is_empty() {
        None
    } else {
        s.parse::<f32>().ok()
    }
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        if let Some(r) = after.strip_prefix("&amp;") {
            out.push('&');
            rest = r;
        } else if let Some(r) = after.strip_prefix("&lt;") {
            out.push('<');
            rest = r;
        } else if let Some(r) = after.strip_prefix("&gt;") {
            out.push('>');
            rest = r;
        } else if let Some(r) = after.strip_prefix("&quot;") {
            out.push('"');
            rest = r;
        } else if let Some(r) = after.strip_prefix("&apos;") {
            out.push('\'');
            rest = r;
        } else {
            out.push('&');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

fn parse_xml_attrs(mut s: &str) -> Result<Vec<(String, String)>, InterchangeError> {
    let mut attrs = Vec::new();
    loop {
        s = s.trim_start();
        if s.is_empty() {
            break;
        }
        let Some(eq) = s.find('=') else {
            return Err(InterchangeError::InvalidXml);
        };
        let key = s[..eq].trim();
        s = s[eq + 1..].trim_start();
        let quote = s.as_bytes().first().copied().ok_or(InterchangeError::InvalidXml)?;
        if quote != b'"' && quote != b'\'' {
            return Err(InterchangeError::InvalidXml);
        }
        s = &s[1..];
        let Some(end_q) = s.find(quote as char) else {
            return Err(InterchangeError::InvalidXml);
        };
        let val = xml_unescape(&s[..end_q]);
        attrs.push((String::from(key), val));
        s = &s[end_q + 1..];
    }
    Ok(attrs)
}

fn xml_attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn sanitize_line(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

fn sanitize_token(s: &str, fallback: &str) -> String {
    let tok: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if tok.is_empty() {
        String::from(fallback)
    } else {
        tok
    }
}

fn make_reel_id(name: &str, idx: usize) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.is_empty() {
        format!("AX{:03}", idx % 1000)
    } else {
        cleaned
    }
}

fn smpte_fps(num: u32, den: u32) -> u32 {
    let n = num.max(1);
    let d = den.max(1);
    n.div_ceil(d).clamp(1, 120)
}

fn frames_to_smpte(audio_frames: u64, sample_rate: u32, fps: u32) -> String {
    let sr = sample_rate.max(1) as u128;
    let total_video_frames =
        ((audio_frames as u128 * fps as u128 + (sr / 2)) / sr) as u64;
    let f = total_video_frames % fps as u64;
    let total_secs = total_video_frames / fps as u64;
    let s = total_secs % 60;
    let total_mins = total_secs / 60;
    let m = total_mins % 60;
    let h = (total_mins / 60) % 100;
    format!("{:02}:{:02}:{:02}:{:02}", h, m, s, f)
}

fn smpte_to_frames(tc: &str, sample_rate: u32, fps: u32) -> Option<u64> {
    let parts: Vec<&str> = tc.split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let h = parts[0].parse::<u64>().ok()?;
    let m = parts[1].parse::<u64>().ok()?;
    let s = parts[2].parse::<u64>().ok()?;
    let f = parts[3].parse::<u64>().ok()?;
    let total_video_frames = (((h * 3600 + m * 60 + s) * fps as u64) + f) as u128;
    Some(((total_video_frames * sample_rate as u128 + (fps as u128 / 2)) / fps as u128) as u64)
}

fn parse_edl_marker(rest: &str, sample_rate: u32, fps: u32) -> Option<InterchangeMarker> {
    let trimmed = rest.trim();
    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let tc = parts.next()?.trim();
    let color = parts.next()?.trim();
    let name = parts.next().unwrap_or("").trim();
    let start_frame = smpte_to_frames(tc, sample_rate, fps)?;
    Some(InterchangeMarker {
        name: String::from(name),
        start_frame,
        duration_frames: 0,
        color: String::from(color),
        comment: String::new(),
    })
}

fn rational_to_u64_frames(value: f64, rate: f64, target_sr: u32) -> u64 {
    if !value.is_finite() || !rate.is_finite() || rate <= 0.0 || value <= 0.0 {
        return 0;
    }
    let frames = (value * (target_sr as f64)) / rate;
    round_f32(frames.max(0.0) as f32) as u64
}

fn rational_to_signed_frames(value: f64, rate: f64, target_sr: u32) -> i64 {
    if !value.is_finite() || !rate.is_finite() || rate <= 0.0 {
        return 0;
    }
    let frames = (value * (target_sr as f64)) / rate;
    round_f32(frames as f32) as i64
}

fn parse_rational_time(obj: &[(String, JsonValue)]) -> Option<(f64, f64)> {
    let value = json_f64(obj, "value")?;
    let rate = json_f64(obj, "rate").unwrap_or(24.0);
    Some((value, rate))
}

fn parse_time_range(obj: &[(String, JsonValue)], target_sr: u32) -> (u64, u64) {
    let start = json_obj(obj, "start_time")
        .and_then(parse_rational_time)
        .map(|(v, r)| rational_to_u64_frames(v, r, target_sr))
        .unwrap_or(0);
    let dur = json_obj(obj, "duration")
        .and_then(parse_rational_time)
        .map(|(v, r)| rational_to_u64_frames(v, r, target_sr))
        .unwrap_or(0);
    (start, dur)
}

fn parse_otio_marker(val: &JsonValue, target_sr: u32) -> Option<InterchangeMarker> {
    let obj = val.as_object()?;
    let name = json_str(obj, "name").unwrap_or("");
    let color = json_str(obj, "color").unwrap_or("GREEN");
    let (start_frame, duration_frames) = json_obj(obj, "marked_range")
        .map(|r| parse_time_range(r, target_sr))
        .unwrap_or((0, 0));
    let comment = json_obj(obj, "metadata")
        .and_then(|m| json_str(m, "comment"))
        .unwrap_or("");
    Some(InterchangeMarker {
        name: String::from(name),
        start_frame,
        duration_frames,
        color: String::from(color),
        comment: String::from(comment),
    })
}

fn extract_media_url(clip: &[(String, JsonValue)]) -> Option<&str> {
    let refs = json_obj(clip, "media_references")?;
    let key = json_str(clip, "active_media_reference_key").unwrap_or("DEFAULT_MEDIA");
    let media = json_obj(refs, key).or_else(|| json_obj(refs, "DEFAULT_MEDIA"))?;
    json_str(media, "target_url")
}

// --- Pure no_std bounded recursive-descent JSON parser ---

#[derive(Clone, Debug)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }
    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

fn json_get<'a>(obj: &'a [(String, JsonValue)], key: &str) -> Option<&'a JsonValue> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}
fn json_str<'a>(obj: &'a [(String, JsonValue)], key: &str) -> Option<&'a str> {
    json_get(obj, key)?.as_str()
}
fn json_f64(obj: &[(String, JsonValue)], key: &str) -> Option<f64> {
    json_get(obj, key)?.as_f64()
}
fn json_bool(obj: &[(String, JsonValue)], key: &str) -> Option<bool> {
    json_get(obj, key)?.as_bool()
}
fn json_obj<'a>(obj: &'a [(String, JsonValue)], key: &str) -> Option<&'a [(String, JsonValue)]> {
    json_get(obj, key)?.as_object()
}
fn json_arr<'a>(obj: &'a [(String, JsonValue)], key: &str) -> Option<&'a [JsonValue]> {
    json_get(obj, key)?.as_array()
}

struct JsonParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.src.len()
            && matches!(self.src[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, InterchangeError> {
        if depth > MAX_JSON_DEPTH {
            return Err(InterchangeError::NestingTooDeep);
        }
        let b = self.peek().ok_or(InterchangeError::InvalidJson)?;
        match b {
            b'{' => self.parse_object(depth + 1),
            b'[' => self.parse_array(depth + 1),
            b'"' => self.parse_string().map(JsonValue::Str),
            b't' => self.expect_bytes(b"true").map(|_| JsonValue::Bool(true)),
            b'f' => self.expect_bytes(b"false").map(|_| JsonValue::Bool(false)),
            b'n' => self.expect_bytes(b"null").map(|_| JsonValue::Null),
            b'-' | b'0'..=b'9' => self.parse_number().map(JsonValue::Number),
            _ => Err(InterchangeError::InvalidJson),
        }
    }

    fn expect_bytes(&mut self, lit: &[u8]) -> Result<(), InterchangeError> {
        self.skip_ws();
        if self.src.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(InterchangeError::InvalidJson)
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue, InterchangeError> {
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(pairs));
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(InterchangeError::InvalidJson);
            }
            let key = self.parse_string()?;
            if self.peek() != Some(b':') {
                return Err(InterchangeError::InvalidJson);
            }
            self.pos += 1;
            let val = self.parse_value(depth)?;
            pairs.push((key, val));
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(InterchangeError::InvalidJson),
            }
        }
        Ok(JsonValue::Object(pairs))
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue, InterchangeError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value(depth)?);
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(InterchangeError::InvalidJson),
            }
        }
        Ok(JsonValue::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, InterchangeError> {
        self.skip_ws();
        if self.src.get(self.pos) != Some(&b'"') {
            return Err(InterchangeError::InvalidJson);
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = *self.src.get(self.pos).ok_or(InterchangeError::InvalidJson)?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0c'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code) {
                            if self.src.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined =
                                        0x10000 + (((code - 0xD800) << 10) | (low - 0xDC00));
                                    let ch = char::from_u32(combined)
                                        .ok_or(InterchangeError::InvalidJson)?;
                                    out.push(ch);
                                } else {
                                    return Err(InterchangeError::InvalidJson);
                                }
                            } else {
                                return Err(InterchangeError::InvalidJson);
                            }
                        } else {
                            let ch = char::from_u32(code).ok_or(InterchangeError::InvalidJson)?;
                            out.push(ch);
                        }
                    }
                    _ => return Err(InterchangeError::InvalidJson),
                }
            } else if b < 0x20 {
                return Err(InterchangeError::InvalidJson);
            } else if b < 0x80 {
                out.push(b as char);
                self.pos += 1;
            } else {
                let s = core::str::from_utf8(&self.src[self.pos..])
                    .map_err(|_| InterchangeError::InvalidJson)?;
                let ch = s.chars().next().ok_or(InterchangeError::InvalidJson)?;
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err(InterchangeError::InvalidJson)
    }

    fn parse_hex4(&mut self) -> Result<u32, InterchangeError> {
        let slice = self
            .src
            .get(self.pos..self.pos + 4)
            .ok_or(InterchangeError::InvalidJson)?;
        self.pos += 4;
        let s = core::str::from_utf8(slice).map_err(|_| InterchangeError::InvalidJson)?;
        u32::from_str_radix(s, 16).map_err(|_| InterchangeError::InvalidJson)
    }

    fn parse_number(&mut self) -> Result<f64, InterchangeError> {
        self.skip_ws();
        let start = self.pos;
        if self.src.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let mut has_digits = false;
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
            has_digits = true;
            self.pos += 1;
        }
        if !has_digits {
            return Err(InterchangeError::InvalidJson);
        }
        if self.src.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            let mut frac_digits = false;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                frac_digits = true;
                self.pos += 1;
            }
            if !frac_digits {
                return Err(InterchangeError::InvalidJson);
            }
        }
        if matches!(self.src.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.src.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let mut exp_digits = false;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                exp_digits = true;
                self.pos += 1;
            }
            if !exp_digits {
                return Err(InterchangeError::InvalidJson);
            }
        }
        let s = core::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| InterchangeError::InvalidJson)?;
        s.parse::<f64>().map_err(|_| InterchangeError::InvalidJson)
    }
}

fn parse_json(input: &str) -> Result<JsonValue, InterchangeError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(InterchangeError::InputTooLarge);
    }
    let mut parser = JsonParser {
        src: input.as_bytes(),
        pos: 0,
    };
    let val = parser.parse_value(0)?;
    parser.skip_ws();
    if parser.pos != parser.src.len() {
        return Err(InterchangeError::InvalidJson);
    }
    Ok(val)
}
