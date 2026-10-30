// SPDX-License-Identifier: GPL-3.0-only
//! Real bounded path codec and pure navigation controller; no authority mock.
extern crate self as sex_pdx;
pub use sex_net_config as network_config;
#[path = "../apps/kaleidoscope/src/http_wire.rs"] mod http_wire;
#[path = "../apps/kaleidoscope/src/http_navigation.rs"] mod http_navigation;

use http_navigation::Navigation;
use http_wire::Path;

fn path(value: &[u8]) -> Path { Path::from_location(value).unwrap() }

#[test]
fn initial_state_and_inactive_follow_cannot_start_navigation() {
    const EMPTY: Navigation = Navigation::new();
    let mut navigation = EMPTY;
    assert!(!navigation.active());
    assert_eq!(navigation.hops(), 0);
    assert_eq!(navigation.current(), Path::initial());
    assert!(!navigation.follow(path(b"/guide")));
    assert!(!navigation.active());
    assert_eq!(navigation.current(), Path::initial());
    assert_eq!(navigation.hops(), 0);
}

#[test]
fn exactly_two_follows_are_recorded_and_third_is_rejected_without_path_change() {
    let mut navigation = Navigation::new();
    navigation.begin();
    assert!(navigation.active());
    assert_eq!(navigation.hops(), 0);
    for (index, target) in [path(b"/guide"), path(b"/guide/text")].into_iter().enumerate() {
        assert!(navigation.follow(target));
        assert!(navigation.active());
        assert_eq!(navigation.current(), target);
        assert_eq!(navigation.hops(), index + 1);
    }
    assert!(!navigation.follow(path(b"/third")));
    assert!(!navigation.active());
    assert_eq!(navigation.hops(), 2);
    assert_eq!(navigation.current(), path(b"/guide/text"));
    assert!(!navigation.follow(path(b"/fourth")));
    assert_eq!(navigation.current(), path(b"/guide/text"));
}

#[test]
fn self_redirect_ends_initial_or_followed_intent_without_accepting_new_path() {
    for first in [None, Some(path(b"/guide"))] {
        let mut navigation = Navigation::new();
        navigation.begin();
        if let Some(target) = first { assert!(navigation.follow(target)); }
        let current = navigation.current();
        let hops = navigation.hops();
        assert!(!navigation.follow(current));
        assert!(!navigation.active());
        assert_eq!(navigation.current(), current);
        assert_eq!(navigation.hops(), hops);
    }
}

#[test]
fn two_node_loop_back_to_initial_is_rejected_before_second_hop() {
    let mut navigation = Navigation::new();
    navigation.begin();
    assert!(navigation.follow(path(b"/guide")));
    assert!(!navigation.follow(Path::initial()));
    assert!(!navigation.active());
    assert_eq!(navigation.hops(), 1);
    assert_eq!(navigation.current(), path(b"/guide"));
}

#[test]
fn completion_preserves_final_path_but_prevents_later_follow() {
    let mut navigation = Navigation::new();
    navigation.begin();
    assert!(navigation.follow(path(b"/guide")));
    navigation.complete();
    assert!(!navigation.active());
    assert_eq!(navigation.current(), path(b"/guide"));
    assert_eq!(navigation.hops(), 1);
    assert!(!navigation.follow(path(b"/next")));
    assert_eq!(navigation.current(), path(b"/guide"));
}

#[test]
fn cancellation_and_new_begin_remove_old_history_and_hop_budget() {
    let mut navigation = Navigation::new();
    navigation.begin();
    assert!(navigation.follow(path(b"/guide")));
    assert!(navigation.follow(path(b"/guide/text")));
    navigation.cancel();
    assert!(!navigation.active());
    assert_eq!(navigation.hops(), 0);
    assert_eq!(navigation.current(), Path::initial());
    assert!(!navigation.follow(path(b"/guide/text")));
    navigation.begin();
    assert!(navigation.follow(path(b"/guide/text")));
    navigation.begin();
    assert_eq!(navigation.current(), Path::initial());
    assert_eq!(navigation.hops(), 0);
    assert!(navigation.follow(path(b"/guide/text")));
}

#[test]
fn distinct_canonical_paths_are_data_and_invalid_locations_cannot_enter_controller() {
    let mut navigation = Navigation::new();
    navigation.begin();
    for invalid in [b"https://other.test/guide".as_slice(), b"//other/guide", b"/../guide", b"/a%2fb", b"/a?b", b"/a#b", b"/a\\b", b"/a\r\nHost: other"] {
        assert!(Path::from_location(invalid).is_err());
        assert!(navigation.active());
        assert_eq!(navigation.hops(), 0);
        assert_eq!(navigation.current(), Path::initial());
    }
    assert!(navigation.follow(path(b"/Guide")));
    assert!(navigation.follow(path(b"/guide")));
    assert_eq!(navigation.hops(), 2);
}
