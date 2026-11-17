// SPDX-License-Identifier: GPL-3.0-only
use crate::cache::{CacheConfig, CachedFrame, FrameCache};
use crate::compositor::{composite_cpu, BlendMode, LayerInput, LayerTransform, QualityMode};
use crate::media::{Library, Peaks, Thumb};
use crate::view::{self, View, SELECT};
use crate::vu::{VuStrip, VU_AMBER, VU_BG, VU_GREEN, VU_PEAK, VU_STRIP_H};
use crate::{words, AuditionMode, Editor};
use alloc::string::String;
use alloc::vec::Vec;
use sex_splice::export::{export_pictures, sound_pieces, Frames, Pictures};
use sex_splice::{Micros, SECOND};

const MS: Micros = 1000;

/// A 10 s talk at 10 fps: a greeting, a filler, a sentence to cut, a take said twice.
const TALK: &str = "\
# test talk
0 400 Hello
600 800 um
1000 1500 there.
2000 2400 This
2500 2900 part
3000 3500 goes.
4000 4400 Keep
4500 4900 this
5000 5500 one.
7000 7400 Keep
7500 7900 this
8000 8500 one.
";

fn talk() -> Editor {
    Editor::open("talk.webm", 10 * SECOND, (10, 1), true, words::parse(TALK).unwrap())
}

/// Pictures that remember which source times were asked for.
#[derive(Default)]
struct Asked(Vec<Micros>);
impl Pictures for Asked {
    type Error = ();
    fn picture(&mut self, _: u32, source: Micros, out: &mut [u32]) -> Result<(), ()> {
        self.0.push(source);
        out.fill((source / (100 * MS)) as u32);
        Ok(())
    }
}

#[derive(Default)]
struct Count(u64);
impl Frames for Count {
    type Error = ();
    fn frame(&mut self, _: &[u32], _: Micros) -> Result<(), ()> {
        self.0 += 1;
        Ok(())
    }
}

/// Click the middle of word `i` where the window paints it.
fn click(view: &mut View, ed: &mut Editor, w: i32, h: i32, i: usize) {
    let a = view::areas(w, h).text;
    let p = view.layout(ed, a.w)[i];
    let (x, y) = (a.x + p.x + p.w / 2, a.y + p.y - view.scroll + view.line_height() / 2);
    let hit = view.word_at(ed, w, h, x, y).expect("a word under the click");
    ed.place(hit, false);
}

#[test]
fn deleting_a_sentence_on_screen_removes_its_frames_and_sound() {
    let mut ed = talk();
    let mut view = View::default();
    let (w, h) = (900, 600);
    click(&mut view, &mut ed, w, h, 4); // "part"
    assert_eq!(ed.caret, 4);
    ed.select_sentence();
    assert_eq!(ed.selection(), Some((3, 5)));
    assert!(ed.delete().unwrap());

    // "This part goes." ran from 2.0 s to where "Keep" starts at 4.0 s.
    let tl = ed.timeline();
    assert_eq!(tl.duration(), 8 * SECOND);
    let (mut asked, mut out) = (Asked::default(), Count::default());
    assert_eq!(export_pictures(tl, None, 4, &mut asked, &mut out).unwrap(), 80);
    assert!(asked.0.iter().all(|s| !(2 * SECOND..4 * SECOND).contains(s)), "a cut frame was shown");
    assert_eq!(asked.0[19], 1900 * MS);
    assert_eq!(asked.0[20], 4 * SECOND, "the frame after the join is the first one after the cut");

    let pieces = sound_pieces(tl, 48_000);
    assert_eq!(pieces.len(), 2);
    assert_eq!((pieces[0].at, pieces[0].samples, pieces[0].source), (0, 96_000, 0));
    assert_eq!((pieces[1].at, pieces[1].samples, pieces[1].source), (96_000, 288_000, 192_000));

    // The words show as cut, the join is where they were, and `\` plays around it.
    let playing = ed.playing();
    assert_eq!(playing, [true, true, true, false, false, false, true, true, true, true, true, true]);
    assert_eq!(ed.joins(), [2 * SECOND]);
    assert_eq!(ed.join_preview().map(|r| (r.start, r.end)), Some((0, 4 * SECOND)));
    assert_eq!(ed.history.versions().last().unwrap().summary, "Cut \"This part goes.\"");

    assert!(ed.undo());
    assert_eq!(ed.timeline().duration(), 10 * SECOND);
    assert!(ed.playing().iter().all(|p| *p));
    assert!(ed.redo());
    assert_eq!(ed.timeline().duration(), 8 * SECOND);
}

#[test]
fn deleting_words_already_cut_changes_nothing() {
    let mut ed = talk();
    ed.place(3, false);
    ed.place(5, true);
    assert!(ed.delete().unwrap());
    let versions = ed.history.versions().len();
    ed.place(3, false);
    ed.place(4, true);
    assert!(!ed.delete().unwrap());
    assert_eq!(ed.history.versions().len(), versions);
}

#[test]
fn clean_up_suggestions_wait_and_still_land_after_other_cuts() {
    let mut ed = talk();
    assert_eq!(ed.suggest_cleanup(), 3);
    let summaries: Vec<String> = ed.pending().iter().map(|id| ed.suggestions.get(*id).unwrap().summary.clone()).collect();
    assert_eq!(summaries, ["Remove the first take of \"Keep this one.\"", "Remove 1 filler word", "Shorten 1 long pause"]);
    assert_eq!(ed.timeline().duration(), 10 * SECOND, "suggestions change nothing until taken");

    // Cut a sentence by hand first; the retake card still finds its words.
    ed.place(3, false);
    ed.select_sentence();
    ed.delete().unwrap();
    assert!(ed.accept().unwrap());
    // First take ran from 4.0 s to the retake at 7.0 s: 3 s more gone.
    assert_eq!(ed.timeline().duration(), 5 * SECOND);
    let playing = ed.playing();
    assert!(!playing[6] && !playing[7] && !playing[8] && playing[9] && playing[10] && playing[11]);

    // Skip the filler, take the pause card: the 1.5 s pause was inside the cut take, so it
    // has nothing left to shorten and the edit stays as it is.
    assert!(ed.skip());
    assert_eq!(ed.pending().len(), 1);
    let before = ed.timeline().duration();
    let _ = ed.accept();
    assert_eq!(ed.timeline().duration(), before);
    assert_eq!(ed.decided().1, 1);
    assert!(ed.playing()[1], "the skipped filler still plays");
}

#[test]
fn caret_and_selection_stay_inside_the_words() {
    let mut ed = talk();
    ed.move_caret(-3, false);
    assert_eq!(ed.caret, 0);
    ed.move_caret(2, true);
    assert_eq!(ed.selection(), Some((0, 2)));
    ed.move_caret(100, false);
    assert_eq!((ed.caret, ed.anchor), (11, None));
    ed.select_sentence();
    assert_eq!(ed.selection(), Some((9, 11)));
    ed.place(0, false);
    ed.select_sentence();
    assert_eq!(ed.selection(), Some((0, 2)));
}

#[test]
fn transcript_file_round_trips_and_refuses_bad_lines() {
    let t = words::parse(TALK).unwrap();
    assert_eq!(t.words.len(), 12);
    assert_eq!(t.words[2].text, "there.");
    assert_eq!((t.words[2].source.start, t.words[2].source.end), (1000 * MS, 1500 * MS));
    assert_eq!(words::parse(&words::write(&t)).unwrap(), t);
    assert_eq!(words::parse("0 100 a\n50 200 b\n"), Err(words::BadLine(2)), "overlap");
    assert_eq!(words::parse("x 100 a\n"), Err(words::BadLine(1)));
    assert_eq!(words::parse("10 5 a\n"), Err(words::BadLine(1)));
    assert_eq!(words::parse("0 100\n"), Err(words::BadLine(1)));
    assert_eq!(words::parse("0 100 New York\n").unwrap().words[0].text, "New York");
    assert_eq!(words::beside("talk.webm"), "talk.words");
    assert_eq!(words::beside("my.dir/talk"), "my.dir/talk.words");
}

#[test]
fn layout_wraps_and_clicks_find_words() {
    let words = ["aaaa", "bb", "cccccc", "d"];
    // Every character 10 px, a space 10 px, 100 px wide, lines 20 px apart.
    let placed = view::layout(&words, 100, 20, |s| s.chars().count() as i32 * 10);
    let at: Vec<(i32, i32)> = placed.iter().map(|p| (p.x, p.y)).collect();
    assert_eq!(at, [(0, 0), (50, 0), (0, 20), (70, 20)]);
    assert_eq!(view::hit(&placed, 55, 5, 20), Some(1));
    assert_eq!(view::hit(&placed, 95, 5, 20), Some(1), "past the end of a line: its last word");
    assert_eq!(view::hit(&placed, 5, 25, 20), Some(2));
    assert_eq!(view::hit(&placed, 5, 45, 20), None);
}

#[test]
fn painting_shows_the_picture_selection_and_cuts_at_any_size() {
    let mut ed = talk();
    ed.suggest_cleanup();
    ed.place(3, false);
    ed.place(5, true);
    let picture: Vec<u32> = alloc::vec![0x00ff0000; 64 * 36];
    let mut view = View::default();
    for (w, h) in [(1280usize, 800usize), (640, 400), (40, 30)] {
        let mut px = alloc::vec![0u32; w * h];
        view.paint(&mut px, w, h, &ed, &Library::default(), Some((&picture, 64, 36)), 3 * SECOND, "");
        if w >= 640 {
            let a = view::areas(w as i32, h as i32).player;
            assert_eq!(px[(a.y + a.h / 2) as usize * w + (a.x + a.w / 2) as usize], 0x00ff0000, "picture in the player");
            assert!(px.contains(&SELECT), "selection drawn at {w}x{h}");
        }
    }
    ed.delete().unwrap();
    let mut px = alloc::vec![0u32; 900 * 600];
    view.paint(&mut px, 900, 600, &ed, &Library::default(), None, 0, "Exported talk.webm");
    assert!(!px.contains(&SELECT), "nothing selected after the cut");
    let tracks = view::areas(900, 600).tracks;
    let join_x = tracks.x + (2 * SECOND / view.us_per_px(ed.timeline(), 900)) as i32;
    assert_eq!(px[(tracks.y + 30) as usize * 900 + join_x as usize], view::ACCENT, "join marked on the tracks");
}

#[test]
fn a_wheel_scroll_stays_until_the_caret_moves() {
    // A transcript taller than the pane.
    let mut text = String::new();
    for i in 0..400 {
        text.push_str(&alloc::format!("{} {} word{}\n", i * 300, i * 300 + 200, i));
    }
    let mut ed = Editor::open("long.webm", 130 * SECOND, (30, 1), true, words::parse(&text).unwrap());
    let mut view = View::default();
    let mut px = alloc::vec![0u32; 800 * 600];
    view.paint(&mut px, 800, 600, &ed, &Library::default(), None, 0, "");
    assert_eq!(view.scroll, 0);
    view.scroll = 120;
    view.paint(&mut px, 800, 600, &ed, &Library::default(), None, 0, "");
    assert_eq!(view.scroll, 120, "the scroll stays while the caret is still");
    ed.move_caret(1, false);
    view.paint(&mut px, 800, 600, &ed, &Library::default(), None, 0, "");
    assert_eq!(view.scroll, 0, "moving the caret brings it back into view");
}

/// A library for the talk: a picture every half second whose colour says when it was
/// taken, and sound that swells and fades every second.
fn talk_library(ed: &Editor) -> Library {
    let media = ed.transcript.media;
    let mut lib = Library::default();
    for i in 0..20u32 {
        let colour = 0x00102030 + i * 0x0a0806;
        lib.add_thumb(media, Thumb { at: i as Micros * 500 * MS, w: 16, h: 9, px: alloc::vec![colour; 16 * 9] });
    }
    let pcm: Vec<[i16; 2]> = (0..48_000 * 10).map(|n| {
        let swell = (n % 48_000) as i32 * 2 - 48_000;
        let v = (24_000 - swell.abs() / 2) as i16;
        [v, if n % 2 == 0 { v } else { -v }]
    }).collect();
    lib.peaks.insert(media, Peaks::from_pcm(&pcm, 48_000, 10 * MS));
    lib
}

#[test]
fn timeline_shows_filmstrips_waveforms_ruler_and_playhead() {
    let mut ed = talk();
    let lib = talk_library(&ed);
    assert_eq!(lib.thumb(ed.transcript.media, 740 * MS).unwrap().at, 500 * MS);
    assert_eq!(lib.thumb(ed.transcript.media, 760 * MS).unwrap().at, 1000 * MS);
    assert_eq!(view::timecode(61 * SECOND + 500 * MS, (30, 1)), "00:01:01:15");
    assert_eq!(view::timecode(60 * SECOND, (30000, 1001)), "00:00:59:28", "frame 1798, non-drop");
    assert_eq!(view::ruler_step(10_000, 110), 2 * SECOND);

    ed.place(3, false);
    ed.select_sentence();
    ed.delete().unwrap();
    let mut view = View::default();
    let (w, h) = (1280usize, 800usize);
    let mut px = alloc::vec![0u32; w * h];
    ed.suggest_cleanup();
    let picture: Vec<u32> = (0..320 * 180).map(|i| {
        let (x, y) = ((i % 320) as u32, (i / 320) as u32);
        (x * 255 / 320) << 16 | (y * 255 / 180) << 8 | 0x60
    }).collect();
    view.paint(&mut px, w, h, &ed, &lib, Some((&picture, 320, 180)), 3 * SECOND, "");
    let a = view::areas(w as i32, h as i32);
    let upp = view.us_per_px(ed.timeline(), w as i32);
    let row = |y: i32| &px[y as usize * w + a.tracks.x as usize..(y as usize + 1) * w];

    // Picture track: each clip's filmstrip starts with the picture at its first frame,
    // so the clip after the cut opens on the 4.0 s picture, not the 2.0 s one.
    let vy = a.tracks.y + 4 + 40;
    let x_at = |t: Micros| (t / upp) as usize;
    assert_eq!(row(vy)[x_at(0) + 3], 0x00102030, "the first clip opens on the 0 s picture");
    assert_eq!(row(vy)[x_at(2 * SECOND) + 3], 0x00102030 + 8 * 0x0a0806, "after the join: the 4.0 s picture");

    // Sound track: the waveform is lighter than its clip and taller where it is louder.
    let sound_top = a.tracks.y + 4 + view::row_height(sex_splice::TrackKind::Video, 2, a.tracks.h - 8);
    let wave = |x: usize| (sound_top..sound_top + 46).filter(|y| row(*y)[x] == sex_office::mix(view::SOUND_COLOURS[0], 0xffffff, 110)).count();
    assert!(wave(x_at(500 * MS)) > wave(x_at(10 * MS)) + 10, "louder mid-second than at its start");

    // Ruler labels and the playhead.
    assert!(row(a.ruler.y + 2).contains(&view::PLAYHEAD));
    assert_eq!(row(a.tracks.y + 10)[x_at(3 * SECOND)], view::PLAYHEAD);

    // Zoom in around the playhead and back out to fit.
    view.zoom_by(ed.timeline(), w as i32, 1, 3 * SECOND);
    assert_eq!(view.zoom, upp / 2);
    view.zoom_by(ed.timeline(), w as i32, -1, 3 * SECOND);
    assert_eq!(view.zoom, 0);

    // A click on the tracks moves to that time, and the transcript follows.
    let t = view.time_at(ed.timeline(), w as i32, h as i32, a.tracks.x + x_at(2600 * MS) as i32, a.tracks.y + 10).unwrap();
    assert!((t - 2600 * MS).abs() <= upp);
    assert_eq!(ed.word_at_time(t), Some(7), "2.6 s on the edit is 4.6 s of source: \"this\"");

    if let Ok(path) = std::env::var("SPLICY_SHOT") {
        let mut ppm = alloc::format!("P6\n{w} {h}\n255\n").into_bytes();
        for p in &px {
            ppm.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        std::fs::write(path, ppm).unwrap();
    }
}

#[test]
fn frame_cache_dual_limits_low_memory_mode_and_evict_media() {
    assert_eq!(CacheConfig::standard().max_entries, 256);
    assert_eq!(CacheConfig::standard().max_bytes, 128 * 1024 * 1024);
    assert_eq!(CacheConfig::low_memory().max_entries, 64);
    assert_eq!(CacheConfig::low_memory().max_bytes, 16 * 1024 * 1024);

    // Test dual-limit eviction (entries limit AND byte limit)
    // 3 entries max, 300 bytes max
    let mut cache = FrameCache::new(CacheConfig { max_entries: 3, max_bytes: 300 });

    // Each frame is 5x5 pixels = 25 pixels = 100 bytes
    let make_frame = |media: u32, source_us: Micros| -> CachedFrame {
        CachedFrame::new(media, source_us, 5, 5, false, alloc::vec![0x00ffffff; 25])
    };

    cache.insert(make_frame(1, 1000 * MS));
    cache.insert(make_frame(1, 2000 * MS));
    cache.insert(make_frame(1, 3000 * MS));
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.total_bytes, 300);
    assert_eq!(cache.evictions, 0);

    // Touch frame 1 (at 1000 MS), making frame 2 (at 2000 MS) the LRU entry
    assert!(cache.get(1, 1000 * MS, 50 * MS, false).is_some());
    assert_eq!(cache.hits, 1);

    // Insert frame 4: should evict frame 2
    cache.insert(make_frame(1, 4000 * MS));
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.total_bytes, 300);
    assert_eq!(cache.evictions, 1);
    assert!(cache.get(1, 2000 * MS, 50 * MS, false).is_none(), "frame 2 was evicted as LRU");
    assert_eq!(cache.misses, 1);
    assert!(cache.get(1, 1000 * MS, 50 * MS, false).is_some(), "frame 1 survived because touched");

    // Single frame exceeding max_bytes must be refused cleanly without panic
    let oversized = CachedFrame::new(1, 5000 * MS, 10, 10, false, alloc::vec![0; 100]); // 400 bytes > 300 bytes
    cache.insert(oversized);
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.total_bytes, 300);

    // Test low memory mode live eviction
    let mut big_cache = FrameCache::default(); // Standard: 256 entries
    for i in 0..70 {
        big_cache.insert(make_frame(1, i * 100 * MS));
    }
    assert_eq!(big_cache.len(), 70);
    // Toggling low memory mode (64 entries limit) must immediately evict excess 6 entries
    big_cache.set_low_memory_mode(true);
    assert_eq!(big_cache.len(), 64);
    assert_eq!(big_cache.evictions, 6);
    assert_eq!(big_cache.config, CacheConfig::low_memory());

    // Switch back to standard mode
    big_cache.set_low_memory_mode(false);
    assert_eq!(big_cache.config, CacheConfig::standard());

    // Test evict_media: purge all frames of a closed clip
    big_cache.insert(make_frame(2, 100 * MS));
    big_cache.insert(make_frame(2, 200 * MS));
    assert!(big_cache.get(2, 100 * MS, 10, false).is_some());
    let before_evict = big_cache.evictions;
    big_cache.evict_media(2);
    assert_eq!(big_cache.evictions, before_evict + 2);
    assert!(big_cache.get(2, 100 * MS, 10, false).is_none());
}

#[test]
fn compositor_quarter_vs_full_res_dimensions_and_blending() {
    let w = 320usize;
    let h = 180usize;

    let red_layer_px = alloc::vec![0x00ff0000; w * h];
    let layer = LayerInput {
        pixels: &red_layer_px,
        width: w,
        height: h,
        transform: LayerTransform::default(),
    };

    // Full-res on paused
    let full = composite_cpu(w, h, QualityMode::PausedFull, 0x00000000, &[layer]);
    assert_eq!(full.width, 320);
    assert_eq!(full.height, 180);
    assert_eq!(full.pixels.len(), 320 * 180);
    assert_eq!(full.pixels[0], 0x00ff0000);

    let layer_quarter = LayerInput {
        pixels: &red_layer_px,
        width: w,
        height: h,
        transform: LayerTransform::default(),
    };

    // Quarter-res on playback
    let quarter = composite_cpu(w, h, QualityMode::PlaybackQuarter, 0x00000000, &[layer_quarter]);
    assert_eq!(quarter.width, 80);
    assert_eq!(quarter.height, 45);
    assert_eq!(quarter.pixels.len(), 80 * 45);
    assert_eq!(quarter.pixels[0], 0x00ff0000);

    // 16x fewer pixels during playback!
    assert_eq!(full.pixels.len() / quarter.pixels.len(), 16);

    // Test Opacity blending: 50% opacity red over black
    let half_alpha_transform = LayerTransform {
        opacity_per_mille: 500,
        ..LayerTransform::default()
    };
    let half_red = LayerInput {
        pixels: &red_layer_px,
        width: 10,
        height: 10,
        transform: half_alpha_transform,
    };
    let blended_half = composite_cpu(10, 10, QualityMode::PausedFull, 0x00000000, &[half_red]);
    let r_ch = (blended_half.pixels[0] >> 16) & 0xff;
    assert!((120..=135).contains(&r_ch), "50% opacity red should have ~127 red value, got {r_ch}");

    // Test Blend Modes:
    let fg_px = [0x00404040];
    let multiply_layer = LayerInput {
        pixels: &fg_px,
        width: 1,
        height: 1,
        transform: LayerTransform {
            blend: BlendMode::Multiply,
            ..LayerTransform::default()
        },
    };
    let mult_out = composite_cpu(1, 1, QualityMode::PausedFull, 0x00808080, &[multiply_layer]);
    // 0x40 (64) * 0x80 (128) / 255 = 32 (0x20)
    let mult_val = mult_out.pixels[0] & 0xff;
    assert!((30..=34).contains(&mult_val), "multiply result ~32, got {mult_val}");

    let add_layer = LayerInput {
        pixels: &[0x00505050],
        width: 1,
        height: 1,
        transform: LayerTransform {
            blend: BlendMode::Add,
            ..LayerTransform::default()
        },
    };
    let add_out = composite_cpu(1, 1, QualityMode::PausedFull, 0x00505050, &[add_layer]);
    assert_eq!(add_out.pixels[0], 0x00a0a0a0);
}

#[test]
fn vu_meter_strip_areas_decay_mute_and_canvas_paint() {
    let (w, h) = (1280, 800);
    let a_off = view::areas_with_vu(w, h, false);
    let a_on = view::areas_with_vu(w, h, true);

    // show_vu == true carves VU_STRIP_H out of bottom timeline height,
    // NEVER reducing player.w by a single pixel!
    assert_eq!(a_off.player.w, a_on.player.w, "player.w must stay completely identical");
    assert_eq!(a_off.player.h, a_on.player.h, "player.h must stay completely identical");
    assert_eq!(a_off.player.x, a_on.player.x);
    assert_eq!(a_off.player.y, a_on.player.y);
    assert_eq!(a_on.vu.h, VU_STRIP_H);
    assert_eq!(a_on.vu.y, a_on.status.y - VU_STRIP_H);
    assert_eq!(a_on.tracks.h, a_off.tracks.h - VU_STRIP_H);

    // Test Peak-hold decay and mute handling
    let ed = talk();
    let lib = talk_library(&ed);
    let mut vu = VuStrip::default();

    // At 500 ms in talk_library, audio is loud (~200)
    vu.update_from_library(ed.timeline(), &lib, 500 * MS, 50 * MS);
    assert!(!vu.tracks.is_empty());
    let initial_level = vu.tracks[0].level;
    let initial_hold = vu.tracks[0].peak_hold;
    assert!(initial_level > 100, "audio active, level = {initial_level}");
    assert_eq!(initial_level, initial_hold);

    // Muting the audio track drops level to 0 and decays peak_hold by 6 each update
    let mut muted_tl = ed.timeline().clone();
    let audio_track = muted_tl.tracks.iter_mut().find(|t| t.kind == sex_splice::TrackKind::Audio).unwrap();
    audio_track.muted = true;

    vu.update_from_library(&muted_tl, &lib, 500 * MS, 50 * MS);
    assert_eq!(vu.tracks[0].level, 0, "muted track must produce 0 level");
    assert_eq!(vu.tracks[0].peak_hold, initial_hold.saturating_sub(6), "peak hold decays smoothly by 6");

    vu.update_from_library(&muted_tl, &lib, 500 * MS, 50 * MS);
    assert_eq!(vu.tracks[0].peak_hold, initial_hold.saturating_sub(12));

    // Test clipping indicator >= 248
    let mut hot_vu = VuStrip::default();
    hot_vu.tracks.push(crate::vu::TrackMeter {
        name: String::from("A1"),
        level: 250,
        peak_hold: 250,
        clipping: true,
    });
    assert!(hot_vu.tracks[0].clipping);

    // Paint VU strip pixels on canvas
    let mut px = alloc::vec![0u32; (w * h) as usize];
    let mut view = View::default();
    view.vu.visible = true;
    view.paint(&mut px, w as usize, h as usize, &ed, &lib, None, 500 * MS, "");

    // Verify VU strip area contains meter pixels (VU_BG, VU_LINE, green/amber)
    let vu_row_start = (a_on.vu.y + 12) as usize * w as usize;
    let vu_slice = &px[vu_row_start..vu_row_start + w as usize];
    assert!(vu_slice.contains(&VU_BG), "VU background painted");
    assert!(vu_slice.contains(&VU_GREEN) || vu_slice.contains(&VU_AMBER) || vu_slice.contains(&VU_PEAK), "VU level segments painted");
}

#[test]
fn audition_cycle_and_nearest_join_removed_range() {
    let mut ed = talk();
    assert_eq!(ed.audition, AuditionMode::HearJoin);

    // Cycle audition toggles between HearJoin and HearRemoved
    assert_eq!(ed.cycle_audition(), AuditionMode::HearRemoved);
    assert_eq!(ed.audition, AuditionMode::HearRemoved);
    assert_eq!(ed.cycle_audition(), AuditionMode::HearJoin);
    assert_eq!(ed.audition, AuditionMode::HearJoin);

    // Before any cut, there are no joins with cut content
    assert_eq!(ed.removed_at_nearest_join(), None);

    // Delete "This part goes." (words 3..=5, runs 2.0 s to 4.0 s)
    ed.place(4, false); // caret on "part"
    ed.select_sentence();
    assert!(ed.delete().unwrap());

    // Join is created at 2.0 s where 2.0 s..4.0 s of source was removed
    let (media, removed_range) = ed.removed_at_nearest_join().expect("removed range at nearest join");
    assert_eq!(media, ed.transcript.media);
    assert_eq!(removed_range.start, 2 * SECOND);
    assert_eq!(removed_range.end, 4 * SECOND);
    assert_eq!(removed_range.len(), 2 * SECOND);
}
