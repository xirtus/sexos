// SPDX-License-Identifier: GPL-3.0-only
//! Intra-slide build animations and effect transitions (Slice T6 / G-28).



/// Visual effect for an intra-slide build step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildEffect {
    Appear,
    DissolveIn,
    FlyInLeft,
    FlyInBottom,
    PopScale,
    WipeRight,
    FadeOut,
}

impl BuildEffect {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Appear => "Appear",
            Self::DissolveIn => "Dissolve In",
            Self::FlyInLeft => "Fly In (Left)",
            Self::FlyInBottom => "Fly In (Bottom)",
            Self::PopScale => "Pop Scale",
            Self::WipeRight => "Wipe Right",
            Self::FadeOut => "Fade Out",
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Appear => "appear",
            Self::DissolveIn => "dissolve_in",
            Self::FlyInLeft => "fly_in_left",
            Self::FlyInBottom => "fly_in_bottom",
            Self::PopScale => "pop_scale",
            Self::WipeRight => "wipe_right",
            Self::FadeOut => "fade_out",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        let clean = s.trim();
        if clean.eq_ignore_ascii_case("appear") {
            Self::Appear
        } else if clean.eq_ignore_ascii_case("dissolve")
            || clean.eq_ignore_ascii_case("dissolve_in")
            || clean.eq_ignore_ascii_case("dissolvein")
        {
            Self::DissolveIn
        } else if clean.eq_ignore_ascii_case("fly_in_left")
            || clean.eq_ignore_ascii_case("flyinleft")
            || clean.eq_ignore_ascii_case("fly_left")
        {
            Self::FlyInLeft
        } else if clean.eq_ignore_ascii_case("fly_in_bottom")
            || clean.eq_ignore_ascii_case("flyinbottom")
            || clean.eq_ignore_ascii_case("fly_bottom")
        {
            Self::FlyInBottom
        } else if clean.eq_ignore_ascii_case("pop")
            || clean.eq_ignore_ascii_case("pop_scale")
            || clean.eq_ignore_ascii_case("popscale")
        {
            Self::PopScale
        } else if clean.eq_ignore_ascii_case("wipe")
            || clean.eq_ignore_ascii_case("wipe_right")
            || clean.eq_ignore_ascii_case("wiperight")
        {
            Self::WipeRight
        } else if clean.eq_ignore_ascii_case("fade_out")
            || clean.eq_ignore_ascii_case("fadeout")
        {
            Self::FadeOut
        } else {
            Self::Appear
        }
    }

    /// Whether this effect is a build-out (dismiss) effect.
    pub fn is_build_out(&self) -> bool {
        matches!(self, Self::FadeOut)
    }
}

/// Trigger condition for advancing to the next build step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildTrigger {
    OnClick,
    AfterPrevious { delay_ms: u32 },
    WithPrevious,
}

impl Default for BuildTrigger {
    fn default() -> Self {
        Self::OnClick
    }
}

/// A single intra-slide build animation step.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildStep {
    pub element_index: usize,
    pub paragraph_index: Option<usize>,
    pub effect: BuildEffect,
    pub duration_ms: u32,
    pub trigger: BuildTrigger,
    pub rehearsed_delay_ms: Option<u32>,
}

impl BuildStep {
    pub fn new(element_index: usize, effect: BuildEffect) -> Self {
        Self {
            element_index,
            paragraph_index: None,
            effect,
            duration_ms: 400,
            trigger: BuildTrigger::OnClick,
            rehearsed_delay_ms: None,
        }
    }

    pub fn with_paragraph(mut self, paragraph_index: usize) -> Self {
        self.paragraph_index = Some(paragraph_index);
        self
    }

    pub fn with_duration(mut self, duration_ms: u32) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    pub fn with_trigger(mut self, trigger: BuildTrigger) -> Self {
        self.trigger = trigger;
        self
    }
}
