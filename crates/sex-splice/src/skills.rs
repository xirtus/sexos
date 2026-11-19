// SPDX-License-Identifier: GPL-3.0-only
//! Inheritable editing skills, workflow presets, and standing-rule learnings.
//!
//! Adapted from Scarlet's `skills/` inheritance and `learning_check` standing-rule
//! approvals (MIT).

use crate::Micros;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A discrete editing workflow or skill definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub extends: Option<String>,
    pub target_pause_us: Option<Micros>,
    pub crossfade_frames: Option<u32>,
    pub max_kept_silence_us: Option<Micros>,
    pub body: String,
    pub user_edited: bool,
}

/// A fully resolved skill with inherited instructions and merged scalar properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: String,
    pub chain: Vec<String>,
    pub target_pause_us: Micros,
    pub crossfade_frames: u32,
    pub max_kept_silence_us: Micros,
    pub instructions: String,
}

/// A proposal to learn or update a standing rule in a skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningProposal {
    pub skill_name: String,
    pub rule_sentence: String,
    pub overrides_quote: Option<String>,
}

/// Possible errors when applying a learned rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearningError {
    SkillNotFound,
    QuoteNotFound,
    AlreadyPresent,
    ContradictsOtherPassage(String),
}

/// Parses a skill from markdown with YAML-style frontmatter `--- ... ---`.
pub fn parse_skill_md(md: &str) -> Option<Skill> {
    let s = md.trim_start();
    if !s.starts_with("---") {
        return None;
    }

    let rest = &s[3..];
    let rest = rest.strip_prefix('\r').unwrap_or(rest);
    let rest = rest.strip_prefix('\n')?;

    // Find the closing --- delimiter on its own line
    let mut fm_end = None;
    let mut search_idx = 0;
    while let Some(line_end) = rest[search_idx..].find('\n') {
        let abs_line_end = search_idx + line_end;
        let line = rest[search_idx..abs_line_end].trim_end_matches('\r').trim();
        if line == "---" {
            fm_end = Some((search_idx, abs_line_end));
            break;
        }
        search_idx = abs_line_end + 1;
    }

    let (fm_start_of_delim, fm_end_of_delim) = fm_end?;
    let fm_str = &rest[..fm_start_of_delim];
    let mut body_str = &rest[fm_end_of_delim + 1..];
    if let Some(b) = body_str.strip_prefix("\r\n") {
        body_str = b;
    } else if let Some(b) = body_str.strip_prefix('\n') {
        body_str = b;
    }

    let mut name = String::new();
    let mut description = String::new();
    let mut extends: Option<String> = None;
    let mut target_pause_us: Option<Micros> = None;
    let mut crossfade_frames: Option<u32> = None;
    let mut max_kept_silence_us: Option<Micros> = None;
    let mut user_edited = false;

    for raw_line in fm_str.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let val = val.trim().trim_matches('"').trim_matches('\'').trim();

        match key.as_str() {
            "name" => name = val.to_string(),
            "description" => description = val.to_string(),
            "extends" => {
                if val.is_empty() || val == "none" || val == "null" || val == "~" {
                    extends = None;
                } else {
                    extends = Some(val.to_string());
                }
            }
            "target_pause_ms" => {
                if let Ok(ms) = val.parse::<i64>() {
                    target_pause_us = Some(ms * 1000);
                }
            }
            "target_pause_us" => {
                if let Ok(us) = val.parse::<i64>() {
                    target_pause_us = Some(us);
                }
            }
            "crossfade_frames" => {
                if let Ok(cf) = val.parse::<u32>() {
                    crossfade_frames = Some(cf);
                }
            }
            "max_kept_silence_ms" => {
                if let Ok(ms) = val.parse::<i64>() {
                    max_kept_silence_us = Some(ms * 1000);
                }
            }
            "max_kept_silence_us" => {
                if let Ok(us) = val.parse::<i64>() {
                    max_kept_silence_us = Some(us);
                }
            }
            "user_edited" => {
                if let Ok(b) = val.parse::<bool>() {
                    user_edited = b;
                }
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(Skill {
        name,
        description,
        extends,
        target_pause_us,
        crossfade_frames,
        max_kept_silence_us,
        body: body_str.to_string(),
        user_edited,
    })
}

/// A library of editing skills with inheritance and learning management.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillLibrary {
    skills: Vec<Skill>,
}

impl SkillLibrary {
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    /// Built-in stock workflows adapted cleanly for Splicy.
    pub fn with_stock() -> Self {
        let mut lib = Self::new();

        lib.add(Skill {
            name: "dialogue-pass".into(),
            description: "Clean cut pass removing filler words and shortening long pauses".into(),
            extends: None,
            target_pause_us: Some(300_000),
            crossfade_frames: Some(0),
            max_kept_silence_us: Some(2_000_000),
            body: "# Dialogue Pass\n\nCut hesitation sounds (um, uh, er, mm) and repeated takes. Shorten silences longer than 2s down to 300ms, preserving natural breath.".into(),
            user_edited: false,
        });

        lib.add(Skill {
            name: "tight-talking-head".into(),
            description: "Pacey talking head edit with tighter pauses and audio crossfades".into(),
            extends: Some("dialogue-pass".into()),
            target_pause_us: Some(180_000),
            crossfade_frames: Some(2),
            max_kept_silence_us: None,
            body: "# Tight Talking Head\n\nMaintain brisk conversational rhythm. Apply 2-frame crossfades at dialogue cuts to eliminate room-tone pops.".into(),
            user_edited: false,
        });

        lib.add(Skill {
            name: "speed-through-waits".into(),
            description: "Speeds through long waits and non-speech pauses instead of ripple cutting".into(),
            extends: Some("dialogue-pass".into()),
            target_pause_us: None,
            crossfade_frames: None,
            max_kept_silence_us: None,
            body: "# Speed Through Waits\n\nDo not ripple delete long silences or waiting sections; speed them up by 4x to 8x to retain continuous visual context.".into(),
            user_edited: false,
        });

        lib.add(Skill {
            name: "cold-open".into(),
            description: "Hooks viewer with an exciting moment before the main intro".into(),
            extends: None,
            target_pause_us: Some(250_000),
            crossfade_frames: Some(0),
            max_kept_silence_us: Some(1_500_000),
            body: "# Cold Open\n\nExtract the most captivating 10-20s hook from later in the footage and place it before the title card or introduction.".into(),
            user_edited: false,
        });

        lib.add(Skill {
            name: "fit-length".into(),
            description: "Trims pauses and low-density sections to hit a target total duration".into(),
            extends: None,
            target_pause_us: Some(200_000),
            crossfade_frames: Some(1),
            max_kept_silence_us: Some(1_000_000),
            body: "# Fit Length\n\nCompress pauses progressively and drop lower-priority segments until the edit fits within the target time limit.".into(),
            user_edited: false,
        });

        lib.add(Skill {
            name: "make-shorts".into(),
            description: "Extracts punchy 30-60 second vertical clips with tight dialogue".into(),
            extends: None,
            target_pause_us: Some(150_000),
            crossfade_frames: Some(1),
            max_kept_silence_us: Some(800_000),
            body: "# Make Shorts\n\nSelect standalone insightful moments. Keep pacing extremely tight (<150ms pauses) and optimize for short-form retention.".into(),
            user_edited: false,
        });

        lib
    }

    pub fn add(&mut self, skill: Skill) {
        if let Some(pos) = self.skills.iter().position(|s| s.name == skill.name) {
            self.skills[pos] = skill;
        } else {
            self.skills.push(skill);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Skill> {
        self.skills.iter_mut().find(|s| s.name == name)
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Resolves the full inheritance chain, merging instructions base-first and
    /// child overrides on top for scalar settings. Returns `None` on cycle or missing parent.
    pub fn resolve(&self, name: &str) -> Option<ResolvedSkill> {
        let mut chain_rev = Vec::new();
        let mut current_name = name.to_string();

        loop {
            if chain_rev.contains(&current_name) {
                // Cycle detected!
                return None;
            }
            let skill = self.get(&current_name)?;
            chain_rev.push(current_name.clone());
            if let Some(ref parent) = skill.extends {
                current_name = parent.clone();
            } else {
                break;
            }
        }

        let chain: Vec<String> = chain_rev.into_iter().rev().collect();

        let mut target_pause_us = 300_000;
        let mut crossfade_frames = 0;
        let mut max_kept_silence_us = 2_000_000;
        let mut instructions_parts = Vec::new();

        for s_name in &chain {
            let s = self.get(s_name)?;
            if let Some(tp) = s.target_pause_us {
                target_pause_us = tp;
            }
            if let Some(cf) = s.crossfade_frames {
                crossfade_frames = cf;
            }
            if let Some(ms) = s.max_kept_silence_us {
                max_kept_silence_us = ms;
            }
            let trimmed = s.body.trim();
            if !trimmed.is_empty() {
                instructions_parts.push(trimmed);
            }
        }

        let instructions = instructions_parts.join("\n\n");

        Some(ResolvedSkill {
            name: name.to_string(),
            chain,
            target_pause_us,
            crossfade_frames,
            max_kept_silence_us,
            instructions,
        })
    }

    /// Applies an approved standing-rule learning.
    ///
    /// Verifies `rule_sentence` isn't already verbatim in the skill, verifies
    /// `overrides_quote` (if `Some`) exists verbatim in the skill's `body` (returning
    /// `Err(LearningError::QuoteNotFound)` if hallucinated), replaces the quoted
    /// passage (or appends under `## Learned Rules`), and marks `skill.user_edited = true`.
    pub fn apply_approved_learning(&mut self, p: &LearningProposal) -> Result<(), LearningError> {
        let skill = self.get_mut(&p.skill_name).ok_or(LearningError::SkillNotFound)?;

        if skill.body.contains(&p.rule_sentence) {
            return Err(LearningError::AlreadyPresent);
        }

        if let Some(ref quote) = p.overrides_quote {
            if quote.is_empty() || !skill.body.contains(quote) {
                return Err(LearningError::QuoteNotFound);
            }
            skill.body = skill.body.replacen(quote, &p.rule_sentence, 1);
        } else {
            if skill.body.contains("## Learned Rules") {
                if !skill.body.ends_with('\n') {
                    skill.body.push('\n');
                }
                skill.body.push_str(&format!("- {}\n", p.rule_sentence));
            } else {
                if !skill.body.ends_with('\n') {
                    skill.body.push('\n');
                }
                skill.body.push_str("\n## Learned Rules\n");
                skill.body.push_str(&format!("- {}\n", p.rule_sentence));
            }
        }

        skill.user_edited = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_parse_skill_md() {
        let md = r#"---
name: custom-vlog
description: Snappy vlog editing workflow
extends: dialogue-pass
target_pause_ms: 150
crossfade_frames: 3
max_kept_silence_ms: 1200
---
# Custom Vlog Instructions

Trim aggressively on reaction shots.
"#;
        let skill = parse_skill_md(md).expect("should parse skill");
        assert_eq!(skill.name, "custom-vlog");
        assert_eq!(skill.description, "Snappy vlog editing workflow");
        assert_eq!(skill.extends, Some("dialogue-pass".into()));
        assert_eq!(skill.target_pause_us, Some(150_000));
        assert_eq!(skill.crossfade_frames, Some(3));
        assert_eq!(skill.max_kept_silence_us, Some(1_200_000));
        assert!(skill.body.contains("# Custom Vlog Instructions"));
        assert!(!skill.user_edited);
    }

    #[test]
    fn test_stock_library_and_inheritance() {
        let lib = SkillLibrary::with_stock();

        // Dialogue pass resolution
        let base = lib.resolve("dialogue-pass").expect("dialogue-pass exists");
        assert_eq!(base.name, "dialogue-pass");
        assert_eq!(base.chain, vec![String::from("dialogue-pass")]);
        assert_eq!(base.target_pause_us, 300_000);
        assert_eq!(base.crossfade_frames, 0);
        assert_eq!(base.max_kept_silence_us, 2_000_000);

        // Tight talking head inherits dialogue-pass
        let child = lib.resolve("tight-talking-head").expect("tight-talking-head exists");
        assert_eq!(child.name, "tight-talking-head");
        assert_eq!(child.chain, vec![String::from("dialogue-pass"), String::from("tight-talking-head")]);
        // Overridden
        assert_eq!(child.target_pause_us, 180_000);
        assert_eq!(child.crossfade_frames, 2);
        // Inherited
        assert_eq!(child.max_kept_silence_us, 2_000_000);
        // Instructions contain base instructions first, child overrides on top
        assert!(child.instructions.contains("# Dialogue Pass"));
        assert!(child.instructions.contains("# Tight Talking Head"));
        let pos_base = child.instructions.find("# Dialogue Pass").unwrap();
        let pos_child = child.instructions.find("# Tight Talking Head").unwrap();
        assert!(pos_base < pos_child);
    }

    #[test]
    fn test_cycle_detection() {
        let mut lib = SkillLibrary::new();
        lib.add(Skill {
            name: "a".into(),
            description: "".into(),
            extends: Some("b".into()),
            target_pause_us: None,
            crossfade_frames: None,
            max_kept_silence_us: None,
            body: "".into(),
            user_edited: false,
        });
        lib.add(Skill {
            name: "b".into(),
            description: "".into(),
            extends: Some("a".into()),
            target_pause_us: None,
            crossfade_frames: None,
            max_kept_silence_us: None,
            body: "".into(),
            user_edited: false,
        });

        assert_eq!(lib.resolve("a"), None);
        assert_eq!(lib.resolve("non-existent"), None);
    }

    #[test]
    fn test_apply_approved_learning() {
        let mut lib = SkillLibrary::with_stock();

        // 1. SkillNotFound
        let p_bad_skill = LearningProposal {
            skill_name: "ghost-skill".into(),
            rule_sentence: "Do not cut laugh.".into(),
            overrides_quote: None,
        };
        assert_eq!(
            lib.apply_approved_learning(&p_bad_skill),
            Err(LearningError::SkillNotFound)
        );

        // 2. QuoteNotFound
        let p_bad_quote = LearningProposal {
            skill_name: "dialogue-pass".into(),
            rule_sentence: "Never cut laugh.".into(),
            overrides_quote: Some("This quote does not exist in stock skill body".into()),
        };
        assert_eq!(
            lib.apply_approved_learning(&p_bad_quote),
            Err(LearningError::QuoteNotFound)
        );

        // 3. Append new learned rule
        let p_append = LearningProposal {
            skill_name: "dialogue-pass".into(),
            rule_sentence: "Never cut during an audible inhale.".into(),
            overrides_quote: None,
        };
        assert_eq!(lib.apply_approved_learning(&p_append), Ok(()));

        let skill = lib.get("dialogue-pass").unwrap();
        assert!(skill.user_edited);
        assert!(skill.body.contains("## Learned Rules"));
        assert!(skill.body.contains("- Never cut during an audible inhale."));

        // 4. AlreadyPresent
        assert_eq!(
            lib.apply_approved_learning(&p_append),
            Err(LearningError::AlreadyPresent)
        );

        // 5. Replace existing quote
        let p_replace = LearningProposal {
            skill_name: "dialogue-pass".into(),
            rule_sentence: "Shorten silences longer than 1.5s down to 250ms.".into(),
            overrides_quote: Some("Shorten silences longer than 2s down to 300ms,".into()),
        };
        assert_eq!(lib.apply_approved_learning(&p_replace), Ok(()));

        let skill_after = lib.get("dialogue-pass").unwrap();
        assert!(skill_after.body.contains("Shorten silences longer than 1.5s down to 250ms."));
        assert!(!skill_after.body.contains("Shorten silences longer than 2s down to 300ms,"));
    }

    #[test]
    fn test_all_stock_skills_resolvable() {
        let lib = SkillLibrary::with_stock();
        let expected = [
            "dialogue-pass",
            "tight-talking-head",
            "speed-through-waits",
            "cold-open",
            "fit-length",
            "make-shorts",
        ];
        for name in &expected {
            let res = lib.resolve(name);
            assert!(res.is_some(), "Stock skill {} must be resolvable", name);
            let skill = res.unwrap();
            assert_eq!(&skill.name, name);
            assert!(!skill.instructions.is_empty());
        }
    }

    #[test]
    fn test_three_level_inheritance() {
        let mut lib = SkillLibrary::new();
        lib.add(Skill {
            name: "level-1".into(),
            description: "Root".into(),
            extends: None,
            target_pause_us: Some(500_000),
            crossfade_frames: Some(0),
            max_kept_silence_us: Some(3_000_000),
            body: "# Level 1\nBase rule.".into(),
            user_edited: false,
        });
        lib.add(Skill {
            name: "level-2".into(),
            description: "Mid".into(),
            extends: Some("level-1".into()),
            target_pause_us: Some(300_000),
            crossfade_frames: None,
            max_kept_silence_us: None,
            body: "# Level 2\nMid rule.".into(),
            user_edited: false,
        });
        lib.add(Skill {
            name: "level-3".into(),
            description: "Leaf".into(),
            extends: Some("level-2".into()),
            target_pause_us: None,
            crossfade_frames: Some(4),
            max_kept_silence_us: Some(1_500_000),
            body: "# Level 3\nLeaf rule.".into(),
            user_edited: false,
        });

        let res = lib.resolve("level-3").expect("should resolve level-3");
        assert_eq!(res.chain, vec![String::from("level-1"), String::from("level-2"), String::from("level-3")]);
        // target_pause_us: 500k -> 300k (level-2) -> None (level-3) => 300k
        assert_eq!(res.target_pause_us, 300_000);
        // crossfade_frames: 0 -> None -> 4 => 4
        assert_eq!(res.crossfade_frames, 4);
        // max_kept_silence_us: 3M -> None -> 1.5M => 1.5M
        assert_eq!(res.max_kept_silence_us, 1_500_000);
        // Instructions order
        assert!(res.instructions.contains("# Level 1\nBase rule.\n\n# Level 2\nMid rule.\n\n# Level 3\nLeaf rule."));
    }
}
