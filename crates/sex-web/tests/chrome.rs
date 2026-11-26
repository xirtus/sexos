// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests for Kaleidoscope's Helium-style single-row compact Candy Lavender chrome:
//! omnibox URL vs search resolution, inline tab open/switch/close & hit-testing,
//! bookmark add/remove/toggle & toggleable bookmarks bar hit-testing, single-row
//! toolbar hit-testing, context menu hit-testing, and full-chrome rendering bounds safety.

use sex_web::{
    bookmark_hit_at, bookmark_pills, build_context_menu, chrome_height, context_menu_contains,
    context_menu_hit_at, context_menu_rect, default_bookmarks, frame_with, merge_bookmarks,
    resolve_omnibox_input, shields_pill_width, strip_slots, tab_close_rect, tab_hit_at,
    tab_strip_layout, toolbar_hit_at, toolbar_layout, BookmarkStore, Chrome, ContextAction, Fonts,
    Slot, TabGroup, TabHit, TabStripState, ToolbarHit, BAR, BOOKMARKS_H, GROUP_COLOURS, TOP_BAR_H,
};

#[test]
fn omnibox_resolves_urls_and_search_queries() {
    assert_eq!(
        resolve_omnibox_input("https://example.com/path?a=1"),
        "https://example.com/path?a=1"
    );
    assert_eq!(
        resolve_omnibox_input("http://localhost:8080/test"),
        "http://localhost:8080/test"
    );
    assert_eq!(resolve_omnibox_input("about:welcome"), "about:welcome");
    assert_eq!(resolve_omnibox_input("about:shields"), "about:shields");

    // Bare domains get https://.
    assert_eq!(resolve_omnibox_input("example.com"), "https://example.com");
    assert_eq!(
        resolve_omnibox_input("en.wikipedia.org/wiki/Rust"),
        "https://en.wikipedia.org/wiki/Rust"
    );

    // Localhost and IPv4 get http://.
    assert_eq!(resolve_omnibox_input("localhost"), "http://localhost");
    assert_eq!(
        resolve_omnibox_input("localhost:3000/app"),
        "http://localhost:3000/app"
    );
    assert_eq!(
        resolve_omnibox_input("127.0.0.1:8000"),
        "http://127.0.0.1:8000"
    );

    // Single words without a dot become DuckDuckGo HTML search queries.
    assert_eq!(
        resolve_omnibox_input("kaleidoscope"),
        "https://html.duckduckgo.com/html/?q=kaleidoscope"
    );

    // Multi-word queries with spaces and special characters are query-encoded.
    assert_eq!(
        resolve_omnibox_input("  rust microkernel & pku  "),
        "https://html.duckduckgo.com/html/?q=rust+microkernel+%26+pku"
    );

    // Empty / whitespace input resolves to empty string.
    assert_eq!(resolve_omnibox_input("   "), "");
}

#[test]
fn tab_strip_open_switch_close_state() {
    let mut strip = TabStripState::new("Welcome");
    assert_eq!(strip.tabs.len(), 1);
    assert_eq!(strip.active, 0);

    let t1 = strip.open_tab("Wikipedia");
    assert_eq!(t1, 1);
    assert_eq!(strip.active, 1);

    let t2 = strip.open_tab("Hacker News");
    assert_eq!(t2, 2);
    assert_eq!(strip.tabs, vec!["Welcome", "Wikipedia", "Hacker News"]);

    assert!(strip.switch_to(0));
    assert_eq!(strip.active, 0);
    assert!(!strip.switch_to(99));

    // Close background tab 1; active stays 0.
    assert!(strip.close_tab(1));
    assert_eq!(strip.tabs, vec!["Welcome", "Hacker News"]);
    assert_eq!(strip.active, 0);

    // Switch to tab 1, then close background tab 0; active shifts down to 0.
    assert!(strip.switch_to(1));
    assert!(strip.close_tab(0));
    assert_eq!(strip.tabs, vec!["Hacker News"]);
    assert_eq!(strip.active, 0);

    // Closing the last remaining tab signals window close (false).
    assert!(!strip.close_tab(0));
}

#[test]
fn helium_inline_tabs_hit_testing_tabs_close_group_and_new_tab() {
    let groups = vec![TabGroup {
        colour: GROUP_COLOURS[0],
        name: String::from("Docs"),
        folded: false,
    }];
    let group_of = vec![Some(0), None];
    let slots = strip_slots(&group_of, &groups);
    assert_eq!(slots, vec![Slot::Group(0), Slot::Tab(0), Slot::Tab(1)]);

    let width = 1100;
    let sw = shields_pill_width(Some((true, 0)));
    let (chips, (plus_x, plus_w)) = tab_strip_layout(width, sw, &slots, &groups);
    assert_eq!(chips.len(), 3);

    // Inline tabs sit on the same single row as the omnibox (y inside 0..TOP_BAR_H).
    let tl = toolbar_layout(width, sw, false);
    assert!(chips[0].0 >= tl.omnibox[0] + tl.omnibox[2]);
    assert!(plus_x + plus_w <= tl.shields[0]);

    // Hit group label.
    assert_eq!(
        tab_hit_at(width, sw, &slots, &groups, chips[0].0 + 6, 18),
        Some(TabHit::Group(0))
    );
    // Hit tab 0 body.
    assert_eq!(
        tab_hit_at(width, sw, &slots, &groups, chips[1].0 + 16, 18),
        Some(TabHit::Tab(0))
    );
    // Hit tab 0 close button.
    let close = tab_close_rect(chips[1].0, chips[1].1).expect("tab chip wide enough for close");
    assert_eq!(
        tab_hit_at(width, sw, &slots, &groups, close[0] + 4, close[1] + 4),
        Some(TabHit::Close(0))
    );
    // Hit '+' New Tab button.
    assert_eq!(
        tab_hit_at(width, sw, &slots, &groups, plus_x + plus_w / 2, 18),
        Some(TabHit::NewTab)
    );
    // Below top bar returns None.
    assert_eq!(
        tab_hit_at(width, sw, &slots, &groups, chips[1].0 + 10, TOP_BAR_H + 2),
        None
    );
}

#[test]
fn bookmarks_add_remove_toggle_and_toggleable_bar_hit_testing() {
    assert_eq!(chrome_height(false), BAR);
    assert_eq!(chrome_height(true), TOP_BAR_H + BOOKMARKS_H);

    let mut store = BookmarkStore::default();
    assert!(!store.contains("https://rust-lang.org/"));

    assert!(store.add("https://rust-lang.org/", "Rust Lang"));
    assert!(store.contains("https://rust-lang.org/"));
    assert!(!store.add("https://rust-lang.org/", "Rust Lang"));

    assert!(!store.toggle("https://rust-lang.org/", "Rust Lang"));
    assert!(!store.contains("https://rust-lang.org/"));
    assert!(store.toggle("https://rust-lang.org/", "Rust Lang"));
    assert!(store.contains("https://rust-lang.org/"));

    let defaults = default_bookmarks();
    assert!(defaults.len() >= 5);
    let merged = merge_bookmarks(&defaults, &store.items);
    assert_eq!(merged.len(), defaults.len() + 1);
    assert_eq!(merged.last().unwrap().url, "https://rust-lang.org/");

    let width = 1000;
    let pills = bookmark_pills(width, &merged);
    assert_eq!(pills.len(), merged.len());
    let bar_mid_y = TOP_BAR_H + BOOKMARKS_H / 2;
    for (idx, &(px, pw)) in pills.iter().enumerate() {
        assert_eq!(
            bookmark_hit_at(width, &merged, px + pw / 2, bar_mid_y),
            Some(idx)
        );
    }
    // Outside bookmarks bar returns None.
    assert_eq!(bookmark_hit_at(width, &merged, 20, 12), None);
    assert_eq!(
        bookmark_hit_at(width, &merged, 20, TOP_BAR_H + BOOKMARKS_H + 5),
        None
    );

    assert!(store.remove("https://rust-lang.org/"));
    assert!(!store.remove("https://rust-lang.org/"));
}

#[test]
fn helium_single_row_toolbar_hit_testing_all_controls() {
    let width = 1100;
    let sw = shields_pill_width(Some((true, 12)));
    let l = toolbar_layout(width, sw, false);
    let mid = |r: [i32; 4]| (r[0] + r[2] / 2, r[1] + r[3] / 2);

    // Omnibox is compact (~260-340px) leaving room for inline tabs on the same row.
    assert!((260..=340).contains(&l.omnibox[2]), "omnibox width={}", l.omnibox[2]);
    assert!(l.tabs_zone[2] > 200, "tabs zone width={}", l.tabs_zone[2]);

    let (x, y) = mid(l.back);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Back));
    let (x, y) = mid(l.forward);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Forward));
    let (x, y) = mid(l.reload);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Reload));
    let (x, y) = (l.omnibox[0] + 24, l.omnibox[1] + l.omnibox[3] / 2);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Omnibox));
    let (x, y) = mid(l.star);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::BookmarkStar));
    let (x, y) = mid(l.shields);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Shields));
    let (x, y) = mid(l.bookmarks_toggle);
    assert_eq!(
        toolbar_hit_at(width, sw, false, x, y),
        Some(ToolbarHit::BookmarksToggle)
    );
    let (x, y) = mid(l.menu);
    assert_eq!(toolbar_hit_at(width, sw, false, x, y), Some(ToolbarHit::Menu));
}

#[test]
fn context_menu_hit_testing_and_window_clamping() {
    let menu = build_context_menu(990, 690, 1024, 720, false, true, false, false, true);
    let [mx, my, mw, mh] = context_menu_rect(&menu);
    assert!(mx >= 4 && mx + mw <= 1020);
    assert!(my >= 4 && my + mh <= 716);

    assert!(context_menu_contains(&menu, mx + 10, my + 10));
    assert!(!context_menu_contains(&menu, mx - 5, my + 10));

    // Row 0 is Back (disabled because can_back = false) -> hit returns None.
    assert_eq!(context_menu_hit_at(&menu, mx + 20, my + 10), None);
    // Row 1 is Forward (enabled).
    let fwd = context_menu_hit_at(&menu, mx + 20, my + 5 + sex_web::CONTEXT_MENU_ROW + 10);
    assert_eq!(fwd.map(|i| menu.items[i].action), Some(ContextAction::Forward));
    // Row 2 is Reload.
    let rel = context_menu_hit_at(&menu, mx + 20, my + 5 + sex_web::CONTEXT_MENU_ROW * 2 + 10);
    assert_eq!(rel.map(|i| menu.items[i].action), Some(ContextAction::Reload));
    // Row 3 is NewTab (after a separator).
    let nt = context_menu_hit_at(
        &menu,
        mx + 20,
        my + 5 + sex_web::CONTEXT_MENU_ROW * 3 + sex_web::CONTEXT_MENU_SEP + 10,
    );
    assert_eq!(nt.map(|i| menu.items[i].action), Some(ContextAction::NewTab));
    // Row 5 is ToggleBookmarksBar.
    let bm = context_menu_hit_at(
        &menu,
        mx + 20,
        my + 5 + sex_web::CONTEXT_MENU_ROW * 5 + sex_web::CONTEXT_MENU_SEP + 10,
    );
    assert_eq!(
        bm.map(|i| menu.items[i].action),
        Some(ContextAction::ToggleBookmarksBar)
    );
    assert_eq!(context_menu_hit_at(&menu, 0, 0), None);
}

#[test]
fn helium_chrome_renders_cleanly_single_row_and_with_bookmarks_bar() {
    let mut fonts = Fonts::new().unwrap();
    let (w, h) = (1024, 600);
    let tabs = vec![String::from("Welcome to Kaleidoscope"), String::from("Rust")];
    let bookmarks = default_bookmarks();
    let suggestions = vec![String::from("https://en.wikipedia.org/")];

    for show_bm in [false, true] {
        let bh = chrome_height(show_bm);
        let page = sex_web::sex_html::render("<h1>Kaleidoscope</h1><p>Hello</p>", w, h - bh, &fonts);
        let mut menu = build_context_menu(240, 160, w, h, true, false, true, show_bm, true);
        menu.hovered = Some(2);

        let chrome = Chrome {
            text: "https://example.com",
            editing: true,
            tabs: &tabs,
            active: 0,
            suggestions: &suggestions,
            chosen: Some(0),
            zoom: 110,
            shields: Some((true, 7)),
            can_back: true,
            can_forward: false,
            loading: false,
            bookmarked: true,
            show_bookmarks_bar: show_bm,
            bookmarks: &bookmarks,
            context_menu: Some(&menu),
            ..Chrome::default()
        };

        let count = (w * h) as usize;
        let mut buf = vec![0x1122_3344u32; count + 16];
        frame_with(&page, &mut fonts, &mut buf[..count], w, h, 0, None, &chrome);
        assert_eq!(&buf[count..], &[0x1122_3344u32; 16]);
        assert_ne!(buf[0], 0x1122_3344u32);
    }
}
