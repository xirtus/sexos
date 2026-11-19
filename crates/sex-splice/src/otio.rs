//! OpenTimelineIO export, so a Splicy edit can continue in DaVinci Resolve, Kdenlive or any
//! other editor that reads `.otio`. Times are written in frames at the timeline's rate.

use crate::render::{plan, Piece};
use crate::{Micros, Timeline, TrackKind};
use alloc::format;
use alloc::string::String;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Frames as a decimal with up to six places, exact for whole frames.
fn frames(t: Micros, num: u32, den: u32) -> String {
    let scaled = t as i128 * num as i128 * 1_000_000 / (den as i128 * 1_000_000);
    // `scaled` is frames * 1e6.
    let whole = scaled / 1_000_000;
    let frac = (scaled % 1_000_000).abs();
    if frac == 0 {
        format!("{}.0", whole)
    } else {
        let s = format!("{:06}", frac);
        format!("{}.{}", whole, s.trim_end_matches('0'))
    }
}

fn rate(num: u32, den: u32) -> String {
    if num % den == 0 {
        format!("{}.0", num / den)
    } else {
        format!("{:.6}", num as f64 / den as f64)
    }
}

fn time_range(start: Micros, dur: Micros, num: u32, den: u32) -> String {
    let r = rate(num, den);
    format!(
        "{{\"OTIO_SCHEMA\":\"TimeRange.1\",\"start_time\":{{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":{r},\"value\":{}}},\"duration\":{{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":{r},\"value\":{}}}}}",
        frames(start, num, den),
        frames(dur, num, den)
    )
}

/// The timeline as an OpenTimelineIO document.
pub fn export(tl: &Timeline, name: &str) -> String {
    let (num, den) = (tl.fps_num.max(1), tl.fps_den.max(1));
    let mut tracks = String::new();
    for (ti, tp) in plan(tl).iter().enumerate() {
        let track = tl.tracks.iter().find(|t| t.id == tp.track).expect("plan track");
        let mut children = String::new();
        for (i, p) in tp.pieces.iter().enumerate() {
            if i > 0 {
                children.push(',');
            }
            match p {
                Piece::Gap { timeline } => children.push_str(&format!(
                    "{{\"OTIO_SCHEMA\":\"Gap.1\",\"name\":\"\",\"source_range\":{}}}",
                    time_range(0, timeline.len(), num, den)
                )),
                Piece::Play { media, source, timeline, .. } => {
                    let m = tl.media(*media).expect("clip media");
                    let file = m.path.rsplit('/').next().unwrap_or(&m.path);
                    // A sped-up clip's source span differs from its time on the timeline;
                    // OTIO carries speed as an effect, which comes with V11. Export the
                    // span it occupies so the edit's timing survives.
                    children.push_str(&format!(
                        "{{\"OTIO_SCHEMA\":\"Clip.2\",\"name\":\"{}\",\"source_range\":{},\"media_references\":{{\"DEFAULT_MEDIA\":{{\"OTIO_SCHEMA\":\"ExternalReference.1\",\"target_url\":\"{}\"}}}},\"active_media_reference_key\":\"DEFAULT_MEDIA\"}}",
                        esc(file),
                        time_range(source.start, timeline.len(), num, den),
                        esc(&m.path)
                    ))
                }
            }
        }
        if ti > 0 {
            tracks.push(',');
        }
        let kind = match track.kind {
            TrackKind::Video => "Video",
            TrackKind::Audio => "Audio",
        };
        tracks.push_str(&format!(
            "{{\"OTIO_SCHEMA\":\"Track.1\",\"name\":\"{}\",\"kind\":\"{}\",\"children\":[{}]}}",
            esc(&track.name),
            kind,
            children
        ));
    }
    format!(
        "{{\"OTIO_SCHEMA\":\"Timeline.1\",\"name\":\"{}\",\"tracks\":{{\"OTIO_SCHEMA\":\"Stack.1\",\"name\":\"tracks\",\"children\":[{}]}}}}\n",
        esc(name),
        tracks
    )
}
