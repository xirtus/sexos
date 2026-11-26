// SPDX-License-Identifier: GPL-3.0-only
//! Conservative eligibility for a future explicit Chastity fill request.
//! This module only identifies a page and two controls; it never handles secrets.
use alloc::string::String;
use sex_html::dom::{Dom, NodeId};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Address,
    Insecure,
    Frame,
    NoLogin,
    Ambiguous,
    UnsafeAction,
    Empty,
    Oversize,
}

/// A bounded snapshot from two current form controls, created only by an
/// explicit browser command. No persistent browser profile field owns it.
pub struct SaveLogin {
    pub origin: String,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub username_node: NodeId,
    pub password_node: NodeId,
}

/// Capture only user/script-updated control values, never markup defaults.
/// The caller must recheck tab, document identity, origin and visibility both
/// before capture and before any eventual consented transfer.
pub fn save_login(dom: &Dom, url: &str, in_frame: bool) -> Result<SaveLogin, Refusal> {
    let pair = login(dom, url, in_frame)?;
    let user = dom.values.get(&pair.username).ok_or(Refusal::Empty)?;
    let pass = dom.values.get(&pair.password).ok_or(Refusal::Empty)?;
    if user.is_empty() || pass.is_empty() || user.contains('\0') || pass.contains('\0') {
        return Err(Refusal::Empty);
    }
    if user.len() > 4096 || pass.len() > 4096 { return Err(Refusal::Oversize); }
    Ok(SaveLogin {
        origin: pair.origin,
        username: Zeroizing::new(user.clone()),
        password: Zeroizing::new(pass.clone()),
        username_node: pair.username,
        password_node: pair.password,
    })
}

pub struct Login {
    /// Exact scheme, host and port supplied to Chastity, without page path.
    pub origin: String,
    pub username: NodeId,
    pub password: NodeId,
}

/// Only the shown top-level HTTPS page may name an account scope.
pub fn origin(url: &str, in_frame: bool) -> Result<String, Refusal> {
    if in_frame { return Err(Refusal::Frame); }
    let url = sex_fetch::Url::parse(url).map_err(|_| Refusal::Address)?;
    if !url.secure { return Err(Refusal::Insecure); }
    Ok(url.origin())
}

fn form(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        node = dom.nodes.get(node)?.parent?;
        if dom.tag(node) == Some("form") { return Some(node); }
    }
}

fn usable(dom: &Dom, id: NodeId) -> bool {
    dom.tag(id) == Some("input")
        && dom.attr(id, "disabled").is_none()
        && dom.attr(id, "readonly").is_none()
        && dom.attr(id, "hidden").is_none()
}

fn action_ok(dom: &Dom, id: NodeId, url: &str, scope: &str) -> bool {
    if dom.attr(id, "autocomplete").is_some_and(|a| a.eq_ignore_ascii_case("off")) { return false; }
    if !dom.attr(id, "method").is_some_and(|m| m.eq_ignore_ascii_case("post")) { return false; }
    let Some(action) = sex_html::url::resolve(url, dom.attr(id, "action").unwrap_or("")) else { return false; };
    sex_fetch::Url::parse(&action).is_ok_and(|a| a.secure && a.origin() == scope)
}

/// Require one unambiguous username/password pair in one same-origin POST form.
/// Before consent and after approval, the caller must recheck the page URL,
/// tab, document identity and on-screen visibility of both controls.
pub fn login(dom: &Dom, url: &str, in_frame: bool) -> Result<Login, Refusal> {
    let scope = origin(url, in_frame)?;
    let mut chosen = None;
    let mut unsafe_action = false;
    for id in dom.descendants(Dom::ROOT) {
        if !usable(dom, id) || !dom.attr(id, "type").is_some_and(|t| t.eq_ignore_ascii_case("password")) { continue; }
        if dom.attr(id, "autocomplete").is_some_and(|a| a.eq_ignore_ascii_case("new-password") || a.eq_ignore_ascii_case("off")) { continue; }
        let Some(f) = form(dom, id) else { continue; };
        if !action_ok(dom, f, url, &scope) { unsafe_action = true; continue; }
        let mut users = VecIds::default();
        let mut passwords = 0;
        let mut before_password = true;
        for n in dom.descendants(f) {
            if !usable(dom, n) { continue; }
            if n == id { before_password = false; }
            let ty = dom.attr(n, "type").unwrap_or("text");
            if ty.eq_ignore_ascii_case("password") { passwords += 1; }
            if before_password && (ty.eq_ignore_ascii_case("text") || ty.eq_ignore_ascii_case("email")) { users.push(n); }
        }
        if passwords != 1 { return Err(Refusal::Ambiguous); }
        if users.many { return Err(Refusal::Ambiguous); }
        let Some(username) = users.one() else { continue; };
        if chosen.is_some() { return Err(Refusal::Ambiguous); }
        chosen = Some((username, id));
    }
    if let Some((username, password)) = chosen { return Ok(Login { origin: scope, username, password }); }
    if unsafe_action { Err(Refusal::UnsafeAction) } else { Err(Refusal::NoLogin) }
}

#[derive(Default)]
struct VecIds { one: Option<NodeId>, many: bool }
impl VecIds {
    fn push(&mut self, id: NodeId) { if self.one.is_some() { self.many = true; } else { self.one = Some(id); } }
    fn one(self) -> Option<NodeId> { if self.many { None } else { self.one } }
}
