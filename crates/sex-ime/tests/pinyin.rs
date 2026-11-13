use sex_ime::{syllables, Input, Outcome, Pinyin};

fn typed(p: &mut Pinyin, letters: &str) { for c in letters.bytes() { assert_eq!(p.key(Input::Letter(c)), Outcome::Changed); } }

#[test]
fn syllables_split_the_way_people_type() {
    let split = |s: &str| { let mut e = [0; 48]; let n = syllables::split(s, &mut e); e[..n].to_vec() };
    assert_eq!(split("nihao"), vec![2, 5]);
    assert_eq!(split("zhongguo"), vec![5, 8]);
    assert_eq!(split("xian"), vec![4]);
    assert_eq!(split("woaini"), vec![2, 4, 6]);
    assert_eq!(split("zh"), vec![2]);
}

#[test]
fn nihao_space_commits_nihao() {
    let mut p = Pinyin::new();
    typed(&mut p, "nihao");
    assert_eq!(p.preedit(), "nihao");
    assert_eq!(p.candidates().next(), Some("你好"));
    assert_eq!(p.key(Input::Space), Outcome::Commit);
    assert_eq!(p.committed(), "你好");
    assert!(!p.active());
    // Keys with nothing composed are the program's.
    assert_eq!(p.key(Input::Space), Outcome::Pass);
    assert_eq!(p.key(Input::Digit(1)), Outcome::Pass);
}

#[test]
fn longer_strings_commit_word_by_word_and_digits_choose() {
    let mut p = Pinyin::new();
    typed(&mut p, "zhongguoren");
    assert_eq!(p.key(Input::Space), Outcome::Commit);
    assert_eq!(p.committed(), "中国");
    assert_eq!(p.preedit(), "ren");
    assert_eq!(p.candidates().next(), Some("人"));
    assert_eq!(p.key(Input::Space), Outcome::Commit);
    assert_eq!(p.committed(), "人");
    typed(&mut p, "ma");
    let second = p.candidates().nth(1).unwrap();
    assert_eq!(p.key(Input::Digit(2)), Outcome::Commit);
    assert_eq!(p.committed(), second);
}

#[test]
fn enter_escape_backspace_and_pages() {
    let mut p = Pinyin::new();
    typed(&mut p, "shi");
    let first: Vec<_> = p.candidates().collect();
    assert_eq!(first.len(), 9);
    assert_eq!(p.key(Input::NextPage), Outcome::Changed);
    assert_eq!(p.page(), 1);
    assert_ne!(p.candidates().next(), Some(first[0]));
    p.key(Input::PrevPage);
    assert_eq!(p.candidates().next(), Some(first[0]));
    p.key(Input::Backspace);
    assert_eq!(p.preedit(), "sh");
    assert_eq!(p.key(Input::Enter), Outcome::Commit);
    assert_eq!(p.committed(), "sh");
    typed(&mut p, "abc");
    p.key(Input::Escape);
    assert!(!p.active());
}
