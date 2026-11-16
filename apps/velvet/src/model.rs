// SPDX-License-Identifier: GPL-3.0-only
//! Original game rules, independent of UI and host/guest services.
#[derive(Clone)]
pub struct Random(u64);
impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % n
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.next(i + 1);
            items.swap(i, j);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Playing,
    Won,
    Lost,
}
#[derive(Clone)]
pub struct Snags {
    mines: [bool; 36],
    pub open: [bool; 36],
    pub flags: [bool; 36],
    placed: bool,
    pub state: State,
    random: Random,
}
impl Snags {
    pub fn new(seed: u64) -> Self {
        Self {
            mines: [false; 36],
            open: [false; 36],
            flags: [false; 36],
            placed: false,
            state: State::Playing,
            random: Random::new(seed),
        }
    }
    fn neighbors(index: usize) -> impl Iterator<Item = usize> {
        (0..36).filter(move |j| {
            *j != index && (j / 6).abs_diff(index / 6) <= 1 && (j % 6).abs_diff(index % 6) <= 1
        })
    }
    pub fn adjacent(&self, i: usize) -> usize {
        if i >= 36 {
            return 0;
        }
        Self::neighbors(i).filter(|j| self.mines[*j]).count()
    }
    pub fn flag(&mut self, i: usize) {
        if i < 36 && !self.open[i] && self.state == State::Playing {
            self.flags[i] = !self.flags[i];
        }
    }
    pub fn reveal(&mut self, i: usize) {
        if i >= 36 || self.open[i] || self.flags[i] || self.state != State::Playing {
            return;
        }
        if !self.placed {
            let mut cells = [0usize; 35];
            let mut n = 0;
            for j in 0..36 {
                if j != i {
                    cells[n] = j;
                    n += 1;
                }
            }
            self.random.shuffle(&mut cells);
            for j in &cells[..6] {
                self.mines[*j] = true;
            }
            self.placed = true;
        }
        if self.mines[i] {
            self.open[i] = true;
            self.state = State::Lost;
            return;
        }
        // Fixed-size stack, each square enqueued once, no recursive flood fill.
        let mut queued = [false; 36];
        let mut stack = [0usize; 36];
        let mut n = 1;
        stack[0] = i;
        queued[i] = true;
        while n > 0 {
            n -= 1;
            let at = stack[n];
            self.open[at] = true;
            if self.adjacent(at) == 0 {
                for j in Self::neighbors(at) {
                    if !queued[j] && !self.flags[j] && !self.mines[j] {
                        queued[j] = true;
                        stack[n] = j;
                        n += 1;
                    }
                }
            }
        }
        if self.open.iter().filter(|x| **x).count() == 30 {
            self.state = State::Won;
        }
    }
    pub fn is_mine(&self, i: usize) -> bool {
        i < 36 && self.mines[i]
    }
}
#[derive(Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
#[derive(Clone)]
pub struct Mingle {
    pub cells: [u32; 16],
    pub score: u64,
    pub moves: u32,
    pub state: State,
    random: Random,
}
impl Mingle {
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            cells: [0; 16],
            score: 0,
            moves: 0,
            state: State::Playing,
            random: Random::new(seed),
        };
        s.spawn();
        s.spawn();
        s
    }
    fn spawn(&mut self) {
        let count = self.cells.iter().filter(|v| **v == 0).count();
        if count == 0 {
            return;
        }
        let mut at = self.random.next(count);
        for i in 0..16 {
            if self.cells[i] == 0 {
                if at == 0 {
                    self.cells[i] = if self.random.next(10) == 0 { 4 } else { 2 };
                    return;
                }
                at -= 1;
            }
        }
    }
    fn index(d: Direction, line: usize, pos: usize) -> usize {
        match d {
            Direction::Left => line * 4 + pos,
            Direction::Right => line * 4 + 3 - pos,
            Direction::Up => pos * 4 + line,
            Direction::Down => (3 - pos) * 4 + line,
        }
    }
    pub fn slide(&mut self, d: Direction) -> bool {
        if self.state != State::Playing {
            return false;
        }
        let old = self.cells;
        for line in 0..4 {
            let mut packed = [0u32; 4];
            let mut n = 0;
            for pos in 0..4 {
                let v = self.cells[Self::index(d, line, pos)];
                if v != 0 {
                    packed[n] = v;
                    n += 1;
                }
            }
            let mut result = [0u32; 4];
            let mut i = 0;
            let mut out = 0;
            while i < n {
                let mut v = packed[i];
                if i + 1 < n && packed[i + 1] == v && v.checked_mul(2).is_some() {
                    v *= 2;
                    self.score += v as u64;
                    i += 1;
                }
                result[out] = v;
                out += 1;
                i += 1;
            }
            for pos in 0..4 {
                self.cells[Self::index(d, line, pos)] = result[pos];
            }
        }
        if self.cells == old {
            self.update_state();
            return false;
        }
        self.moves += 1;
        self.spawn();
        self.update_state();
        true
    }
    fn update_state(&mut self) {
        if self.cells.iter().any(|v| *v >= 2048) {
            self.state = State::Won;
            return;
        }
        let possible = self.cells.contains(&0)
            || (0..16).any(|i| {
                (i % 4 < 3 && self.cells[i] == self.cells[i + 1])
                    || (i / 4 < 3 && self.cells[i] == self.cells[i + 4])
            });
        self.state = if possible {
            State::Playing
        } else {
            State::Lost
        };
    }
}
#[derive(Clone)]
pub struct TwinFlames {
    pub cards: [u8; 16],
    pub matched: [bool; 16],
    pub first: Option<usize>,
    pub second: Option<usize>,
    pub turns: u32,
    pub state: State,
}
impl TwinFlames {
    pub fn new(seed: u64) -> Self {
        let mut cards = core::array::from_fn(|i| (i / 2) as u8);
        Random::new(seed).shuffle(&mut cards);
        Self {
            cards,
            matched: [false; 16],
            first: None,
            second: None,
            turns: 0,
            state: State::Playing,
        }
    }
    pub fn pick(&mut self, i: usize) {
        if i >= 16 || self.state != State::Playing || self.matched[i] {
            return;
        }
        if self.second.is_some() {
            self.first = None;
            self.second = None;
        }
        if self.first == Some(i) {
            return;
        }
        if let Some(first) = self.first {
            self.turns += 1;
            self.second = Some(i);
            if self.cards[first] == self.cards[i] {
                self.matched[first] = true;
                self.matched[i] = true;
                self.first = None;
                self.second = None;
            }
            if self.matched.iter().all(|v| *v) {
                self.state = State::Won;
            }
        } else {
            self.first = Some(i);
        }
    }
    pub fn visible(&self, i: usize) -> bool {
        i < 16 && (self.matched[i] || self.first == Some(i) || self.second == Some(i))
    }
}
