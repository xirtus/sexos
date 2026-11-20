// SPDX-License-Identifier: GPL-3.0-only
//! Voyeur, the system search, in the launcher. Once the user types, the
//! launcher shows Voyeur's merged results instead of the command rows.
//!
//! Built-in providers answer from what the Shell already holds: Abacus
//! (`2+2`), the launcher's apps and actions, this workspace's windows,
//! clipboard history, settings page names and emoji. Programs registered
//! with `sex_pdx::search` answer over their own route; they receive the
//! query text and nothing else. Scores, frecency and the merge are the
//! host-tested `voyeur` crate's; this file only gathers, sends and shows.
use crate::{launcher, COMMAND_LIST};
use sex_pdx::search as wire;
use voyeur::{abacus, rank, tables, Action, Kind};

/// Results the launcher keeps for one query.
const HITS: usize = 12;
/// Programs that may register as providers at once.
const PROGRAMS: usize = 4;
const TEXT: usize = 48;

/// Built-in providers in tie order, then programs.
const NAMES: [&str; 6] = ["abacus", "apps", "windows", "settings", "clipboard", "emoji"];
const ABACUS: u8 = 0;
const APPS: u8 = 1;
const WINDOWS: u8 = 2;
const SETTINGS: u8 = 3;
const CLIPS: u8 = 4;
const EMOJI: u8 = 5;
const FIRST_PROGRAM: u8 = 6;
/// Percent each provider's matches count with.
const WEIGHTS: [u8; 6] = [100, 100, 60, 90, 60, 50];
const PROGRAM_WEIGHT: u8 = 80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Payload { None, Command(u8), Window(u64), Clip(u8), Emoji(u8), Setting(u8), Answer, Program { slot: u8, index: u8, key: u64 } }

#[derive(Clone, Copy)]
struct Hit { title: [u8; TEXT], tlen: u8, provider: u8, kind: u8, action: u8, score: u32, payload: Payload }
impl Hit {
    const EMPTY: Self = Self { title: [0; TEXT], tlen: 0, provider: 0, kind: 0, action: 0, score: 0, payload: Payload::None };
    fn title(&self) -> &str { core::str::from_utf8(&self.title[..self.tlen as usize]).unwrap_or("") }
}

struct Program { participant: sex_pdx::clipboard::Participant, registration: wire::Registration, answer: wire::Collector }

struct State {
    query: [u8; wire::QUERY_MAX],
    qlen: u8,
    ticket: u32,
    hits: [Hit; HITS],
    count: u8,
    selected: u8,
    frecency: rank::Frecency,
    programs: [Option<Program>; PROGRAMS],
}

static mut STATE: State = State {
    query: [0; wire::QUERY_MAX], qlen: 0, ticket: 0, hits: [Hit::EMPTY; HITS], count: 0, selected: 0,
    frecency: rank::Frecency::new(), programs: [const { None }; PROGRAMS],
};

fn state() -> &'static mut State { unsafe { &mut *core::ptr::addr_of_mut!(STATE) } }
fn now() -> u64 { sex_pdx::get_ticks() / 1000 }
fn provider_name(p: u8) -> &'static str {
    if (p as usize) < NAMES.len() { return NAMES[p as usize]; }
    match state().programs.get((p - FIRST_PROGRAM) as usize) {
        // Registration names are fixed ASCII, but only names in this table
        // are 'static; programs show as "program".
        Some(Some(_)) => "program",
        _ => "?",
    }
}
fn kind_name(k: u8) -> &'static str { Kind::decode(k).map_or("?", Kind::name) }

/// Copies the start of `text` (its first line, cut on a character boundary).
fn clip_title(text: &[u8], out: &mut [u8; TEXT]) -> usize {
    let line = text.iter().position(|&b| b == b'\n' || b == b'\r').map_or(text, |e| &text[..e]);
    let mut cut = line.len().min(TEXT);
    while cut > 0 && cut < line.len() && line[cut] & 0xC0 == 0x80 { cut -= 1; }
    let s = core::str::from_utf8(&line[..cut]).map(|s| s.len()).unwrap_or(0);
    out[..s].copy_from_slice(&line[..s]);
    s
}

fn window_name(sid: u64) -> &'static str {
    match sid {
        crate::SURFACE_ID_QUIL => "Quil",
        crate::SURFACE_ID_LINEN => "Linen",
        crate::SURFACE_ID_MESH => "Mesh",
        crate::SURFACE_ID_COLLAR => "Collar",
        crate::SURFACE_ID_BELL_PLACEHOLDER => "Bell",
        crate::SURFACE_ID_BROWSER => "Browser",
        crate::SURFACE_ID_SPINDLE => "Spindle",
        _ => "Program window",
    }
}

/// Settings pages that an existing Shell action already opens.
fn setting_command(name: &str) -> Option<u8> {
    match name { "Privacy" => Some(5), "Notifications" => Some(4), "Workspaces" => Some(3), "Storage" => Some(2), _ => None }
}

/// The query changed: ask every provider again and merge what is known now.
/// Programs' answers merge in as they arrive.
pub unsafe fn refresh(query: &str) {
    let st = state();
    let b = query.as_bytes();
    let n = b.len().min(wire::QUERY_MAX);
    st.query[..n].copy_from_slice(&b[..n]);
    st.qlen = n as u8;
    st.ticket = st.ticket.wrapping_add(1).max(1);
    st.selected = 0;
    retire_dead_programs();
    let ticket = st.ticket;
    for (slot, p) in st.programs.iter_mut().enumerate() {
        let Some(p) = p else { continue };
        let old = p.answer.ticket();
        if old != 0 { let c = wire::cancel(old); sex_pdx::pdx_call(p.participant.route, c.opcode, c.args[0], c.args[1], c.args[2]); }
        p.answer.begin(ticket);
        let mut parts = [wire::Packet { opcode: 0, args: [0; 3] }; 3];
        let count = wire::query_packets(ticket, query, &mut parts);
        let mut status = 0;
        for w in &parts[..count] { status |= sex_pdx::pdx_call(p.participant.route, w.opcode, w.args[0], w.args[1], w.args[2]).0; }
        sex_pdx::serial_record!("[voyeur.ask] provider={} slot={} pd={} ticket={} parts={} status={:#x}",
            p.registration.name(), slot, p.participant.subject.pd, ticket, count, status);
    }
    merge();
}

/// Builds candidates from every provider, ranks them and keeps the best.
unsafe fn merge() {
    let st = state();
    let query = core::str::from_utf8(&st.query[..st.qlen as usize]).unwrap_or("");
    let mut cands = [rank::Candidate { provider: 0, weight: 0, kind: 0, title: "", keywords: "", hint: 0, hint_cap: 0 }; rank::MERGE_MAX];
    let mut payloads = [Payload::None; rank::MERGE_MAX];
    let mut n = 0usize;
    macro_rules! push { ($c:expr, $p:expr) => { if n < rank::MERGE_MAX { cands[n] = $c; payloads[n] = $p; n += 1; } } }
    let cand = |provider: u8, kind: Kind, title, keywords| rank::Candidate {
        provider, weight: WEIGHTS[provider as usize], kind: kind as u8, title, keywords, hint: 0, hint_cap: rank::HINT_CAP_BUILTIN,
    };
    // Abacus.
    let mut answer = [0u8; TEXT];
    let alen = abacus::evaluate(query).map_or(0, |v| v.format(&mut answer));
    let answer_str = core::str::from_utf8(&answer[..alen]).unwrap_or("");
    if alen != 0 { push!(rank::Candidate { hint: rank::HINT_CAP_BUILTIN, ..cand(ABACUS, Kind::Calculator, answer_str, "") }, Payload::Answer); }
    // Apps and actions: the stable command names, so frecency survives a
    // row's live title changing (Open/Focus/Restore).
    for (i, item) in COMMAND_LIST.iter().enumerate() {
        push!(cand(APPS, Kind::App, item.name, crate::command_words(item.command)), Payload::Command(i as u8));
    }
    // Windows of this workspace, most recent first.
    let mut ordered = [0u64; sex_pdx::window_switcher::MAX_ENTRIES];
    let windows = crate::switcher_windows(&mut ordered);
    for &sid in &ordered[..windows] { push!(cand(WINDOWS, Kind::Window, window_name(sid), "window"), Payload::Window(sid)); }
    for (i, s) in tables::SETTINGS.iter().enumerate() { push!(cand(SETTINGS, Kind::Setting, s.name, s.keywords), Payload::Setting(i as u8)); }
    // Clipboard history: the current text, then earlier copies.
    let clipboard = &*core::ptr::addr_of!(crate::CLIPBOARD);
    let mut clips = [[0u8; TEXT]; crate::clipboard::HISTORY + 1];
    let mut clip_len = [0usize; crate::clipboard::HISTORY + 1];
    if let Some(t) = clipboard.current_text() { clip_len[0] = clip_title(t, &mut clips[0]); }
    for i in 0..clipboard.history_len().min(crate::clipboard::HISTORY) {
        if let Some(t) = clipboard.history_text(i) { clip_len[i + 1] = clip_title(t, &mut clips[i + 1]); }
    }
    for i in 0..clips.len() {
        if clip_len[i] == 0 { continue; }
        let title = core::str::from_utf8(&clips[i][..clip_len[i]]).unwrap_or("");
        push!(cand(CLIPS, Kind::Clipboard, title, "clipboard"), Payload::Clip(if i == 0 { u8::MAX } else { (i - 1) as u8 }));
    }
    for (i, e) in tables::EMOJI.iter().enumerate() { push!(cand(EMOJI, Kind::Emoji, e.name, e.keywords), Payload::Emoji(i as u8)); }
    // Programs' answers to this ticket; their hint is capped.
    for (slot, p) in st.programs.iter().enumerate() {
        let Some(p) = p else { continue };
        if p.answer.ticket() != st.ticket { continue; }
        for (index, r) in p.answer.records().iter().enumerate() {
            push!(rank::Candidate { provider: FIRST_PROGRAM + slot as u8, weight: PROGRAM_WEIGHT, kind: r.kind, title: r.title(), keywords: r.subtitle(),
                hint: r.hint, hint_cap: rank::HINT_CAP_EXTERNAL }, Payload::Program { slot: slot as u8, index: index as u8, key: r.key });
        }
    }
    let mut ranked = [rank::Ranked::default(); 32];
    let total = rank::merge(query, now(), &st.frecency, &cands[..n], &mut ranked);
    let mut answered = 0u64;
    for r in &ranked[..total] { answered |= 1 << cands[r.index].provider.min(63); }
    st.count = total.min(HITS) as u8;
    for (hit, r) in st.hits.iter_mut().zip(&ranked[..st.count as usize]) {
        let c = &cands[r.index];
        let t = c.title.as_bytes();
        let mut cut = t.len().min(TEXT);
        while cut > 0 && cut < t.len() && t[cut] & 0xC0 == 0x80 { cut -= 1; }
        *hit = Hit { title: [0; TEXT], tlen: cut as u8, provider: c.provider, kind: c.kind, score: r.score, payload: payloads[r.index],
            action: match payloads[r.index] {
                Payload::Window(_) => Action::Focus as u8,
                Payload::Answer | Payload::Emoji(_) | Payload::Clip(_) => Action::Copy as u8,
                Payload::Command(_) => Action::Run as u8,
                Payload::Program { slot, index, .. } => st.programs[slot as usize].as_ref()
                    .and_then(|p| p.answer.records().get(index as usize)).map_or(Action::Open as u8, |r| r.action),
                _ => Action::Open as u8,
            } };
        hit.title[..cut].copy_from_slice(&t[..cut]);
    }
    let first = st.hits[0];
    let mut names = [0u8; 64];
    let mut len = 0;
    for p in 0..64u8 {
        if answered & 1 << p == 0 { continue; }
        let name = provider_name(p).as_bytes();
        if len != 0 && len < names.len() { names[len] = b'+'; len += 1; }
        let take = name.len().min(names.len() - len);
        names[len..len + take].copy_from_slice(&name[..take]); len += take;
    }
    sex_pdx::serial_record!("[voyeur.query] ticket={} query={} providers={} answered={} results={} first={} first_kind={} first_provider={}",
        st.ticket, query, answered.count_ones(), core::str::from_utf8(&names[..len]).unwrap_or("?"), total,
        if st.count == 0 { "-" } else { first.title() }, if st.count == 0 { "-" } else { kind_name(first.kind) },
        if st.count == 0 { "-" } else { provider_name(first.provider) });
    for (i, h) in st.hits[..(st.count as usize).min(5)].iter().enumerate() {
        sex_pdx::serial_record!("[voyeur.result] rank={} provider={} kind={} action={} score={} title={}",
            i, provider_name(h.provider), kind_name(h.kind), Action::decode(h.action).map_or("?", Action::name), h.score, h.title());
    }
}

/// Whether the shown results belong to the launcher's current query.
unsafe fn current() -> bool {
    let st = state();
    let q = (*core::ptr::addr_of!(crate::PALETTE_QUERY)).as_str().as_bytes();
    st.count != 0 && !q.is_empty() && q == &st.query[..st.qlen as usize]
}

/// The launcher rows for the current query: an app row keeps the live
/// title and status the launcher computed (`apps`, by command index).
/// None when Voyeur has nothing for this query (the launcher's own rows show).
pub unsafe fn rows<'a, const N: usize>(apps: &[launcher::Row<'a>], out: &mut [launcher::Row<'a>; N]) -> Option<(usize, usize)> {
    if !current() { return None; }
    let st = state();
    let hits = &*core::ptr::addr_of!(STATE.hits);
    let count = (st.count as usize).min(N);
    for (row, hit) in out.iter_mut().zip(&hits[..count]) {
        *row = match hit.payload {
            Payload::Command(i) if (i as usize) < apps.len() => apps[i as usize],
            p => launcher::Row { name: hit.title(), status: match p {
                Payload::Answer => "ABACUS / ENTER SHOWS",
                Payload::Window(_) => "WINDOW / ENTER FOCUSES",
                Payload::Setting(_) => "SETTINGS PAGE",
                Payload::Clip(_) => "CLIPBOARD / ENTER RESTORES",
                Payload::Emoji(_) => "EMOJI",
                Payload::Program { .. } => "FROM A PROGRAM",
                _ => "",
            } },
        };
    }
    Some((count, (st.selected as usize).min(count.saturating_sub(1))))
}

/// Arrow keys over Voyeur's rows; false when they are not showing.
pub unsafe fn step(dir: i32) -> bool {
    if !current() { return false; }
    let st = state();
    let n = st.count as i32;
    st.selected = ((st.selected as i32 + dir).rem_euclid(n)) as u8;
    sex_pdx::serial_record!("[voyeur.select] index={} title={}", st.selected, st.hits[st.selected as usize].title());
    true
}

pub enum Enter { NotShowing, Command(u8), Done }

/// Enter on the selected row. A command row goes back to the launcher's own
/// checked activation path; the rest are carried out here.
pub unsafe fn enter() -> Enter {
    if !current() { return Enter::NotShowing; }
    let st = state();
    let hit = st.hits[st.selected as usize];
    let title = hit.title();
    if !matches!(hit.payload, Payload::Command(_)) { st.frecency.record(rank::key(hit.kind, title), now()); }
    let mut text = [0u8; 120];
    let mut n = 0;
    let mut put = |part: &[u8]| { let take = part.len().min(text.len() - n); text[n..n + take].copy_from_slice(&part[..take]); n += take; };
    let (done, detail) = match hit.payload {
        Payload::Command(i) => return Enter::Command(i),
        Payload::Setting(i) => match setting_command(title) {
            Some(cmd) => return Enter::Command(cmd),
            None => { put(b"No settings program yet for "); put(tables::SETTINGS[i as usize].name.as_bytes()); (true, "no_settings_program") }
        },
        Payload::Window(sid) => (crate::try_set_focus(sid), "focus"),
        Payload::Clip(u8::MAX) => { put(b"Already on the clipboard"); (true, "current") }
        Payload::Clip(i) => {
            let ok = (*core::ptr::addr_of_mut!(crate::CLIPBOARD)).restore(i as usize).is_some();
            put(if ok { b"Clipboard restored: ".as_slice() } else { b"Clipboard busy".as_slice() }); put(title.as_bytes());
            (ok, "restore")
        }
        // Shell has no clipboard publication of its own yet: show the value.
        Payload::Answer => { put(st.query[..st.qlen as usize].as_ref()); put(b" = "); put(title.as_bytes()); (true, "shown_copy_pending") }
        Payload::Emoji(i) => { let e = &tables::EMOJI[i as usize]; put(e.text.as_bytes()); put(b" "); put(e.name.as_bytes()); (true, "shown_copy_pending") }
        Payload::Program { slot, index, key } => match &st.programs[slot as usize] {
            Some(p) => {
                let w = wire::activate(p.answer.ticket(), index, hit.action, key);
                (sex_pdx::pdx_call(p.participant.route, w.opcode, w.args[0], w.args[1], w.args[2]).0 == 0, "program")
            }
            None => (false, "program_gone"),
        },
        Payload::None => (false, "none"),
    };
    if n != 0 && crate::notify_panel(&text[..n]) { crate::NOTIFY_NOTICE_UNTIL = sex_pdx::get_ticks() + crate::NOTIFY_SHOW_TICKS; }
    sex_pdx::serial_record!("[voyeur.activate] provider={} kind={} action={} title={} ok={} detail={}",
        provider_name(hit.provider), kind_name(hit.kind), Action::decode(hit.action).map_or("?", Action::name), title, done as u8, detail);
    Enter::Done
}

/// The launcher ran command `index` (from its rows or from Voyeur's).
pub unsafe fn launched(index: usize) {
    if let Some(item) = COMMAND_LIST.get(index) { state().frecency.record(rank::key(Kind::App as u8, item.name), now()); }
}

unsafe fn retire_dead_programs() {
    for p in state().programs.iter_mut() {
        if p.as_ref().is_some_and(|p| crate::program_participant(p.participant.surface) != Some(p.participant)) {
            sex_pdx::serial_record!("[voyeur.provider.retired] pd={}", p.as_ref().map_or(0, |p| p.participant.subject.pd));
            *p = None;
        }
    }
}

/// Provider words from programs. Only a live catalogued program, on its own
/// route, may register; answers count only for the current ticket.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if !wire::provider_opcode(msg.type_id) { return false; }
    let packet = wire::Packet { opcode: msg.type_id, args: [msg.arg0, msg.arg1, msg.arg2] };
    let participant = (*core::ptr::addr_of!(crate::APP_CUSTODY)).iter().flatten()
        .find(|e| e.target == msg.caller_pd && e.task == msg.caller_task && !e.dead && !e.retired && !e.close_pending)
        .and_then(|e| crate::program_participant(e.surface));
    let Some(participant) = participant else {
        sex_pdx::serial_record!("[voyeur.provider.denied] pd={} task={} opcode={:#x}", msg.caller_pd, msg.caller_task, msg.type_id);
        return true;
    };
    let st = state();
    let slot = st.programs.iter().position(|p| p.as_ref().is_some_and(|p| p.participant == participant));
    if msg.type_id == wire::REGISTER {
        let reg = wire::Registration::decode(packet);
        let free = slot.or_else(|| st.programs.iter().position(Option::is_none));
        let status = match (reg, free) {
            (None, _) => wire::Status::Invalid,
            (_, None) => wire::Status::Full,
            (Some(registration), Some(i)) => {
                st.programs[i] = Some(Program { participant, registration, answer: wire::Collector::new() });
                wire::Status::Ok
            }
        };
        let id = free.map_or(0, |i| FIRST_PROGRAM + i as u8);
        let r = wire::registered(id, status);
        let sent = sex_pdx::pdx_call(participant.route, r.opcode, r.args[0], r.args[1], r.args[2]).0;
        sex_pdx::serial_record!("[voyeur.provider.register] pd={} name={} status={:?} id={} sent={:#x}",
            participant.subject.pd, reg.as_ref().map_or("-", |r| r.name()), status, id, sent);
        return true;
    }
    let Some(i) = slot else { return true };
    let Some(p) = st.programs[i].as_mut() else { return true };
    match p.answer.accept(packet) {
        Ok(wire::Collect::Done(count)) => {
            sex_pdx::serial_record!("[voyeur.provider.answer] name={} ticket={} results={}", p.registration.name(), p.answer.ticket(), count);
            merge();
            if crate::COMMAND_PALETTE_OPEN { crate::palette_render_list(); }
        }
        Ok(wire::Collect::Pending) | Err(wire::CollectError::StaleTicket) => {}
        Err(e) => sex_pdx::serial_record!("[voyeur.provider.dropped] name={} reason={:?}", p.registration.name(), e),
    }
    true
}
