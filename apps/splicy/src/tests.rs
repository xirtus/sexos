// SPDX-License-Identifier: GPL-3.0-only
use crate::view::{self, View, SELECT};
use crate::{words, Editor};
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
    for (w, h) in [(900usize, 600usize), (320, 200), (40, 30)] {
        let mut px = alloc::vec![0u32; w * h];
        view.paint(&mut px, w, h, &ed, Some((&picture, 64, 36)), 3 * SECOND, "");
        if w >= 320 {
            let a = view::areas(w as i32, h as i32).player;
            assert_eq!(px[(a.y + a.h / 2) as usize * w + (a.x + a.w / 2) as usize], 0x00ff0000, "picture in the player");
            assert!(px.contains(&SELECT), "selection drawn at {w}x{h}");
        }
    }
    ed.delete().unwrap();
    let mut px = alloc::vec![0u32; 900 * 600];
    view.paint(&mut px, 900, 600, &ed, None, 0, "Exported talk.webm");
    assert!(!px.contains(&SELECT), "nothing selected after the cut");
    let strip = view::areas(900, 600).strip;
    let join_x = strip.x + strip.w / 4; // 2 s of 8 s
    assert_eq!(px[(strip.y + 2) as usize * 900 + join_x as usize], view::ACCENT, "join marked on the strip");
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
    view.paint(&mut px, 800, 600, &ed, None, 0, "");
    assert_eq!(view.scroll, 0);
    view.scroll = 120;
    view.paint(&mut px, 800, 600, &ed, None, 0, "");
    assert_eq!(view.scroll, 120, "the scroll stays while the caret is still");
    ed.move_caret(1, false);
    view.paint(&mut px, 800, 600, &ed, None, 0, "");
    assert_eq!(view.scroll, 0, "moving the caret brings it back into view");
}
