// SPDX-License-Identifier: GPL-3.0-only
use velvet::{
    model::{Direction, Mingle, Snags, State, TwinFlames},
    Game, Velvet,
};
use weft::{Button, Event, Key, Pointer, PointerKind, Response, Strings, Theme, Ui};
#[test]
fn snags_first_square_safe_and_exactly_six_mines() {
    for seed in 0..80 {
        for first in [0, 17, 35] {
            let mut g = Snags::new(seed);
            g.reveal(first);
            assert!(g.open[first]);
            assert!(!g.is_mine(first));
            assert_eq!((0..36).filter(|i| g.is_mine(*i)).count(), 6);
            assert_ne!(g.state, State::Lost);
        }
    }
}
#[test]
fn snags_flags_block_reveal_and_can_be_removed() {
    let mut g = Snags::new(4);
    g.flag(0);
    g.reveal(0);
    assert!(!g.open[0]);
    g.flag(0);
    g.reveal(0);
    assert!(g.open[0]);
    let flags = g.flags;
    g.flag(0);
    assert_eq!(g.flags, flags);
    g.flag(99);
    g.reveal(99);
}
#[test]
fn snags_win_loss_terminal_and_counts() {
    let mut g = Snags::new(27);
    g.reveal(0);
    for i in 0..36 {
        if !g.is_mine(i) {
            g.reveal(i);
        }
    }
    assert_eq!(g.state, State::Won);
    assert_eq!(g.open.iter().filter(|v| **v).count(), 30);
    let old = g.open;
    let mine = (0..36).find(|i| g.is_mine(*i)).unwrap();
    g.reveal(mine);
    assert_eq!(g.open, old);
    let mut g = Snags::new(27);
    g.reveal(0);
    let mine = (0..36).find(|i| g.is_mine(*i)).unwrap();
    g.reveal(mine);
    assert_eq!(g.state, State::Lost);
}
#[test]
fn snags_adjacency_never_wraps_between_rows() {
    let mut g = Snags::new(17);
    g.reveal(5);
    for i in 0..36 {
        let count = (0usize..36)
            .filter(|j| {
                *j != i
                    && (j / 6).abs_diff(i / 6) <= 1
                    && (j % 6).abs_diff(i % 6) <= 1
                    && g.is_mine(*j)
            })
            .count();
        assert_eq!(g.adjacent(i), count);
    }
}
#[test]
fn mingle_one_merge_per_tile_and_score() {
    let mut g = Mingle::new(1);
    g.cells = [0; 16];
    g.cells[..4].copy_from_slice(&[2, 2, 2, 2]);
    assert!(g.slide(Direction::Left));
    assert_eq!(&g.cells[..2], &[4, 4]);
    assert_eq!(g.score, 8);
    assert_eq!(g.moves, 1);
    assert_eq!(g.cells.iter().filter(|v| **v != 0).count(), 3);
}
#[test]
fn mingle_noop_does_not_spawn_or_increment_moves() {
    let mut g = Mingle::new(3);
    g.cells = [0; 16];
    g.cells[0] = 2;
    let old = g.cells;
    assert!(!g.slide(Direction::Left));
    assert_eq!(g.cells, old);
    assert_eq!(g.moves, 0);
}
#[test]
fn mingle_all_directions_merge() {
    for (d, pair, result) in [
        (Direction::Right, [0, 1], 3),
        (Direction::Left, [2, 3], 0),
        (Direction::Down, [0, 4], 12),
        (Direction::Up, [8, 12], 0),
    ] {
        let mut g = Mingle::new(1);
        g.cells = [0; 16];
        g.cells[pair[0]] = 4;
        g.cells[pair[1]] = 4;
        g.slide(d);
        assert_eq!(g.cells[result], 8);
        assert_eq!(g.score, 8);
    }
}
#[test]
fn mingle_loss_and_win() {
    let mut g = Mingle::new(1);
    g.cells = [2, 4, 2, 4, 4, 2, 4, 2, 2, 4, 2, 4, 4, 2, 4, 2];
    g.slide(Direction::Left);
    assert_eq!(g.state, State::Lost);
    let old = g.cells;
    assert!(!g.slide(Direction::Up));
    assert_eq!(g.cells, old);
    let mut g = Mingle::new(1);
    g.cells = [0; 16];
    g.cells[0] = 1024;
    g.cells[1] = 1024;
    g.slide(Direction::Left);
    assert_eq!(g.state, State::Won);
}
#[test]
fn twins_pairs_mismatch_reset_and_win() {
    let mut g = TwinFlames::new(10);
    for symbol in 0..8 {
        assert_eq!(g.cards.iter().filter(|x| **x == symbol).count(), 2);
    }
    let a = 0;
    let b = (1..16).find(|i| g.cards[*i] != g.cards[a]).unwrap();
    g.pick(a);
    g.pick(a);
    assert_eq!(g.turns, 0);
    g.pick(b);
    assert_eq!(g.turns, 1);
    assert!(g.visible(a) && g.visible(b));
    let c = (1..16).find(|i| *i != b).unwrap();
    g.pick(c);
    assert_eq!(g.first, Some(c));
    assert_eq!(g.second, None);
    g = TwinFlames::new(10);
    for symbol in 0..8 {
        let pair: Vec<_> = (0..16).filter(|i| g.cards[*i] == symbol).collect();
        g.pick(pair[0]);
        g.pick(pair[1]);
    }
    assert_eq!(g.state, State::Won);
    assert_eq!(g.turns, 8);
    g.pick(99);
}
fn ui() -> (Ui, Velvet, Vec<u32>) {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let g = Velvet::build(&mut ui, 42);
    let mut pixels = vec![0; 800 * 600];
    ui.paint(&mut pixels);
    (ui, g, pixels)
}
fn apply(ui: &mut Ui, g: &mut Velvet, event: Event) {
    for r in ui.handle(event) {
        g.on(ui, &r);
    }
}
fn click(ui: &mut Ui, g: &mut Velvet, id: weft::Id) {
    let c = ui.rect(id).center();
    for kind in [
        PointerKind::Move,
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        apply(
            ui,
            g,
            Event::Pointer(Pointer {
                x: c.x,
                y: c.y,
                kind,
            }),
        );
    }
}
#[test]
fn weft_pointer_reveal_and_keyboard_flag_restart() {
    let (mut ui, mut g, mut pixels) = ui();
    let id = g.cells[0];
    click(&mut ui, &mut g, id);
    assert!(g.snags.open[0]);
    apply(&mut ui, &mut g, Event::Key(Key::ctrl(0x21)));
    assert!(ui.is_checked(g.flag_mode));
    let i = (0..36).find(|i| !g.snags.open[*i]).unwrap();
    let id = g.cells[i];
    click(&mut ui, &mut g, id);
    assert!(g.snags.flags[i]);
    apply(&mut ui, &mut g, Event::Key(Key::ctrl(0x31)));
    assert!(g.snags.open.iter().all(|x| !*x));
    assert!(!ui.is_checked(g.flag_mode));
    ui.paint(&mut pixels);
    assert!(pixels.iter().any(|x| *x != pixels[0]));
}
#[test]
fn weft_switching_arrows_matching_and_restart() {
    let (mut ui, mut g, mut pixels) = ui();
    apply(&mut ui, &mut g, Event::Key(Key::ctrl(0x03)));
    assert_eq!(g.game, Game::Mingle);
    g.mingle.cells = [0; 16];
    g.mingle.cells[0] = 2;
    g.mingle.cells[1] = 2;
    apply(&mut ui, &mut g, Event::Key(Key::new(0x4B)));
    assert_eq!(g.mingle.score, 4);
    apply(&mut ui, &mut g, Event::Key(Key::ctrl(0x04)));
    assert_eq!(g.game, Game::TwinFlames);
    ui.paint(&mut pixels);
    let id = g.cells[0];
    click(&mut ui, &mut g, id);
    assert_eq!(g.twins.first, Some(0));
    let pair = (1..16)
        .find(|i| g.twins.cards[*i] == g.twins.cards[0])
        .unwrap();
    let id = g.cells[pair];
    click(&mut ui, &mut g, id);
    assert!(g.twins.matched[0]);
    let new = g.new_game;
    click(&mut ui, &mut g, new);
    assert!(g.twins.matched.iter().all(|x| !*x));
    assert!(!g.on(&mut ui, &Response::Command("unknown".into())));
}

#[test]
fn all_game_squares_fit_initial_window() {
    let mut ui = Ui::new(640, 400, Theme::light(), Strings::new("en"));
    let mut g = Velvet::build(&mut ui, 42);
    let mut pixels = vec![0u32; 640 * 400];
    for command in ["velvet.snags", "velvet.mingle", "velvet.twins"] {
        g.on(&mut ui, &Response::Command(command.into()));
        ui.paint(&mut pixels);
        for id in &g.cells {
            let r = ui.rect(*id);
            assert!(
                r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 640.0 && r.y + r.h <= 400.0,
                "{command}: {r:?}"
            );
        }
    }
}
