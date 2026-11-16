// SPDX-License-Identifier: GPL-3.0-only
//! 4-Tier Hardware RAM Probe & Automatic Selector.
//!
//! Supervises the 4 operational tiers:
//! - Tier 0 Featherweight: < 4096 MB RAM or no pack & no endpoint (< 2 MiB RAM footprint)
//! - Tier 1 Remote Bridge: any RAM + live ace-server (:8080) or acestep-api (:8001) endpoint (0 MB local model RAM)
//! - Tier 2 Local Turbo-Lite: 6144..20480 MB RAM + TurboLiteGguf installed (ephemeral one-shot sidecar, exits immediately)
//! - Tier 3 Local Studio XL: >= 20480 MB RAM + StudioXlGguf installed

use crate::pack::{ModelPackKind, ModelPackState};

/// The four operational generative engine tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamTier {
    /// < 4096 MB RAM or offline without models: pure deterministic `#![no_std]` Rust (< 2 MiB RAM).
    Tier0Featherweight,
    /// Any RAM with healthy remote endpoint (0 MB local model RAM, peak ~8 MB audio buffer).
    Tier1RemoteBridge,
    /// 6144..20480 MB RAM with TurboLiteGguf installed (~2.4 GB GGUF, peak ~3.5 GB ephemeral).
    Tier2LocalTurboLite,
    /// >= 20480 MB RAM with StudioXlGguf installed (~7.7 GB GGUF, peak ~12 GB).
    Tier3LocalStudioXl,
}

impl RamTier {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tier0Featherweight => "Tier 0 Featherweight (Pure Rust)",
            Self::Tier1RemoteBridge => "Tier 1 Remote Bridge",
            Self::Tier2LocalTurboLite => "Tier 2 Local Turbo-Lite (Ephemeral)",
            Self::Tier3LocalStudioXl => "Tier 3 Local Studio XL",
        }
    }
}

/// The result of an adaptive RAM and hardware probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierSelection {
    pub tier: RamTier,
    pub badge_text: &'static str,
    pub estimated_peak_mb: u64,
    pub reason: &'static str,
}

/// Select the optimal generation tier given hardware constraints, pack state, and user preference.
pub fn select_tier(
    available_ram_mb: u64,
    pack: &ModelPackState,
    endpoint_healthy: bool,
    preferred: Option<RamTier>,
) -> TierSelection {
    // 1. Honor preferred tier if prerequisites are satisfied
    if let Some(pref) = preferred {
        match pref {
            RamTier::Tier1RemoteBridge if endpoint_healthy => {
                return TierSelection {
                    tier: RamTier::Tier1RemoteBridge,
                    badge_text: "⚡ Remote ACE Bridge (Preferred)",
                    estimated_peak_mb: 8,
                    reason: "User preferred remote bridge with verified healthy endpoint",
                };
            }
            RamTier::Tier3LocalStudioXl
                if available_ram_mb >= 20480
                    && matches!(pack, ModelPackState::Installed { kind: ModelPackKind::StudioXlGguf, .. }) =>
            {
                return TierSelection {
                    tier: RamTier::Tier3LocalStudioXl,
                    badge_text: "🎛️ Local Studio XL (Preferred)",
                    estimated_peak_mb: 12288,
                    reason: "User preferred local Studio XL on high-RAM workstation",
                };
            }
            RamTier::Tier2LocalTurboLite
                if available_ram_mb >= 6144
                    && matches!(pack, ModelPackState::Installed { .. }) =>
            {
                return TierSelection {
                    tier: RamTier::Tier2LocalTurboLite,
                    badge_text: "🚀 Local Turbo Lite (Preferred)",
                    estimated_peak_mb: 3500,
                    reason: "User preferred local Turbo Lite ephemeral sidecar",
                };
            }
            RamTier::Tier0Featherweight => {
                return TierSelection {
                    tier: RamTier::Tier0Featherweight,
                    badge_text: "🪶 Tier 0 Featherweight (Preferred)",
                    estimated_peak_mb: 2,
                    reason: "User preferred offline pure-Rust engine",
                };
            }
            _ => {
                // Preferred tier cannot be satisfied; fall back to automatic probe selection
            }
        }
    }

    // 2. Automatic hardware probe
    if endpoint_healthy {
        TierSelection {
            tier: RamTier::Tier1RemoteBridge,
            badge_text: "⚡ Remote ACE Bridge",
            estimated_peak_mb: 8,
            reason: "Connected to remote ACE endpoint with zero local model RAM overhead",
        }
    } else if available_ram_mb >= 20480
        && matches!(pack, ModelPackState::Installed { kind: ModelPackKind::StudioXlGguf, .. })
    {
        TierSelection {
            tier: RamTier::Tier3LocalStudioXl,
            badge_text: "🎛️ Local Studio XL",
            estimated_peak_mb: 12288,
            reason: "High-RAM workstation (>=20GB) with complete 7.7GB Studio XL model pack",
        }
    } else if available_ram_mb >= 6144
        && matches!(pack, ModelPackState::Installed { .. })
    {
        TierSelection {
            tier: RamTier::Tier2LocalTurboLite,
            badge_text: "🚀 Local Turbo Lite (Ephemeral)",
            estimated_peak_mb: 3500,
            reason: "Mid-RAM machine running ephemeral one-shot sidecar; exits immediately to free RAM",
        }
    } else {
        TierSelection {
            tier: RamTier::Tier0Featherweight,
            badge_text: "🪶 Tier 0 Featherweight (Offline Pure Rust)",
            estimated_peak_mb: 2,
            reason: "Lightweight offline mode: pure deterministic Rust synthesis with < 2 MiB RAM",
        }
    }
}
