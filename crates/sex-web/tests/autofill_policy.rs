// SPDX-License-Identifier: GPL-3.0-only
use sex_web::autofill_policy::{login, origin, Refusal};
use sex_web::sex_html::parse::parse;

const PAGE: &str = "https://example.org/login?next=%2F";

fn rejected(html: &str, url: &str, in_frame: bool, why: Refusal) {
    assert_eq!(login(&parse(html), url, in_frame).err(), Some(why));
}

#[test]
fn only_top_level_https_supplies_an_exact_account_scope() {
    assert_eq!(origin("https://EXAMPLE.ORG:443/login#part", false).unwrap(), "https://example.org");
    assert_eq!(origin("https://example.org:8443/login", false).unwrap(), "https://example.org:8443");
    assert_eq!(origin("http://example.org/login", false), Err(Refusal::Insecure));
    assert_eq!(origin(PAGE, true), Err(Refusal::Frame));
    assert_eq!(origin("about:site", false), Err(Refusal::Address));
    assert_eq!(origin("@login.html", false), Err(Refusal::Address));
}

#[test]
fn selects_only_one_pair_in_a_same_origin_post_form() {
    let html = "<form method=post action='/session'><input name=outside type=hidden><input name=user autocomplete=username><input name=pass type=password></form>";
    let dom = parse(html);
    let pair = login(&dom, PAGE, false).unwrap();
    assert_eq!(pair.origin, "https://example.org");
    assert_eq!(dom.attr(pair.username, "name"), Some("user"));
    assert_eq!(dom.attr(pair.password, "name"), Some("pass"));
}

#[test]
fn refuses_get_downgrade_cross_origin_and_frames() {
    let form = "<form method=post><input name=u><input name=p type=password></form>";
    rejected(form, "http://example.org/login", false, Refusal::Insecure);
    rejected(form, PAGE, true, Refusal::Frame);
    rejected("<form><input name=u><input name=p type=password></form>", PAGE, false, Refusal::UnsafeAction);
    rejected("<form method=post action='http://example.org/session'><input name=u><input name=p type=password></form>", PAGE, false, Refusal::UnsafeAction);
    rejected("<form method=post action='https://other.org/session'><input name=u><input name=p type=password></form>", PAGE, false, Refusal::UnsafeAction);
    rejected("<form method=post autocomplete=off><input name=u><input name=p type=password></form>", PAGE, false, Refusal::UnsafeAction);
}

#[test]
fn refuses_ambiguous_and_non_login_forms() {
    rejected("<input name=u><input name=p type=password>", PAGE, false, Refusal::NoLogin);
    rejected("<form method=post><input name=u><input name=second><input name=p type=password></form>", PAGE, false, Refusal::Ambiguous);
    rejected("<form method=post><input name=u><input name=p type=password><input name=confirm type=password></form>", PAGE, false, Refusal::Ambiguous);
    rejected("<form method=post><input name=u><input name=p type=password autocomplete=new-password></form>", PAGE, false, Refusal::NoLogin);
    rejected("<form method=post><input name=u><input name=p type=password disabled></form>", PAGE, false, Refusal::NoLogin);
    rejected("<form method=post><input name=u><input name=p type=password readonly></form>", PAGE, false, Refusal::NoLogin);
}
