// SPDX-License-Identifier: GPL-3.0-only
//! Handoff to non-linear editors: FCPXML 1.10, CMX 3600 EDL, SubRip SRT subtitles,
//! and self-documenting NLE handoff bundles with audit checklists.
//!
//! Adapted from Scarlet's Advanced Export `handoff.ts` & `fcpxml.ts` (MIT) and Rook's
//! `rook-exporters` (GPL-3.0).

use crate::render::{plan, Piece};
use crate::transcript::Transcript;
use crate::{Micros, Timeline, TrackKind, SPEED_NORMAL};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Rational seconds `{us}/1000000s` reduced by `gcd`.
fn rational_time(us: Micros) -> String {
    if us == 0 {
        return "0s".into();
    }
    let sign = if us < 0 { "-" } else { "" };
    let n = us.unsigned_abs();
    let d = 1_000_000u64;
    let g = gcd(n, d);
    let num = n / g;
    let den = d / g;
    if den == 1 {
        format!("{}{}s", sign, num)
    } else {
        format!("{}{}/{}s", sign, num, den)
    }
}

/// XML-escapes text for attribute values and text nodes.
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

/// Generates valid Final Cut Pro XML 1.10 from the timeline.
pub fn export_fcpxml(tl: &Timeline, name: &str) -> String {
    let fps_num = tl.fps_num.max(1) as u64;
    let fps_den = tl.fps_den.max(1) as u64;
    let g = gcd(fps_den, fps_num);
    let fd_den = fps_den / g;
    let fd_num = fps_num / g;
    let frame_duration = if fd_num == 1 {
        format!("{}s", fd_den)
    } else {
        format!("{}/{}s", fd_den, fd_num)
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<!DOCTYPE fcpxml>\n");
    out.push_str("<fcpxml version=\"1.10\">\n");
    out.push_str("  <resources>\n");
    out.push_str(&format!(
        "    <format id=\"r1\" frameDuration=\"{}\"/>\n",
        frame_duration
    ));

    for m in &tl.media {
        let file_name = m.path.rsplit('/').next().unwrap_or(&m.path);
        let src = if m.path.starts_with("file://") {
            m.path.clone()
        } else if m.path.starts_with('/') {
            format!("file://{}", m.path)
        } else {
            format!("file:///{}", m.path)
        };
        let (has_video, has_audio) = match m.kind {
            crate::MediaKind::Video => ("1", "1"),
            crate::MediaKind::Audio => ("0", "1"),
            crate::MediaKind::Image => ("1", "0"),
        };
        out.push_str(&format!(
            "    <asset id=\"a{}\" name=\"{}\" src=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"{}\" hasAudio=\"{}\"/>\n",
            m.id,
            xml_escape(file_name),
            xml_escape(&src),
            rational_time(m.duration),
            has_video,
            has_audio
        ));
    }
    out.push_str("  </resources>\n");

    out.push_str("  <library>\n");
    out.push_str(&format!("    <event name=\"{}\">\n", xml_escape(name)));
    out.push_str(&format!("      <project name=\"{}\">\n", xml_escape(name)));
    out.push_str(&format!(
        "        <sequence format=\"r1\" duration=\"{}\">\n",
        rational_time(tl.duration())
    ));
    out.push_str("          <spine>\n");

    let plans = plan(tl);
    let primary_track_id = tl
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .or_else(|| tl.tracks.first())
        .map(|t| t.id);

    let primary_plan = primary_track_id.and_then(|id| plans.iter().find(|p| p.track == id));

    // Gather secondary audio pieces to attach as lane="-1"
    let mut attached_audio: Vec<&Piece> = Vec::new();
    for track in &tl.tracks {
        if track.kind == TrackKind::Audio && Some(track.id) != primary_track_id {
            if let Some(tp) = plans.iter().find(|p| p.track == track.id) {
                for piece in &tp.pieces {
                    if let Piece::Play { .. } = piece {
                        attached_audio.push(piece);
                    }
                }
            }
        }
    }

    if let Some(pp) = primary_plan {
        let piece_count = pp.pieces.len();
        for (idx, piece) in pp.pieces.iter().enumerate() {
            let is_last = idx + 1 == piece_count;
            let p_span = piece.timeline();

            // Find audio pieces whose start falls within this spine piece
            let mut matching_audio = Vec::new();
            attached_audio.retain(|ap| {
                let a_start = ap.timeline().start;
                if a_start >= p_span.start && (a_start < p_span.end || (is_last && a_start <= p_span.end)) {
                    matching_audio.push(*ap);
                    false
                } else {
                    true
                }
            });

            // If it's the last piece, take any remaining unattached audio pieces
            if is_last {
                matching_audio.extend(attached_audio.drain(..));
            }

            match piece {
                Piece::Play {
                    media,
                    timeline,
                    source,
                    speed,
                    ..
                } => {
                    let m = tl.media(*media);
                    let file_name = m.map(|m| m.path.rsplit('/').next().unwrap_or(&m.path)).unwrap_or("clip");
                    let has_speed = *speed != SPEED_NORMAL;
                    let has_children = has_speed || !matching_audio.is_empty();

                    if has_children {
                        out.push_str(&format!(
                            "            <asset-clip ref=\"a{}\" name=\"{}\" offset=\"{}\" duration=\"{}\" start=\"{}\">\n",
                            media,
                            xml_escape(file_name),
                            rational_time(timeline.start),
                            rational_time(timeline.len()),
                            rational_time(source.start)
                        ));
                        if has_speed {
                            out.push_str(&format!(
                                "              <note>speed: {}‰ (conform-rate / timeMap)</note>\n",
                                speed
                            ));
                            out.push_str("              <conform-rate scaleEnabled=\"0\"/>\n");
                        }
                        for ap in matching_audio {
                            render_attached_audio(ap, tl, &mut out);
                        }
                        out.push_str("            </asset-clip>\n");
                    } else {
                        out.push_str(&format!(
                            "            <asset-clip ref=\"a{}\" name=\"{}\" offset=\"{}\" duration=\"{}\" start=\"{}\"/>\n",
                            media,
                            xml_escape(file_name),
                            rational_time(timeline.start),
                            rational_time(timeline.len()),
                            rational_time(source.start)
                        ));
                    }
                }
                Piece::Gap { timeline } => {
                    if !matching_audio.is_empty() {
                        out.push_str(&format!(
                            "            <gap name=\"Gap\" offset=\"{}\" duration=\"{}\" start=\"0s\">\n",
                            rational_time(timeline.start),
                            rational_time(timeline.len())
                        ));
                        for ap in matching_audio {
                            render_attached_audio(ap, tl, &mut out);
                        }
                        out.push_str("            </gap>\n");
                    } else {
                        out.push_str(&format!(
                            "            <gap name=\"Gap\" offset=\"{}\" duration=\"{}\" start=\"0s\"/>\n",
                            rational_time(timeline.start),
                            rational_time(timeline.len())
                        ));
                    }
                }
            }
        }
    }

    out.push_str("          </spine>\n");
    out.push_str("        </sequence>\n");
    out.push_str("      </project>\n");
    out.push_str("    </event>\n");
    out.push_str("  </library>\n");
    out.push_str("</fcpxml>\n");

    out
}

fn render_attached_audio(piece: &Piece, tl: &Timeline, out: &mut String) {
    if let Piece::Play {
        media,
        timeline,
        source,
        speed,
        ..
    } = piece
    {
        let m = tl.media(*media);
        let file_name = m.map(|m| m.path.rsplit('/').next().unwrap_or(&m.path)).unwrap_or("audio");
        let has_speed = *speed != SPEED_NORMAL;

        if has_speed {
            out.push_str(&format!(
                "              <asset-clip ref=\"a{}\" lane=\"-1\" name=\"{}\" offset=\"{}\" duration=\"{}\" start=\"{}\">\n",
                media,
                xml_escape(file_name),
                rational_time(timeline.start),
                rational_time(timeline.len()),
                rational_time(source.start)
            ));
            out.push_str(&format!(
                "                <note>speed: {}‰ (conform-rate / timeMap)</note>\n",
                speed
            ));
            out.push_str("                <conform-rate scaleEnabled=\"0\"/>\n");
            out.push_str("              </asset-clip>\n");
        } else {
            out.push_str(&format!(
                "              <asset-clip ref=\"a{}\" lane=\"-1\" name=\"{}\" offset=\"{}\" duration=\"{}\" start=\"{}\"/>\n",
                media,
                xml_escape(file_name),
                rational_time(timeline.start),
                rational_time(timeline.len()),
                rational_time(source.start)
            ));
        }
    }
}

/// Convert timeline microseconds to standard non-drop frame `HH:MM:SS:FF` timecode.
fn timecode(t: Micros, num: u32, den: u32) -> String {
    let t = t.max(0);
    let fps_num = num.max(1) as u64;
    let fps_den = den.max(1) as u64;
    let total_frames = (t as u128 * fps_num as u128) / (1_000_000u128 * fps_den as u128);
    let nominal_fps = ((fps_num + fps_den / 2) / fps_den).max(1);
    let ff = (total_frames % nominal_fps as u128) as u64;
    let total_sec = (total_frames / nominal_fps as u128) as u64;
    let ss = total_sec % 60;
    let total_min = total_sec / 60;
    let mm = total_min % 60;
    let hh = total_min / 60;
    format!("{:02}:{:02}:{:02}:{:02}", hh, mm, ss, ff)
}

/// Generates standard CMX 3600 EDL for the timeline.
pub fn export_edl(tl: &Timeline, title: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", title));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    let primary_track = tl
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .or_else(|| tl.tracks.first());

    let Some(track) = primary_track else {
        return out;
    };

    let track_code = match track.kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
    };

    let mut event_idx = 1;
    for clip in track.clips.iter().filter(|c| c.enabled) {
        let src_in = timecode(clip.source.start, tl.fps_num, tl.fps_den);
        let src_out = timecode(clip.source.end, tl.fps_num, tl.fps_den);
        let rec_in = timecode(clip.at, tl.fps_num, tl.fps_den);
        let rec_out = timecode(clip.end(), tl.fps_num, tl.fps_den);

        let clip_name = tl
            .media(clip.media)
            .map(|m| m.path.rsplit('/').next().unwrap_or(&m.path))
            .unwrap_or("CLIP");

        out.push_str(&format!(
            "{:03}  AX       {:5} C        {} {} {} {}\n",
            event_idx, track_code, src_in, src_out, rec_in, rec_out
        ));

        if clip.speed != SPEED_NORMAL {
            let fps = (tl.fps_num.max(1) as f64 / tl.fps_den.max(1) as f64) * (clip.speed as f64 / 1000.0);
            out.push_str(&format!("M2   AX       {:>5.1}                {}\n", fps, src_in));
        }

        out.push_str(&format!("* FROM CLIP NAME: {}\n\n", clip_name));
        event_idx += 1;
    }

    out
}

fn srt_timestamp(us: Micros) -> String {
    let us = us.max(0);
    let total_ms = us / 1000;
    let ms = total_ms % 1000;
    let total_sec = total_ms / 1000;
    let ss = total_sec % 60;
    let total_min = total_sec / 60;
    let mm = total_min % 60;
    let hh = total_min / 60;
    format!("{:02}:{:02}:{:02},{:03}", hh, mm, ss, ms)
}

fn is_sentence_end(text: &str) -> bool {
    let trimmed = text.trim_end_matches(&['"', '\'', '”', '’', ')', ']', '}'][..]);
    trimmed.ends_with(&['.', '?', '!'][..])
}

struct MappedWord {
    text: String,
    start: Micros,
    end: Micros,
}

/// Maps currently playing words onto the edited timeline and formats valid SubRip `.srt`.
pub fn export_srt(tl: &Timeline, transcript: &Transcript, max_chars: usize) -> String {
    let limit = max_chars.max(12);

    let mut mapped_words: Vec<MappedWord> = Vec::new();
    for w in &transcript.words {
        let Some(clip) = tl
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.media == transcript.media && c.enabled)
            .find(|c| w.source.start >= c.source.start && w.source.start < c.source.end)
        else {
            continue;
        };

        let Some(start_tl) = clip.timeline_at(w.source.start) else {
            continue;
        };
        let end_src = w.source.end.min(clip.source.end);
        let end_tl = clip.timeline_at(end_src).unwrap_or_else(|| clip.end());

        if end_tl > start_tl {
            mapped_words.push(MappedWord {
                text: w.text.clone(),
                start: start_tl,
                end: end_tl,
            });
        }
    }

    mapped_words.sort_by_key(|w| w.start);

    if mapped_words.is_empty() {
        return String::new();
    }

    let mut cues: Vec<(Micros, Micros, String)> = Vec::new();
    let mut current_words: Vec<MappedWord> = Vec::new();

    for word in mapped_words {
        if current_words.is_empty() {
            current_words.push(word);
            continue;
        }

        let prev = current_words.last().unwrap();
        let gap = word.start.saturating_sub(prev.end);
        let prev_ends_sentence = is_sentence_end(&prev.text);

        let current_text_len: usize =
            current_words.iter().map(|w| w.text.len()).sum::<usize>() + (current_words.len() - 1);
        let would_exceed = current_text_len + 1 + word.text.len() > limit;

        if prev_ends_sentence || gap >= 400_000 || would_exceed {
            let cue_start = current_words.first().unwrap().start;
            let cue_end = current_words.last().unwrap().end;
            let text = current_words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            cues.push((cue_start, cue_end, text));
            current_words.clear();
        }

        current_words.push(word);
    }

    if !current_words.is_empty() {
        let cue_start = current_words.first().unwrap().start;
        let cue_end = current_words.last().unwrap().end;
        let text = current_words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        cues.push((cue_start, cue_end, text));
    }

    let mut out = String::new();
    for (i, (start, end, text)) in cues.into_iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_timestamp(start),
            srt_timestamp(end),
            text
        ));
    }

    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffAudit {
    pub media_paths: Vec<String>,
    pub speed_changed_clips: usize,
    pub muted_tracks: usize,
    pub disabled_clips: usize,
    pub markers_count: usize,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffBundle {
    pub fcpxml: String,
    pub otio: String,
    pub edl: String,
    pub srt: Option<String>,
    pub readme: String,
    pub audit: HandoffAudit,
}

fn build_readme(name: &str, tl: &Timeline, audit: &HandoffAudit, has_srt: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {} — Splicy Handoff Bundle\n\n", name));
    s.push_str("Exported from Splicy (SexOS editing core).\n\n");

    s.push_str("## Included Formats\n");
    s.push_str(&format!("- `{}.fcpxml`: Final Cut Pro XML 1.10 (Apple Final Cut Pro, DaVinci Resolve)\n", name));
    s.push_str(&format!("- `{}.otio`: OpenTimelineIO schema (DaVinci Resolve, Kdenlive)\n", name));
    s.push_str(&format!("- `{}.edl`: CMX 3600 Edit Decision List (Adobe Premiere Pro, DaVinci Resolve)\n", name));
    if has_srt {
        s.push_str(&format!("- `{}.srt`: SubRip subtitle captions aligned with the edited timeline\n", name));
    }
    s.push_str("\n");

    s.push_str("## How to Open in Your NLE\n\n");
    s.push_str("### DaVinci Resolve\n");
    s.push_str("1. Open DaVinci Resolve and create or open your target project.\n");
    s.push_str(&format!("2. Choose **File > Import > Timeline...** and select `{}.otio` or `{}.fcpxml`.\n", name, name));
    s.push_str("3. When prompted to link media, select the directory containing your source media files listed below.\n\n");

    s.push_str("### Final Cut Pro\n");
    s.push_str("1. Open Final Cut Pro.\n");
    s.push_str(&format!("2. Choose **File > Import > XML...** and select `{}.fcpxml`.\n", name));
    s.push_str("3. FCP will create an event with the complete sequence and attached audio tracks.\n\n");

    s.push_str("### Adobe Premiere Pro\n");
    s.push_str("1. Open Adobe Premiere Pro.\n");
    s.push_str(&format!("2. Choose **File > Import...** and select `{}.edl` or `{}.fcpxml`.\n", name, name));
    s.push_str(&format!("3. For EDL, confirm the sequence frame rate ({} fps).\n", tl.fps_num.max(1) / tl.fps_den.max(1)));
    s.push_str("4. Relink clips to source media.\n\n");

    s.push_str("## Included Media Files\n");
    if audit.media_paths.is_empty() {
        s.push_str("- *(No media files)*\n");
    } else {
        for path in &audit.media_paths {
            s.push_str(&format!("- `{}`\n", path));
        }
    }
    s.push_str("\n");

    s.push_str("## Audit & Review Checklist\n");
    s.push_str(&format!("- **Speed-changed clips**: {}\n", audit.speed_changed_clips));
    s.push_str(&format!("- **Muted tracks**: {}\n", audit.muted_tracks));
    s.push_str(&format!("- **Disabled clips**: {}\n", audit.disabled_clips));
    s.push_str(&format!("- **Markers**: {}\n", audit.markers_count));
    s.push_str("\n");

    s.push_str("### What Carried Over\n");
    s.push_str("- Timeline cuts, ripple edits, and multi-track joins.\n");
    s.push_str("- In/out source time ranges and sequence placements.\n");
    s.push_str("- Secondary audio track synchronization (`lane=\"-1\"` attachments).\n");
    if has_srt {
        s.push_str("- Subtitle cues synchronized to word cuts on the edited timeline.\n");
    }
    s.push_str("\n");

    s.push_str("### What Needs Review\n");
    if audit.notes.is_empty() {
        s.push_str("- None: Clean timeline with 1:1 speed, no muted tracks, and no disabled clips.\n");
    } else {
        for note in &audit.notes {
            s.push_str(&format!("- {}\n", note));
        }
    }

    s
}

/// Builds `.fcpxml`, `.otio`, `.edl`, optional `.srt`, `HandoffAudit`, and `README.md`.
pub fn build_handoff_bundle(
    tl: &Timeline,
    transcript: Option<&Transcript>,
    name: &str,
) -> HandoffBundle {
    let fcpxml = export_fcpxml(tl, name);
    let otio = crate::otio::export(tl, name);
    let edl = export_edl(tl, name);
    let srt = transcript.map(|tr| export_srt(tl, tr, 42));

    let mut media_paths = Vec::new();
    for m in &tl.media {
        if !media_paths.contains(&m.path) {
            media_paths.push(m.path.clone());
        }
    }

    let speed_changed_clips = tl
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.speed != SPEED_NORMAL)
        .count();

    let muted_tracks = tl.tracks.iter().filter(|t| t.muted).count();
    let disabled_clips = tl
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| !c.enabled)
        .count();
    let markers_count = tl.markers.len();

    let mut notes = Vec::new();
    if speed_changed_clips > 0 {
        notes.push(format!(
            "{} clip(s) use non-100% playback speed; NLEs may require conform / timeMap confirmation.",
            speed_changed_clips
        ));
    }
    if muted_tracks > 0 {
        notes.push(format!(
            "{} track(s) are muted; exported as silence / gaps.",
            muted_tracks
        ));
    }
    if disabled_clips > 0 {
        notes.push(format!(
            "{} clip(s) are disabled; excluded from playback.",
            disabled_clips
        ));
    }
    if markers_count > 0 {
        notes.push(format!(
            "{} marker(s) present on timeline.",
            markers_count
        ));
    }

    let audit = HandoffAudit {
        media_paths,
        speed_changed_clips,
        muted_tracks,
        disabled_clips,
        markers_count,
        notes,
    };

    let readme = build_readme(name, tl, &audit, srt.is_some());

    HandoffBundle {
        fcpxml,
        otio,
        edl,
        srt,
        readme,
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::transcript::Word;
    use crate::{MediaKind, Op, Range, SECOND};

    fn make_interview() -> (Timeline, u32, u32, u32) {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/interview.mp4", 20 * SECOND);
        let v = tl.add_track(TrackKind::Video, "Video 1");
        let a = tl.add_track(TrackKind::Audio, "Audio 1");
        tl.apply(&Op::AddClip {
            track: v,
            media: m,
            at: 0,
            source: Range::new(0, 20 * SECOND),
        })
        .unwrap();
        tl.apply(&Op::AddClip {
            track: a,
            media: m,
            at: 0,
            source: Range::new(0, 20 * SECOND),
        })
        .unwrap();
        (tl, m, v, a)
    }

    #[test]
    fn test_export_fcpxml() {
        let (mut tl, m, _v, _a) = make_interview();
        // Cut timeline: split at 5s and 15s, ripple delete 5s..10s
        tl.apply(&Op::RippleDelete {
            range: Range::new(5 * SECOND, 10 * SECOND),
        })
        .unwrap();
        // Speed up first clip to 1.5x (1500)
        let first_clip_id = tl.tracks[0].clips[0].id;
        tl.apply(&Op::SetSpeed {
            clip: first_clip_id,
            speed: 1500,
        })
        .unwrap();

        let xml = export_fcpxml(&tl, "InterviewEdit");
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<!DOCTYPE fcpxml>"));
        assert!(xml.contains("<fcpxml version=\"1.10\">"));
        assert!(xml.contains("<format id=\"r1\" frameDuration=\"1/30s\"/>"));
        assert!(xml.contains(&format!("<asset id=\"a{}\"", m)));
        assert!(xml.contains("src=\"file:///clips/interview.mp4\""));
        assert!(xml.contains("<project name=\"InterviewEdit\">"));
        assert!(xml.contains("<spine>"));
        assert!(xml.contains("lane=\"-1\"")); // Attached audio
        assert!(xml.contains("<note>speed: 1500‰ (conform-rate / timeMap)</note>"));
        assert!(xml.contains("<conform-rate scaleEnabled=\"0\"/>"));
    }

    #[test]
    fn test_export_edl() {
        let (mut tl, _m, _v, _a) = make_interview();
        tl.apply(&Op::Split {
            clip: tl.tracks[0].clips[0].id,
            at: 5 * SECOND,
        })
        .unwrap();
        let second_clip_id = tl.tracks[0].clips[1].id;
        tl.apply(&Op::SetSpeed {
            clip: second_clip_id,
            speed: 2000,
        })
        .unwrap();

        let edl = export_edl(&tl, "InterviewEDL");
        assert!(edl.contains("TITLE: InterviewEDL\nFCM: NON-DROP FRAME\n"));
        assert!(edl.contains("001  AX       V     C        00:00:00:00 00:00:05:00 00:00:00:00 00:00:05:00"));
        assert!(edl.contains("002  AX       V     C"));
        assert!(edl.contains("M2   AX        60.0                00:00:05:00"));
        assert!(edl.contains("* FROM CLIP NAME: interview.mp4"));
    }

    #[test]
    fn test_export_srt() {
        let (mut tl, m, _v, _a) = make_interview();
        // Ripple delete 2s..4s
        tl.apply(&Op::RippleDelete {
            range: Range::new(2 * SECOND, 4 * SECOND),
        })
        .unwrap();

        let transcript = Transcript {
            media: m,
            words: vec![
                Word {
                    text: "Hello".into(),
                    source: Range::new(500_000, 900_000),
                },
                Word {
                    text: "world.".into(),
                    source: Range::new(1_000_000, 1_500_000),
                },
                Word {
                    text: "Cut".into(),
                    source: Range::new(2_500_000, 3_000_000), // Cut out!
                },
                Word {
                    text: "Next".into(),
                    source: Range::new(5_000_000, 5_400_000), // Was 5s, now 3s on timeline!
                },
                Word {
                    text: "sentence!".into(),
                    source: Range::new(5_500_000, 6_000_000),
                },
            ],
        };

        let srt = export_srt(&tl, &transcript, 42);
        assert!(srt.contains("1\n00:00:00,500 --> 00:00:01,500\nHello world.\n\n"));
        // "Cut" was deleted by the ripple cut, so it shouldn't appear
        assert!(!srt.contains("Cut"));
        // Next cue broke at "world." punctuation and begins with "Next"
        assert!(srt.contains("2\n00:00:03,000 --> 00:00:04,000\nNext sentence!\n\n"));
    }

    #[test]
    fn test_build_handoff_bundle() {
        let (mut tl, m, _v, _a) = make_interview();
        tl.tracks[1].muted = true;
        tl.apply(&Op::AddMarker {
            at: 2 * SECOND,
            label: "Cut point".into(),
        })
        .unwrap();

        let transcript = Transcript {
            media: m,
            words: vec![Word {
                text: "Hi.".into(),
                source: Range::new(100_000, 400_000),
            }],
        };

        let bundle = build_handoff_bundle(&tl, Some(&transcript), "FullProject");
        assert_eq!(bundle.audit.muted_tracks, 1);
        assert_eq!(bundle.audit.markers_count, 1);
        assert_eq!(bundle.audit.media_paths, vec![String::from("/clips/interview.mp4")]);
        assert!(bundle.srt.is_some());
        assert!(bundle.readme.contains("DaVinci Resolve"));
        assert!(bundle.readme.contains("Final Cut Pro"));
        assert!(bundle.readme.contains("Adobe Premiere Pro"));
        assert!(bundle.readme.contains("/clips/interview.mp4"));
        assert!(bundle.fcpxml.contains("<fcpxml version=\"1.10\">"));
        assert!(bundle.otio.contains("OTIO_SCHEMA"));
        assert!(bundle.edl.contains("TITLE: FullProject"));
    }

    #[test]
    fn test_export_fcpxml_xml_escaping_and_gaps() {
        let mut tl = Timeline::new(24, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/A & B <test> \"quote\" 'apostrophe'.mp4", 10 * SECOND);
        let v = tl.add_track(TrackKind::Video, "Video & More");
        // Place clip at 2s..5s (leaving 0..2s as gap)
        tl.apply(&Op::AddClip {
            track: v,
            media: m,
            at: 2 * SECOND,
            source: Range::new(0, 3 * SECOND),
        })
        .unwrap();

        let xml = export_fcpxml(&tl, "Project <One> & \"Two\"");
        assert!(xml.contains("name=\"Project &lt;One&gt; &amp; &quot;Two&quot;\""));
        assert!(xml.contains("src=\"file:///clips/A &amp; B &lt;test&gt; &quot;quote&quot; &apos;apostrophe&apos;.mp4\""));
        assert!(xml.contains("<format id=\"r1\" frameDuration=\"1/24s\"/>"));
        assert!(xml.contains("<gap name=\"Gap\" offset=\"0s\" duration=\"2s\" start=\"0s\"/>"));
        assert!(xml.contains("<asset-clip ref=\"a1\""));
    }

    #[test]
    fn test_export_srt_max_chars_and_long_gap() {
        let (tl, m, _v, _a) = make_interview();
        let transcript = Transcript {
            media: m,
            words: vec![
                Word {
                    text: "This".into(),
                    source: Range::new(100_000, 200_000),
                },
                Word {
                    text: "is".into(),
                    source: Range::new(220_000, 300_000),
                },
                Word {
                    text: "a".into(),
                    source: Range::new(320_000, 350_000),
                },
                Word {
                    text: "very".into(),
                    source: Range::new(360_000, 450_000),
                },
                Word {
                    text: "long".into(),
                    source: Range::new(460_000, 550_000),
                },
                Word {
                    text: "phrase".into(),
                    source: Range::new(560_000, 700_000),
                },
                Word {
                    text: "exceeding".into(),
                    source: Range::new(710_000, 850_000),
                },
                Word {
                    text: "limit".into(),
                    source: Range::new(860_000, 950_000),
                },
                // Long gap of 500ms (>= 400_000 us)
                Word {
                    text: "Aftergap".into(),
                    source: Range::new(1_450_000, 1_800_000),
                },
            ],
        };

        // max_chars = 15
        let srt = export_srt(&tl, &transcript, 15);
        assert!(srt.contains("1\n"));
        assert!(srt.contains("2\n"));
        assert!(srt.contains("Aftergap"));
    }
}
