// SPDX-License-Identifier: GPL-3.0-only
//! Pure `#![no_std]` + `alloc` `SKILL.md` frontmatter parser and system-prompt formatter.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillDoc {
    pub name: String,
    pub description: String,
    pub body: String,
    pub path: String,
}

/// Parses a `SKILL.md` document with YAML frontmatter delimited by `---`.
///
/// Extracts `name` and `description` from the frontmatter and preserves the markdown body.
/// Returns `None` if frontmatter delimiters are missing or `name` is empty.
pub fn parse_skill_md(path: &str, raw: &str) -> Option<SkillDoc> {
    let content = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = content.split('\n');

    let first = lines.next()?.trim_end_matches('\r').trim();
    if first != "---" {
        return None;
    }

    let mut frontmatter_lines: Vec<&str> = Vec::new();
    let mut found_closing = false;
    let after_first = match content.find('\n') {
        Some(idx) => &content[idx + 1..],
        None => return None,
    };

    let mut cursor = 0usize;
    let mut body_start = 0usize;
    for seg in after_first.split('\n') {
        let seg_len = seg.len();
        let trimmed = seg.trim_end_matches('\r').trim();
        let next_cursor = (cursor + seg_len + 1).min(after_first.len());
        if trimmed == "---" {
            found_closing = true;
            body_start = next_cursor;
            break;
        }
        frontmatter_lines.push(seg.trim_end_matches('\r'));
        cursor = next_cursor;
    }

    if !found_closing {
        return None;
    }

    let mut name = String::new();
    let mut description = String::new();
    let mut active_key: Option<&str> = None;

    for line in frontmatter_lines {
        let trimmed_start = line.trim_start();
        if trimmed_start.is_empty() || trimmed_start.starts_with('#') {
            continue;
        }

        // Indented continuation line for multiline scalar values.
        if (line.starts_with(' ') || line.starts_with('\t')) && active_key.is_some() {
            let cont = trimmed_start.trim_end();
            match active_key {
                Some("name") => {
                    if !name.is_empty() {
                        name.push(' ');
                    }
                    name.push_str(cont);
                }
                Some("description") => {
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(cont);
                }
                _ => {}
            }
            continue;
        }

        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = unquote_yaml(v.trim());
            if key == "name" {
                name = if val == ">" || val == "|" || val == ">-" || val == "|-" {
                    String::new()
                } else {
                    String::from(val)
                };
                active_key = Some("name");
            } else if key == "description" {
                description = if val == ">" || val == "|" || val == ">-" || val == "|-" {
                    String::new()
                } else {
                    String::from(val)
                };
                active_key = Some("description");
            } else {
                active_key = None;
            }
        } else {
            active_key = None;
        }
    }

    let name = String::from(name.trim());
    if name.is_empty() {
        return None;
    }
    let description = String::from(description.trim());
    let body = String::from(after_first[body_start..].trim_start_matches(['\r', '\n']));

    Some(SkillDoc {
        name,
        description,
        body,
        path: String::from(path),
    })
}

fn unquote_yaml(s: &str) -> &str {
    if s.len() >= 2 {
        if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Formats a list of parsed [`SkillDoc`]s into a system-prompt `<skills>` section.
pub fn format_skills_prompt(skills: &[SkillDoc]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<skills>\nYou have access to the following specialized skills. If a skill is relevant to the current task, read its SKILL.md instructions before proceeding:\n",
    );
    for skill in skills {
        if skill.description.is_empty() {
            let _ = writeln!(&mut out, "- {} ({})", skill.name, skill.path);
        } else {
            let _ = writeln!(
                &mut out,
                "- {} ({}): {}",
                skill.name, skill.path, skill.description
            );
        }
    }
    out.push_str("</skills>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parses_standard_skill_md() {
        let raw = "\
---
name: rust-kernel
description: \"Guide for building SexOS kernel modules.\"
---

# Rust Kernel Skill

Always use `#![no_std]`.
";
        let doc = parse_skill_md(".agents/skills/rust-kernel/SKILL.md", raw).unwrap();
        assert_eq!(doc.name, "rust-kernel");
        assert_eq!(doc.description, "Guide for building SexOS kernel modules.");
        assert_eq!(doc.path, ".agents/skills/rust-kernel/SKILL.md");
        assert!(doc.body.starts_with("# Rust Kernel Skill"));
        assert!(doc.body.contains("#![no_std]"));
    }

    #[test]
    fn parses_multiline_description_and_crlf() {
        let raw = "---\r\nname: 'deploy'\r\ndescription: >\r\n  Deploy services\r\n  safely.\r\n---\r\nBody text.";
        let doc = parse_skill_md("skills/deploy/SKILL.md", raw).unwrap();
        assert_eq!(doc.name, "deploy");
        assert_eq!(doc.description, "Deploy services safely.");
        assert_eq!(doc.body, "Body text.");
    }

    #[test]
    fn rejects_missing_frontmatter_or_name() {
        assert!(parse_skill_md("a.md", "# No frontmatter").is_none());
        assert!(parse_skill_md("b.md", "---\ndescription: missing name\n---\nbody").is_none());
        assert!(parse_skill_md("c.md", "---\nname: unclosed\nbody").is_none());
    }

    #[test]
    fn formats_skills_prompt() {
        assert!(format_skills_prompt(&[]).is_empty());
        let skills = vec![
            SkillDoc {
                name: String::from("alpha"),
                description: String::from("First skill"),
                body: String::new(),
                path: String::from("/skills/alpha/SKILL.md"),
            },
            SkillDoc {
                name: String::from("beta"),
                description: String::new(),
                body: String::new(),
                path: String::from("/skills/beta/SKILL.md"),
            },
        ];
        let prompt = format_skills_prompt(&skills);
        assert!(prompt.starts_with("<skills>"));
        assert!(prompt.contains("- alpha (/skills/alpha/SKILL.md): First skill"));
        assert!(prompt.contains("- beta (/skills/beta/SKILL.md)\n"));
        assert!(prompt.ends_with("</skills>"));
    }
}
