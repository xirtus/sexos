//! Pure-Rust port of **advanced-scene-switcher** for Cabaret.
//!
//! Deterministic [`MacroEngine`] evaluating prioritized [`MacroRule`]s with composite boolean
//! conditions, hysteresis latches, dwell hold-time, and cooldown anti-flapping protection.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Reverse;

use cabaret_core::audio::{AudioMixer, StripId};
use cabaret_core::scene::{ItemId, SceneCollection, SourceId, StudioId};
use cabaret_core::transitions::StudioMode;

use crate::downstream_keyer::DownstreamKeyerBank;

/// Live streaming state observed by [`MacroEngine`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StreamActiveState {
    #[default]
    Stopped,
    Starting,
    Live,
    Reconnecting,
}

/// Live recording state observed by [`MacroEngine`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RecordingActiveState {
    #[default]
    Stopped,
    Recording,
    Paused,
}

/// Live media playback status for [`Condition::MediaRemainingMs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaPlaybackSnapshot {
    pub source_id: SourceId,
    pub remaining_ms: u64,
    pub playing: bool,
}

/// Trigger conditions supported by [`MacroRule`].
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// Matches currently focused Silk window by `frame_id` and/or substring of window title.
    FocusedSilkWindow {
        frame_id: Option<u64>,
        title_contains: Option<String>,
    },
    /// Triggers when mixer strip level crosses `threshold_db` with `hysteresis_db` band.
    AudioThresholdDb {
        strip_id: StripId,
        threshold_db: f32,
        above: bool,
        hysteresis_db: f32,
    },
    /// Triggers when a playing media source has `<= threshold_ms` remaining.
    MediaRemainingMs {
        source_id: SourceId,
        threshold_ms: u64,
    },
    /// Triggers deterministically on every `interval_ms` tick boundary.
    TimerIntervalMs {
        interval_ms: u64,
    },
    /// Triggers when user input idle time reaches or exceeds `min_idle_ms`.
    UserIdleMs {
        min_idle_ms: u64,
    },
    /// Matches live streaming state.
    StreamState {
        expected: StreamActiveState,
    },
    /// Logical conjunction of all child conditions.
    CompositeAnd(Vec<Condition>),
    /// Logical disjunction of child conditions.
    CompositeOr(Vec<Condition>),
}

/// Actions emitted when a [`MacroRule`] fires.
#[derive(Clone, Debug, PartialEq)]
pub enum MacroAction {
    SwitchStudio {
        studio_id: StudioId,
        preview_only: bool,
    },
    TriggerTake,
    SetSourceVisible {
        studio_id: StudioId,
        item_id: ItemId,
        visible: bool,
    },
    SetStripMute {
        strip_id: StripId,
        muted: bool,
    },
    StartStopRecord {
        start: bool,
    },
    SaveReplay {
        seconds: f32,
    },
    SetDskActive {
        channel_id: u64,
        active: bool,
    },
}

/// Prioritized automation macro rule with hysteresis hold and cooldown protection.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroRule {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    /// Higher numbers evaluate first.
    pub priority: i32,
    pub condition: Condition,
    pub actions: Vec<MacroAction>,
    /// Minimum milliseconds required before this rule can fire again.
    pub cooldown_ms: u64,
    /// Minimum continuous milliseconds the condition must remain true before firing.
    pub hold_ms: u64,
    /// When `true`, fires once per false -> true transition; when `false`, repeats after `cooldown_ms`.
    pub edge_triggered: bool,
}

impl MacroRule {
    pub fn new(
        id: u64,
        name: impl Into<String>,
        priority: i32,
        condition: Condition,
        actions: Vec<MacroAction>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            enabled: true,
            priority,
            condition,
            actions,
            cooldown_ms: 0,
            hold_ms: 0,
            edge_triggered: true,
        }
    }

    pub fn with_cooldown_ms(mut self, cooldown_ms: u64) -> Self {
        self.cooldown_ms = cooldown_ms;
        self
    }

    pub fn with_hold_ms(mut self, hold_ms: u64) -> Self {
        self.hold_ms = hold_ms;
        self
    }

    pub fn with_edge_triggered(mut self, edge_triggered: bool) -> Self {
        self.edge_triggered = edge_triggered;
        self
    }
}

/// Snapshot of system, Silk window, audio, media, and broadcast states for one evaluation tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroWorldState {
    pub now_ms: u64,
    pub focused_window_frame_id: Option<u64>,
    pub focused_window_title: String,
    pub strip_levels_db: BTreeMap<StripId, f32>,
    pub media: BTreeMap<SourceId, MediaPlaybackSnapshot>,
    pub user_idle_ms: u64,
    pub stream_state: StreamActiveState,
    pub recording_state: RecordingActiveState,
}

/// Event emitted when a [`MacroRule`] triggers during [`MacroEngine::evaluate`].
#[derive(Clone, Debug, PartialEq)]
pub struct MacroTriggerEvent {
    pub rule_id: u64,
    pub rule_name: String,
    pub priority: i32,
    pub fired_at_ms: u64,
    pub actions: Vec<MacroAction>,
}

#[derive(Clone, Debug, Default)]
struct RuleRuntimeState {
    condition_true_since_ms: Option<u64>,
    last_fired_ms: Option<u64>,
    was_active: bool,
    latches: BTreeMap<u64, bool>,
    timer_next_ms: BTreeMap<u64, u64>,
}

/// Deterministic rule evaluation engine for `advanced-scene-switcher`.
#[derive(Clone, Debug, Default)]
pub struct MacroEngine {
    pub rules: Vec<MacroRule>,
    states: BTreeMap<u64, RuleRuntimeState>,
}

impl MacroEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: MacroRule) {
        self.rules.push(rule);
    }

    pub fn remove_rule(&mut self, id: u64) -> Option<MacroRule> {
        let pos = self.rules.iter().position(|r| r.id == id)?;
        self.states.remove(&id);
        Some(self.rules.remove(pos))
    }

    pub fn set_rule_enabled(&mut self, id: u64, enabled: bool) -> bool {
        if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
            r.enabled = enabled;
            if !enabled {
                if let Some(st) = self.states.get_mut(&id) {
                    st.condition_true_since_ms = None;
                    st.was_active = false;
                }
            }
            true
        } else {
            false
        }
    }

    /// Evaluates all enabled rules in descending `(priority, id)` order against `world`
    /// and returns all triggered macro events.
    pub fn evaluate(&mut self, world: &MacroWorldState) -> Vec<MacroTriggerEvent> {
        let mut order: Vec<usize> = (0..self.rules.len()).collect();
        order.sort_by_key(|&i| (Reverse(self.rules[i].priority), self.rules[i].id));

        let mut events = Vec::new();

        for idx in order {
            let rule = &self.rules[idx];
            if !rule.enabled {
                continue;
            }
            let rt = self.states.entry(rule.id).or_default();
            let cond_met = eval_condition(&rule.condition, world, rt, 1);

            if !cond_met {
                rt.condition_true_since_ms = None;
                rt.was_active = false;
                continue;
            }

            let since = *rt.condition_true_since_ms.get_or_insert(world.now_ms);
            let held_ok = world.now_ms.saturating_sub(since) >= rule.hold_ms;
            let cooldown_ok = match rt.last_fired_ms {
                None => true,
                Some(prev) => world.now_ms.saturating_sub(prev) >= rule.cooldown_ms,
            };

            let edge_ok = !rule.edge_triggered || !rt.was_active;

            if held_ok && cooldown_ok && edge_ok {
                rt.last_fired_ms = Some(world.now_ms);
                rt.was_active = true;
                events.push(MacroTriggerEvent {
                    rule_id: rule.id,
                    rule_name: rule.name.clone(),
                    priority: rule.priority,
                    fired_at_ms: world.now_ms,
                    actions: rule.actions.clone(),
                });
            }
        }

        events
    }
}

fn eval_condition(
    cond: &Condition,
    world: &MacroWorldState,
    rt: &mut RuleRuntimeState,
    node_path: u64,
) -> bool {
    match cond {
        Condition::FocusedSilkWindow {
            frame_id,
            title_contains,
        } => {
            let id_ok = match frame_id {
                Some(fid) => world.focused_window_frame_id == Some(*fid),
                None => true,
            };
            let title_ok = match title_contains {
                Some(sub) => world.focused_window_title.contains(sub.as_str()),
                None => true,
            };
            if frame_id.is_none() && title_contains.is_none() {
                world.focused_window_frame_id.is_some() || !world.focused_window_title.is_empty()
            } else {
                id_ok && title_ok
            }
        }
        Condition::AudioThresholdDb {
            strip_id,
            threshold_db,
            above,
            hysteresis_db,
        } => {
            let level = world
                .strip_levels_db
                .get(strip_id)
                .copied()
                .unwrap_or(-120.0);
            let hyst = hysteresis_db.max(0.0);
            let prev = rt.latches.get(&node_path).copied().unwrap_or(false);
            let next = if *above {
                if prev {
                    level >= (*threshold_db - hyst)
                } else {
                    level >= *threshold_db
                }
            } else if prev {
                level <= (*threshold_db + hyst)
            } else {
                level <= *threshold_db
            };
            rt.latches.insert(node_path, next);
            next
        }
        Condition::MediaRemainingMs {
            source_id,
            threshold_ms,
        } => world
            .media
            .get(source_id)
            .is_some_and(|m| m.playing && m.remaining_ms <= *threshold_ms),
        Condition::TimerIntervalMs { interval_ms } => {
            let interval = (*interval_ms).max(1);
            let next_ms = rt
                .timer_next_ms
                .entry(node_path)
                .or_insert_with(|| world.now_ms.saturating_add(interval));
            if world.now_ms >= *next_ms {
                while *next_ms <= world.now_ms {
                    *next_ms = next_ms.saturating_add(interval);
                }
                true
            } else {
                false
            }
        }
        Condition::UserIdleMs { min_idle_ms } => world.user_idle_ms >= *min_idle_ms,
        Condition::StreamState { expected } => world.stream_state == *expected,
        Condition::CompositeAnd(children) => {
            if children.is_empty() {
                return false;
            }
            let mut all_ok = true;
            for (i, child) in children.iter().enumerate() {
                let child_path = node_path.wrapping_mul(31).wrapping_add(i as u64 + 1);
                if !eval_condition(child, world, rt, child_path) {
                    all_ok = false;
                }
            }
            all_ok
        }
        Condition::CompositeOr(children) => {
            let mut any_ok = false;
            for (i, child) in children.iter().enumerate() {
                let child_path = node_path.wrapping_mul(31).wrapping_add(i as u64 + 1);
                if eval_condition(child, world, rt, child_path) {
                    any_ok = true;
                }
            }
            any_ok
        }
    }
}

/// Pending recording/replay commands emitted by [`execute_macro_actions`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MacroOutputCommand {
    StartRecord,
    StopRecord,
    SaveReplay { seconds: f32 },
}

/// Applies triggered [`MacroAction`]s directly onto Cabaret's core [`StudioMode`],
/// [`SceneCollection`], [`AudioMixer`], and [`DownstreamKeyerBank`], returning any
/// recording/replay commands for the output pipeline.
pub fn execute_macro_actions(
    events: &[MacroTriggerEvent],
    studio_mode: &mut StudioMode,
    collection: &mut SceneCollection,
    mixer: &mut AudioMixer,
    dsk_bank: &mut DownstreamKeyerBank,
) -> Vec<MacroOutputCommand> {
    let mut output_cmds = Vec::new();
    for ev in events {
        for act in &ev.actions {
            match act {
                MacroAction::SwitchStudio {
                    studio_id,
                    preview_only,
                } => {
                    studio_mode.preview_studio = *studio_id;
                    if !*preview_only {
                        studio_mode.take_with_collection(collection);
                        dsk_bank.on_main_transition_triggered();
                    }
                }
                MacroAction::TriggerTake => {
                    studio_mode.take_with_collection(collection);
                    dsk_bank.on_main_transition_triggered();
                }
                MacroAction::SetSourceVisible {
                    studio_id,
                    item_id,
                    visible,
                } => {
                    if let Some(st) = collection.studio_mut(*studio_id) {
                        if let Some(item) = st.items.iter_mut().find(|i| i.id == *item_id) {
                            item.visible = *visible;
                        }
                    }
                }
                MacroAction::SetStripMute { strip_id, muted } => {
                    if let Some(strip) = mixer.strip_mut(*strip_id) {
                        strip.muted = *muted;
                    }
                }
                MacroAction::StartStopRecord { start } => {
                    output_cmds.push(if *start {
                        MacroOutputCommand::StartRecord
                    } else {
                        MacroOutputCommand::StopRecord
                    });
                }
                MacroAction::SaveReplay { seconds } => {
                    output_cmds.push(MacroOutputCommand::SaveReplay { seconds: *seconds });
                }
                MacroAction::SetDskActive { channel_id, active } => {
                    dsk_bank.set_active(*channel_id, *active);
                }
            }
        }
    }
    output_cmds
}
