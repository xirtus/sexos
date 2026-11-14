//! Host tests for the Voyeur provider wire protocol: the exact
//! `crates/sex-pdx/src/search.rs` the Shell and providers compile, and an
//! external provider's answer merged with a Shell built-in one through the
//! exact `crates/voyeur` ranking.
#[path = "../crates/sex-pdx/src/search.rs"]
mod search;
#[path = "../crates/voyeur/src/rank.rs"]
mod rank;
use search::*;

const APP: u8 = 1;
const FILE: u8 = 2;
const OPEN: u8 = 1;
const REVEAL: u8 = 3;

fn answer(ticket: u32, records: &[Record]) -> Vec<Packet> {
    let mut out = [Packet { opcode: 0, args: [0; 3] }; ANSWER_PACKETS];
    let n = answer_packets(ticket, records, &mut out);
    out[..n].to_vec()
}
fn collect(ticket: u32, packets: &[Packet]) -> Result<Collector, CollectError> {
    let mut c = Collector::new();
    c.begin(ticket);
    for p in packets { c.accept(*p)?; }
    Ok(c)
}

#[test]
fn opcodes_are_distinct_and_in_their_block() {
    let all = [REGISTER, REGISTERED, QUERY, RESULT, TEXT, DONE, ACTIVATE, CANCEL];
    for (i, a) in all.iter().enumerate() {
        assert!((0x3C0..=0x3CF).contains(a));
        assert!(all[i + 1..].iter().all(|b| b != a));
        assert!(provider_opcode(*a) != shell_opcode(*a));
    }
}

#[test]
fn registration_round_trips_and_refuses_bad_names() {
    let r = Registration::new("linen files", 1 << FILE | 1 << 3).unwrap();
    assert_eq!(Registration::decode(r.wire()), Some(r));
    assert_eq!(r.name(), "linen files");
    assert!(Registration::new("", 1).is_none());
    assert!(Registration::new("seventeen chars!!", 1).is_none());
    assert!(Registration::new("bad\nname", 1).is_none());
    assert!(Registration::new("ok", 0).is_none());
    let mut w = r.wire();
    w.args[2] |= 1 << 30;
    assert!(Registration::decode(w).is_none(), "reserved bits");
    let mut w = r.wire();
    w.args[2] = (w.args[2] & 0xFFFF) | 2 << 56;
    assert!(Registration::decode(w).is_none(), "version");
    assert_eq!(decode_registered(registered(4, Status::Ok)), Some((4, Status::Ok)));
}

#[test]
fn query_round_trips_in_parts() {
    for q in ["2+2", "exactly sixteen!", "a query that needs all three of its parts!!"] {
        let mut out = [Packet { opcode: 0, args: [0; 3] }; 3];
        let n = query_packets(9, q, &mut out);
        assert_eq!(n, q.len().div_ceil(16));
        let mut r = QueryReader::new();
        let done: Vec<_> = out[..n].iter().map(|p| r.accept(*p)).collect();
        assert_eq!(done.last().copied().flatten(), Some(9));
        assert_eq!(r.query(), q);
    }
}

#[test]
fn query_refuses_too_long_empty_and_out_of_order() {
    let mut out = [Packet { opcode: 0, args: [0; 3] }; 3];
    assert_eq!(query_packets(1, &"x".repeat(QUERY_MAX + 1), &mut out), 0);
    assert_eq!(query_packets(1, "", &mut out), 0);
    assert_eq!(query_packets(0, "a", &mut out), 0);
    let n = query_packets(3, "twenty bytes of text", &mut out);
    assert_eq!(n, 2);
    let mut r = QueryReader::new();
    assert_eq!(r.accept(out[1]), None, "second part first");
    assert_eq!(r.query(), "");
}

#[test]
fn query_carries_nothing_but_the_query() {
    // Privacy: the words a provider gets hold the ticket, the length, the
    // part numbers and the query bytes, and nothing else.
    let mut out = [Packet { opcode: 0, args: [0; 3] }; 3];
    let n = query_packets(0x1234, "2+2", &mut out);
    assert_eq!(n, 1);
    assert_eq!(out[0], Packet { opcode: QUERY, args: [0x1234 | 3 << 32 | 1 << 48, u64::from_le_bytes(*b"2+2\0\0\0\0\0"), 0] });
}

#[test]
fn answer_round_trips_through_the_collector() {
    let recs = [
        Record::new(FILE, OPEN, 300, 77, "budget-2026.ods", "Documents / spreadsheets").unwrap(),
        Record::new(FILE, REVEAL, 0, 78, "a title that is longer than thirty-two bytes!!", "").unwrap(),
    ];
    let c = collect(5, &answer(5, &recs)).unwrap();
    assert!(c.done());
    assert_eq!(c.records(), &recs);
    assert_eq!(c.records()[1].title(), "a title that is longer than thirty-two bytes!!");
}

#[test]
fn collector_ignores_stale_tickets_and_drops_broken_answers() {
    let recs = [Record::new(FILE, OPEN, 0, 1, "notes.txt", "Home").unwrap()];
    let words = answer(5, &recs);
    let mut c = Collector::new();
    c.begin(6);
    assert_eq!(c.accept(words[0]), Err(CollectError::StaleTicket));
    // Missing text: DONE is refused and nothing is kept.
    let mut short = words.clone();
    short.remove(2);
    assert_eq!(collect(5, &short).err(), Some(CollectError::Incomplete));
    // Subtitle before the title is whole.
    let mut early = words.clone();
    early.remove(1);
    assert_eq!(collect(5, &early).err(), Some(CollectError::OutOfOrder));
    // Text before its header.
    let mut swapped = words.clone();
    swapped.swap(0, 1);
    assert_eq!(collect(5, &swapped).err(), Some(CollectError::Malformed));
    // A DONE that claims more results than came.
    let mut lying = words.clone();
    let last = lying.len() - 1;
    lying[last].args[0] += 1 << 32;
    assert_eq!(collect(5, &lying).err(), Some(CollectError::Incomplete));
    // Words after DONE.
    let mut extra = words.clone();
    extra.push(words[0]);
    assert_eq!(collect(5, &extra).err(), Some(CollectError::OutOfOrder));
    // Bytes past a chunk's count.
    let mut padded = words.clone();
    padded[1].args[2] = 1;
    assert_eq!(collect(5, &padded).err(), Some(CollectError::Malformed));
}

#[test]
fn records_refuse_control_text_and_overlong_titles() {
    assert!(Record::new(FILE, OPEN, 0, 0, "", "x").is_none());
    assert!(Record::new(FILE, OPEN, 0, 0, "a\u{1b}[2J", "").is_none());
    assert!(Record::new(FILE, OPEN, 0, 0, &"t".repeat(TITLE_MAX + 1), "").is_none());
    assert!(Record::new(0, OPEN, 0, 0, "t", "").is_none());
    assert!(Record::new(FILE, OPEN, 0, 0, "caf\u{e9}", "").is_some());
}

#[test]
fn at_most_eight_results_per_answer() {
    let r = Record::new(FILE, OPEN, 0, 0, "f", "").unwrap();
    let words = answer(2, &[r; 12]);
    let c = collect(2, &words).unwrap();
    assert_eq!(c.records().len(), RESULTS_MAX);
}

#[test]
fn activate_and_cancel_round_trip() {
    assert_eq!(decode_activate(activate(7, 2, OPEN, 99)), Some((7, 2, OPEN, 99)));
    assert_eq!(decode_cancel(cancel(7)), Some(7));
    assert_eq!(decode_cancel(cancel(0)), None);
}

/// The R9 gate on the host: two providers answer one query, one a Shell
/// built-in and one an external program over the wire, and the Shell's
/// merge orders them by its own scores, capping the program's hint.
#[test]
fn two_providers_answer_one_query() {
    let query = "budget";
    // External provider (a file index) answers over the wire, with a hint
    // far above what a program may claim.
    let wire = answer(11, &[
        Record::new(FILE, OPEN, u16::MAX, 1, "budget-2026.ods", "Documents").unwrap(),
        Record::new(FILE, OPEN, u16::MAX, 2, "holiday photos", "Pictures").unwrap(),
    ]);
    let c = collect(11, &wire).unwrap();
    let mut cands = vec![rank::Candidate { provider: 0, weight: 100, kind: APP, title: "Budget Planner", keywords: "money finance", hint: 0, hint_cap: rank::HINT_CAP_BUILTIN }];
    for r in c.records() {
        cands.push(rank::Candidate { provider: 1, weight: 100, kind: r.kind, title: r.title(), keywords: r.subtitle(), hint: r.hint, hint_cap: rank::HINT_CAP_EXTERNAL });
    }
    let mut out = [rank::Ranked::default(); 8];
    let n = rank::merge(query, 0, &rank::Frecency::new(), &cands, &mut out);
    let titles: Vec<_> = out[..n].iter().map(|r| (cands[r.index].provider, cands[r.index].title)).collect();
    assert_eq!(titles, vec![(0, "Budget Planner"), (1, "budget-2026.ods"), (1, "holiday photos")]);
    // The unrelated file counts only with the capped hint.
    assert_eq!(out[2].score, rank::HINT_CAP_EXTERNAL as u32);
}
