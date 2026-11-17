// SPDX-License-Identifier: GPL-3.0-only
//! Moan Muse: Opt-In Generative Music Engine, Model-Pack Lifecycle & 4-Tier Supervisor.
//!
//! Provides ACE-Step 1.5 & Muse generative workflows, 4-tier adaptive
//! RAM supervision (from < 2 MiB offline Tier 0 to 7.7 GB local Studio XL), pure `#![no_std]`
//! RIFF WAV/JSON codecs, and sample-accurate stem model.

#![no_std]
extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod chat;
pub mod job;
pub mod lrc;
pub mod pack;
pub mod protocol;
pub mod stems;
pub mod supervisor;
pub mod tier;
pub mod tier0;

pub use chat::{parse_studio_chat, StudioChatCommand};
pub use job::{
    try_alloc_bytes, try_alloc_frames, validate_nonce, MuseError, MuseJobRequest, MuseJobResult,
    MuseMode, Nonce, Status, StemRole, UnderstandResult,
};
pub use lrc::{frame_to_ms, generate_lrc, ms_to_frame, parse_lrc, LrcLine};
pub use pack::{
    parse_manifest, sha256_digest_hex, ManifestEntry, ModelFileInfo, ModelPackDownloader,
    ModelPackKind, ModelPackState, MuseConfig, PackError, STUDIO_XL_FILES, TURBO_LITE_FILES,
};
pub use protocol::{
    build_ace_lm_request, build_ace_props_request, build_ace_synth_request,
    build_ace_understand_request, build_query_result_request, build_release_task_request,
    decode_wav, encode_wav_16bit, encode_wav_float, parse_ace_lm_response,
    parse_ace_props_response, parse_ace_synth_response, parse_ace_understand_response,
    parse_http_response, parse_json, parse_query_result_response, parse_release_task_response,
    serialize_json, AceApiTaskStatus, AceProps, HttpResponse, JsonValue,
};
pub use stems::{TakeAudio, TakeCrate, TakeVersion};
pub use supervisor::MuseSupervisor;
pub use tier::{select_tier, RamTier, TierSelection};
pub use tier0::Tier0Engine;
