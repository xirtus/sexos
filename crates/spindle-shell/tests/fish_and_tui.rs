// SPDX-License-Identifier: GPL-3.0-only
mod common;
use common::*;
use spindle_shell::shell_ui::{
    accept_ghost_word, complete_with_extras, format_completion_menu, format_reverse_search,
    fuzzy_search_history, ghost_suggestion, history_search_prefix,
};
use spindle_shell::tui::{BarsApp, BarsMode, BtopApp, TuiExit, TuiKey, YaziApp};
use spindle_shell::{ergonomics::SHELL_EXTRA_CMDS, Engine, Style, Value};

#[test]
fn void_aliases_and_session_commands() {
    let mut e = Engine::new();
    let mut h = host();
    assert_eq!(run(&mut e, &mut h, "cd src; ..; pwd").value, Value::string("/home"));
    assert_eq!(run(&mut e, &mut h, "alias go_docs='cd docs'").error, None);
    assert_eq!(run(&mut e, &mut h, "go_docs; pwd").value, Value::string("/home/docs"));
    run(&mut e, &mut h, "..");
    assert_eq!(run(&mut e, &mut h, "unalias go_docs").error, None);
    assert!(run(&mut e, &mut h, "go_docs").error.is_some());

    assert_eq!(run(&mut e, &mut h, "export EDITOR=bat").error, None);
    assert_eq!(run(&mut e, &mut h, "$EDITOR").value, Value::string("bat"));

    let w_fetch = run(&mut e, &mut h, "which fetch");
    assert!(w_fetch.text.contains("aliased to 'fastfetch'"), "{}", w_fetch.text);
    assert_eq!(w_fetch.exit_code, 0);

    let w_miss = run(&mut e, &mut h, "which definitely_missing");
    assert!(w_miss.text.contains("not found"), "{}", w_miss.text);
    assert_eq!(w_miss.exit_code, 1);
}

#[test]
fn rich_linux_builtins_ll_tree_bat_fetch() {
    let mut e = Engine::new();
    let mut h = host();

    let ll = run(&mut e, &mut h, "ll");
    assert!(ll.error.is_none(), "{}", ll.text);
    assert!(ll.text.contains("drwxr-xr-x") && ll.text.contains("notes.txt"), "{}", ll.text);

    let tr = run(&mut e, &mut h, "tree");
    assert!(tr.error.is_none(), "{}", tr.text);
    assert!(tr.text.contains("├──") && tr.text.contains("└──") && tr.text.contains("directories"), "{}", tr.text);

    let bat = run(&mut e, &mut h, "bat src/main.rs");
    assert!(bat.error.is_none(), "{}", bat.text);
    assert!(bat.text.contains("╭─ src/main.rs") && bat.text.contains("1 │"), "{}", bat.text);

    let fetch = run(&mut e, &mut h, "fetch");
    assert!(fetch.error.is_none(), "{}", fetch.text);
    assert!(fetch.text.contains("root@") && fetch.text.contains("Kernel") && fetch.text.contains("Spindle"), "{}", fetch.text);
}

#[test]
fn host_aware_syntax_highlighting() {
    let e = Engine::new();
    let h = host();

    let spans = e.highlight_with("fetch notes.txt", &h, SHELL_EXTRA_CMDS);
    assert_eq!(spans[0].1, Style::Alias);
    assert_eq!(spans[1].1, Style::Path);
    assert!(Style::Alias.ansi_truecolor().contains("38;2;"));

    let bad = e.highlight_with("not_a_cmd -f \"str\"", &h, SHELL_EXTRA_CMDS);
    assert_eq!(bad[0].1, Style::Unknown);
    assert_eq!(bad[1].1, Style::Flag);
    assert_eq!(bad[2].1, Style::String);
    assert!(Style::Unknown.ansi_truecolor().starts_with("\x1b[4;"));
}

#[test]
fn fish_autosuggestions_ctrl_f_and_prefix_history() {
    let e = Engine::new();
    let h = host();
    let themes = ["noir", "nord", "mocha"];
    let history = vec![
        String::from("ls src"),
        String::from("cd docs"),
        String::from("cat notes.txt"),
        String::from("cd notes/sub"),
    ];

    // Theme completion
    let th = complete_with_extras("theme no", 8, &h, &e, SHELL_EXTRA_CMDS, &themes);
    let names: Vec<&str> = th.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(names, ["noir", "nord"]);

    // Ghost suggestion prioritizes most recent history prefix match
    let g = ghost_suggestion("cd ", 3, &history, &h, &e, SHELL_EXTRA_CMDS, &themes).unwrap();
    assert_eq!(g, "notes/sub");

    // Word-wise Ctrl+F acceptance stops cleanly at `/` then consumes next word
    let mut line = String::from("cd ");
    let mut cursor = line.len();
    assert!(accept_ghost_word(&mut line, &mut cursor, &g));
    assert_eq!(line, "cd notes/");
    let rest = ghost_suggestion(&line, cursor, &history, &h, &e, SHELL_EXTRA_CMDS, &themes).unwrap();
    assert!(accept_ghost_word(&mut line, &mut cursor, &rest));
    assert_eq!(line, "cd notes/sub");

    // Prefix-filtered Up/Down history search (`up-line-or-search`)
    let i1 = history_search_prefix(&history, "cd ", None, true);
    assert_eq!(i1, Some(Some(3)));
    let i2 = history_search_prefix(&history, "cd ", Some(3), true);
    assert_eq!(i2, Some(Some(1)));
    let i3 = history_search_prefix(&history, "cd ", Some(1), true);
    assert_eq!(i3, None);
    let d1 = history_search_prefix(&history, "cd ", Some(1), false);
    assert_eq!(d1, Some(Some(3)));
    let d2 = history_search_prefix(&history, "cd ", Some(3), false);
    assert_eq!(d2, Some(None));

    // Fuzzy Ctrl+R reverse search + overlay formatters
    let fm = fuzzy_search_history(&history, "nts", 8);
    assert!(!fm.is_empty());
    assert_eq!(fm[0].text, "cd notes/sub");
    let (rev_ansi, rev_rows) = format_reverse_search("nts", &fm, 0, 80, 6, true);
    assert!(rev_rows >= 2 && rev_ansi.contains("bck-i-search"));

    let cmds = complete_with_extras("bt", 2, &h, &e, SHELL_EXTRA_CMDS, &themes);
    assert!(cmds.iter().any(|c| c.text == "btop"));
    let (menu_ansi, menu_rows) = format_completion_menu(&th, 0, 80, 6, true);
    assert!(menu_rows >= 2 && menu_ansi.contains("noir"));
}

#[test]
fn bars_app_animates_to_completion_and_cancels() {
    let mut app = BarsApp::new(BarsMode::Update, &[], 0);
    let init = app.init(80, 0);
    assert!(init.exit.is_none());
    assert!(init.bytes.contains("╭─") && init.bytes.contains("sexshop"));

    let mut now = 40u64;
    let mut done = false;
    for _ in 0..80 {
        let step = app.tick(80, now);
        if let Some(TuiExit::Done) = step.exit {
            assert!(step.bytes.contains("✓") && step.bytes.contains("\x1b[?25h"));
            done = true;
            break;
        }
        now += 40;
    }
    assert!(done, "bars should reach 100% completion");

    let mut cancel_app = BarsApp::new(BarsMode::Demo, &[], 0);
    let _ = cancel_app.init(80, 0);
    let quit = cancel_app.key(TuiKey::Char('q'), 80);
    assert_eq!(quit.exit, Some(TuiExit::Cancel));
    assert!(quit.bytes.contains("\x1b[?25h"));
}

#[test]
fn btop_app_renders_telemetry_filters_and_exits() {
    let h = host();
    let mut app = BtopApp::new(0);
    let init = app.init(&h, 96, 28, 0);
    assert!(init.bytes.contains("\x1b[?1049h\x1b[?25l"));
    assert!(init.bytes.contains("CPU") && init.bytes.contains("Memory") && init.bytes.contains("silk-shell"));

    // Filter mode `/silk`
    let f0 = app.key(TuiKey::Char('/'), &h, 96, 28, 50);
    assert!(f0.bytes.contains("/_"));
    let _ = app.key(TuiKey::Char('z'), &h, 96, 28, 60);
    let fz = app.key(TuiKey::Char('z'), &h, 96, 28, 70);
    assert!(fz.bytes.contains("0/0"));
    let _ = app.key(TuiKey::Escape, &h, 96, 28, 80); // exit filter clearing query

    // Sort cycle + exit
    let sorted = app.key(TuiKey::Char('s'), &h, 96, 28, 100);
    assert!(sorted.bytes.contains("pid▲"));

    let exit = app.key(TuiKey::Char('q'), &h, 96, 28, 120);
    assert_eq!(exit.exit, Some(TuiExit::Done));
    assert!(exit.bytes.contains("\x1b[?25h\x1b[?1049l"));
}

#[test]
fn yazi_app_navigates_previews_and_cd_exits() {
    let h = host();
    let mut app = YaziApp::new(&h, None).unwrap();
    let init = app.init(100, 24);
    assert!(init.bytes.contains("\x1b[?1049h\x1b[?25l"));
    assert!(init.bytes.contains("Parent") && init.bytes.contains("Files") && init.bytes.contains("/home"));

    // Directories are sorted first (`docs`, `src`), move down to `src` and enter it
    let _ = app.key(TuiKey::Char('j'), &h, 100, 24);
    let entered = app.key(TuiKey::Enter, &h, 100, 24);
    assert_eq!(app.cwd(), "/home/src");
    assert!(entered.bytes.contains("/home/src"));

    // Toggle hidden files on root/home after going back
    let back = app.key(TuiKey::Char('h'), &h, 100, 24);
    assert_eq!(app.cwd(), "/home");
    assert!(back.bytes.contains("/home"));
    // `src` remained selected when returning from child, and stays selected after toggling hidden
    let hid = app.key(TuiKey::Char('.'), &h, 100, 24);
    assert!(hid.bytes.contains("hidden:on") && hid.bytes.contains(".hidden"));

    // Enter selected `src` again and exit with `q` -> TuiExit::Cd("/home/src")
    let _ = app.key(TuiKey::Char('l'), &h, 100, 24);
    assert_eq!(app.cwd(), "/home/src");
    let q_out = app.key(TuiKey::Char('q'), &h, 100, 24);
    assert_eq!(q_out.exit, Some(TuiExit::Cd(String::from("/home/src"))));
    assert!(q_out.bytes.contains("\x1b[?25h\x1b[?1049l"));

    // Escape exits with Cancel
    let mut app2 = YaziApp::new(&h, Some("docs")).unwrap();
    let esc_out = app2.key(TuiKey::Escape, &h, 100, 24);
    assert_eq!(esc_out.exit, Some(TuiExit::Cancel));
}
