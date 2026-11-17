// SPDX-License-Identifier: GPL-3.0-only
//! Complete `cabaret-proto` control plane for Cabaret:
//! - OBS WebSocket v5 (`ObsRequest`, `ObsMessage`, SHA-256/Base64 `Identify` auth, `RequestBatch`)
//! - OmaTools Headless CLI (`schema`, `status --json`, `list`, `inspect`, `ctl`, `apply`)
//! - `window.obsstudio` Browser Source bridge RPC handler
//! - Elgato Stream Deck HID event dispatcher
//! - Out-of-process `cabaret-ops` atomic script batch executor

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use cabaret_core::outputs::RecordingState;
use cabaret_core::scene::{SourceKind, Studio};
use cabaret_proto::browser_bridge::{BrowserToHostMessage, ControlLevel, HostToBrowserMessage};
use cabaret_proto::cli::{ctl_to_obs_request, parse_args, schema_json, CliCommand};
use cabaret_proto::json::JsonVal;
use cabaret_proto::obs_ws::{
    compute_obs_auth, status_code, ObsFilterInfo, ObsInputInfo, ObsMessage, ObsRequest,
    ObsResponse, ObsResponseData, ObsSceneInfo, ObsSceneItemInfo, OBS_WS_RPC_VERSION,
};
use cabaret_proto::script_rpc::{execute_batch_atomic, ScriptAction, ScriptRpcMessage};
use cabaret_proto::streamdeck::StreamDeckEvent;

use crate::studio::{ActiveTransitionPreset, CabaretStudio};

/// Output returned by executing a headless `cabaret` CLI invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CliOutput {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    pub fn err(stderr: impl Into<String>, exit_code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.into(),
            exit_code,
        }
    }
}

/// Optional OBS WebSocket v5 authentication challenge state for one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObsAuthSession {
    pub password: String,
    pub salt: String,
    pub challenge: String,
    pub identified: bool,
}

impl ObsAuthSession {
    pub fn new(password: impl Into<String>, salt: impl Into<String>, challenge: impl Into<String>) -> Self {
        Self {
            password: password.into(),
            salt: salt.into(),
            challenge: challenge.into(),
            identified: false,
        }
    }

    pub fn hello_message(&self) -> ObsMessage {
        ObsMessage::Hello {
            obs_web_socket_version: String::from("5.5.0-cabaret"),
            rpc_version: OBS_WS_RPC_VERSION,
            authentication: if self.password.is_empty() {
                None
            } else {
                Some(cabaret_proto::obs_ws::ObsAuthChallenge {
                    challenge: self.challenge.clone(),
                    salt: self.salt.clone(),
                })
            },
        }
    }

    pub fn verify_identify(&mut self, authentication: Option<&str>) -> bool {
        if self.password.is_empty() {
            self.identified = true;
            return true;
        }
        let expected = compute_obs_auth(&self.password, &self.salt, &self.challenge);
        let ok = authentication == Some(expected.as_str());
        if ok {
            self.identified = true;
        }
        ok
    }
}

fn format_timecode_ms(ms: u64) -> String {
    let millis = ms % 1000;
    let total_s = ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}.{millis:03}")
}

fn db_to_mul(db: f32) -> f64 {
    if db <= -100.0 {
        0.0
    } else {
        libm::pow(10.0, db as f64 / 20.0)
    }
}

fn mul_to_db(mul: f64) -> f32 {
    if mul <= 1e-5 {
        -100.0
    } else {
        (20.0 * libm::log10(mul)) as f32
    }
}

fn find_studio_idx(studio: &CabaretStudio, name: &str) -> Option<usize> {
    studio
        .collection
        .studios
        .iter()
        .position(|s| s.name == name)
        .or_else(|| {
            studio
                .collection
                .studios
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name))
        })
}

fn find_strip_idx(studio: &CabaretStudio, name: &str) -> Option<usize> {
    studio
        .audio
        .strips
        .iter()
        .position(|s| s.name == name)
        .or_else(|| {
            studio
                .audio
                .strips
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name))
        })
}

fn source_kind_tag(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::Display { .. } => "sexos_display",
        SourceKind::SilkWindow { .. } => "sexos_silk_window",
        SourceKind::DisplayRegion { .. } => "sexos_display_region",
        SourceKind::UvcCamera { .. } => "sexos_uvc_camera",
        SourceKind::PhoneStream { .. } => "phone_srt_camera",
        SourceKind::MediaFile { .. } => "ffmpeg_source",
        SourceKind::Browser { .. } => "browser_source",
        SourceKind::PresentationDeck { .. } => "burlesque_deck",
        SourceKind::ImageSlideshow { .. } => "slideshow",
        SourceKind::RichText { .. } => "text_ft2_source",
        SourceKind::ColorSolid { .. } => "color_source",
        SourceKind::NestedStudio { .. } => "nested_scene",
        SourceKind::Group { .. } => "group",
    }
}

/// Dispatches a typed OBS WebSocket v5 [`ObsRequest`] directly against `studio`.
pub fn dispatch_obs_request(studio: &mut CabaretStudio, req: &ObsRequest) -> ObsResponse {
    let rtype = req.request_type();
    match req {
        ObsRequest::GetVersion => ObsResponse::ok(
            rtype,
            ObsResponseData::Version {
                obs_version: String::from("Cabaret 0.1.0 (SexOS OBS-Parity)"),
                obs_web_socket_version: String::from("5.5.0"),
                rpc_version: OBS_WS_RPC_VERSION,
                available_requests: vec![
                    String::from("GetVersion"),
                    String::from("GetSceneList"),
                    String::from("SetCurrentProgramScene"),
                    String::from("SetCurrentPreviewScene"),
                    String::from("GetStudioModeEnabled"),
                    String::from("SetStudioModeEnabled"),
                    String::from("TriggerStudioModeTransition"),
                    String::from("GetInputList"),
                    String::from("GetInputMute"),
                    String::from("SetInputMute"),
                    String::from("ToggleInputMute"),
                    String::from("GetInputVolume"),
                    String::from("SetInputVolume"),
                    String::from("GetSceneItemList"),
                    String::from("SetSceneItemEnabled"),
                    String::from("SetSceneItemTransform"),
                    String::from("GetStreamStatus"),
                    String::from("StartStream"),
                    String::from("StopStream"),
                    String::from("ToggleStream"),
                    String::from("GetRecordStatus"),
                    String::from("StartRecord"),
                    String::from("StopRecord"),
                    String::from("ToggleRecordPause"),
                    String::from("SaveReplayBuffer"),
                    String::from("GetVirtualCamStatus"),
                    String::from("ToggleVirtualCam"),
                    String::from("GetSourceFilterList"),
                    String::from("SetSourceFilterEnabled"),
                ],
                supported_image_formats: vec![String::from("png"), String::from("rgba")],
                platform: String::from("sexos"),
                platform_description: String::from("SexOS x86_64 Broadcast Compositor"),
            },
        ),
        ObsRequest::GetSceneList => {
            let scenes = studio
                .collection
                .studios
                .iter()
                .enumerate()
                .map(|(i, s)| ObsSceneInfo {
                    scene_name: s.name.clone(),
                    scene_index: i as u32,
                })
                .collect();
            ObsResponse::ok(
                rtype,
                ObsResponseData::SceneList {
                    current_program_scene_name: Some(
                        studio
                            .studio_name(studio.studio_mode.program_studio)
                            .to_string(),
                    ),
                    current_preview_scene_name: if studio.studio_mode.enabled {
                        Some(
                            studio
                                .studio_name(studio.studio_mode.preview_studio)
                                .to_string(),
                        )
                    } else {
                        None
                    },
                    scenes,
                },
            )
        }
        ObsRequest::SetCurrentProgramScene { scene_name } => {
            if let Some(id) = studio.find_studio_by_name(scene_name) {
                studio.set_program_studio(id);
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Studio '{scene_name}' not found"),
                )
            }
        }
        ObsRequest::SetCurrentPreviewScene { scene_name } => {
            if let Some(id) = studio.find_studio_by_name(scene_name) {
                studio.set_preview_studio(id);
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Studio '{scene_name}' not found"),
                )
            }
        }
        ObsRequest::GetStudioModeEnabled => ObsResponse::ok(
            rtype,
            ObsResponseData::StudioModeEnabled {
                studio_mode_enabled: studio.studio_mode.enabled,
            },
        ),
        ObsRequest::SetStudioModeEnabled {
            studio_mode_enabled,
        } => {
            studio.studio_mode.enabled = *studio_mode_enabled;
            ObsResponse::ok(rtype, ObsResponseData::Empty)
        }
        ObsRequest::TriggerStudioModeTransition => {
            studio.trigger_take(true);
            ObsResponse::ok(rtype, ObsResponseData::Empty)
        }
        ObsRequest::GetInputList { input_kind } => {
            let mut inputs = Vec::new();
            for strip in &studio.audio.strips {
                let kind = "audio_strip";
                if input_kind.as_deref().is_none_or(|k| k == kind) {
                    inputs.push(ObsInputInfo {
                        input_name: strip.name.clone(),
                        input_kind: String::from(kind),
                        unversioned_input_kind: String::from(kind),
                    });
                }
            }
            for src in studio.collection.sources.values() {
                let kind = source_kind_tag(&src.kind);
                if input_kind.as_deref().is_none_or(|k| k == kind) {
                    inputs.push(ObsInputInfo {
                        input_name: src.name.clone(),
                        input_kind: String::from(kind),
                        unversioned_input_kind: String::from(kind),
                    });
                }
            }
            ObsResponse::ok(rtype, ObsResponseData::InputList { inputs })
        }
        ObsRequest::GetInputMute { input_name } => {
            if let Some(idx) = find_strip_idx(studio, input_name) {
                ObsResponse::ok(
                    rtype,
                    ObsResponseData::InputMute {
                        input_muted: studio.audio.strips[idx].muted,
                    },
                )
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Input strip '{input_name}' not found"),
                )
            }
        }
        ObsRequest::SetInputMute {
            input_name,
            input_muted,
        } => {
            if let Some(idx) = find_strip_idx(studio, input_name) {
                studio.audio.strips[idx].muted = *input_muted;
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Input strip '{input_name}' not found"),
                )
            }
        }
        ObsRequest::ToggleInputMute { input_name } => {
            if let Some(idx) = find_strip_idx(studio, input_name) {
                studio.audio.strips[idx].muted = !studio.audio.strips[idx].muted;
                ObsResponse::ok(
                    rtype,
                    ObsResponseData::InputMute {
                        input_muted: studio.audio.strips[idx].muted,
                    },
                )
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Input strip '{input_name}' not found"),
                )
            }
        }
        ObsRequest::GetInputVolume { input_name } => {
            if let Some(idx) = find_strip_idx(studio, input_name) {
                let db = studio.audio.strips[idx].fader_db;
                ObsResponse::ok(
                    rtype,
                    ObsResponseData::InputVolume {
                        input_volume_mul: db_to_mul(db),
                        input_volume_db: db as f64,
                    },
                )
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Input strip '{input_name}' not found"),
                )
            }
        }
        ObsRequest::SetInputVolume {
            input_name,
            input_volume_mul,
            input_volume_db,
        } => {
            if let Some(idx) = find_strip_idx(studio, input_name) {
                if let Some(db) = input_volume_db {
                    studio.audio.strips[idx].fader_db = (*db as f32).clamp(-100.0, 20.0);
                } else if let Some(mul) = input_volume_mul {
                    studio.audio.strips[idx].fader_db = mul_to_db(*mul).clamp(-100.0, 20.0);
                } else {
                    return ObsResponse::err(
                        rtype,
                        status_code::MISSING_REQUEST_FIELD,
                        "Expected inputVolumeDb or inputVolumeMul",
                    );
                }
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Input strip '{input_name}' not found"),
                )
            }
        }
        ObsRequest::GetSceneItemList { scene_name } => {
            if let Some(sidx) = find_studio_idx(studio, scene_name) {
                let scene_items = studio.collection.studios[sidx]
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let is_group = studio
                            .collection
                            .sources
                            .get(&item.source_id)
                            .map(|s| matches!(s.kind, SourceKind::Group { .. }));
                        ObsSceneItemInfo {
                            scene_item_id: item.id.0 as i64,
                            scene_item_index: i as u32,
                            source_name: item.name.clone(),
                            scene_item_enabled: item.visible,
                            is_group,
                        }
                    })
                    .collect();
                ObsResponse::ok(rtype, ObsResponseData::SceneItemList { scene_items })
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Studio '{scene_name}' not found"),
                )
            }
        }
        ObsRequest::SetSceneItemEnabled {
            scene_name,
            scene_item_id,
            scene_item_enabled,
        } => {
            if let Some(sidx) = find_studio_idx(studio, scene_name) {
                if let Some(item) = studio.collection.studios[sidx]
                    .items
                    .iter_mut()
                    .find(|i| i.id.0 as i64 == *scene_item_id)
                {
                    item.visible = *scene_item_enabled;
                    ObsResponse::ok(rtype, ObsResponseData::Empty)
                } else {
                    ObsResponse::err(
                        rtype,
                        status_code::RESOURCE_NOT_FOUND,
                        format!("Scene item {scene_item_id} not found in '{scene_name}'"),
                    )
                }
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Studio '{scene_name}' not found"),
                )
            }
        }
        ObsRequest::SetSceneItemTransform {
            scene_name,
            scene_item_id,
            scene_item_transform,
        } => {
            if let Some(sidx) = find_studio_idx(studio, scene_name) {
                if let Some(item) = studio.collection.studios[sidx]
                    .items
                    .iter_mut()
                    .find(|i| i.id.0 as i64 == *scene_item_id)
                {
                    if let Some(px) = scene_item_transform.position_x {
                        item.transform.pos[0] = px as f32;
                    }
                    if let Some(py) = scene_item_transform.position_y {
                        item.transform.pos[1] = py as f32;
                    }
                    if let Some(sx) = scene_item_transform.scale_x {
                        item.transform.scale[0] = sx as f32;
                    }
                    if let Some(sy) = scene_item_transform.scale_y {
                        item.transform.scale[1] = sy as f32;
                    }
                    if let Some(rot) = scene_item_transform.rotation {
                        item.transform.rot_deg = rot as f32;
                    }
                    ObsResponse::ok(rtype, ObsResponseData::Empty)
                } else {
                    ObsResponse::err(
                        rtype,
                        status_code::RESOURCE_NOT_FOUND,
                        format!("Scene item {scene_item_id} not found"),
                    )
                }
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Studio '{scene_name}' not found"),
                )
            }
        }
        ObsRequest::GetStreamStatus => ObsResponse::ok(
            rtype,
            ObsResponseData::StreamStatus {
                output_active: studio.stream_active,
                output_reconnecting: studio.stream_reconnecting,
                output_timecode: format_timecode_ms(studio.stream_duration_ms),
                output_duration: studio.stream_duration_ms,
                output_congestion: 0.02,
                output_bytes: studio.stream_bytes_sent,
                output_skipped_frames: studio.stream_frames_dropped,
                output_total_frames: studio.stream_frames_sent,
            },
        ),
        ObsRequest::StartStream => {
            if studio.stream_active {
                ObsResponse::err(rtype, status_code::OUTPUT_RUNNING, "Stream already active")
            } else {
                studio.start_streaming();
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            }
        }
        ObsRequest::StopStream => {
            if !studio.stream_active {
                ObsResponse::err(rtype, status_code::OUTPUT_NOT_RUNNING, "Stream is not running")
            } else {
                studio.stop_streaming();
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            }
        }
        ObsRequest::ToggleStream => {
            if studio.stream_active {
                studio.stop_streaming();
            } else {
                studio.start_streaming();
            }
            ObsResponse::ok(
                rtype,
                ObsResponseData::OutputActive {
                    output_active: studio.stream_active,
                },
            )
        }
        ObsRequest::GetRecordStatus => {
            let active = matches!(
                studio.recording.state,
                RecordingState::Recording | RecordingState::Paused
            );
            let paused = studio.recording.state == RecordingState::Paused;
            let dur_ms = (studio.recording.last_active_pts_us.max(0) / 1000) as u64;
            ObsResponse::ok(
                rtype,
                ObsResponseData::RecordStatus {
                    output_active: active,
                    output_paused: paused,
                    output_timecode: format_timecode_ms(dur_ms),
                    output_duration: dur_ms,
                    output_bytes: studio.recording.segment_bytes,
                },
            )
        }
        ObsRequest::StartRecord => {
            if matches!(
                studio.recording.state,
                RecordingState::Recording | RecordingState::Paused
            ) {
                ObsResponse::err(rtype, status_code::OUTPUT_RUNNING, "Recording already active")
            } else {
                studio.start_recording();
                ObsResponse::ok(rtype, ObsResponseData::Empty)
            }
        }
        ObsRequest::StopRecord => {
            if !matches!(
                studio.recording.state,
                RecordingState::Recording | RecordingState::Paused
            ) {
                ObsResponse::err(
                    rtype,
                    status_code::OUTPUT_NOT_RUNNING,
                    "Recording is not running",
                )
            } else {
                let output_path = studio.stop_recording();
                ObsResponse::ok(rtype, ObsResponseData::StopRecord { output_path })
            }
        }
        ObsRequest::ToggleRecordPause => {
            studio.toggle_record_pause();
            ObsResponse::ok(rtype, ObsResponseData::Empty)
        }
        ObsRequest::SaveReplayBuffer => match studio.save_replay(15.0) {
            Ok(_) => ObsResponse::ok(rtype, ObsResponseData::Empty),
            Err(e) => ObsResponse::err(rtype, status_code::GENERIC_ERROR, format!("{e:?}")),
        },
        ObsRequest::GetVirtualCamStatus => ObsResponse::ok(
            rtype,
            ObsResponseData::OutputActive {
                output_active: studio.virtual_cam.active,
            },
        ),
        ObsRequest::ToggleVirtualCam => {
            let active = studio.toggle_virtual_cam();
            ObsResponse::ok(
                rtype,
                ObsResponseData::OutputActive {
                    output_active: active,
                },
            )
        }
        ObsRequest::GetSourceFilterList { source_name } => {
            if let Some(src) = studio
                .collection
                .sources
                .values()
                .find(|s| s.name == *source_name)
            {
                let filters = src
                    .filters
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, f)| ObsFilterInfo {
                        filter_name: f.name.clone(),
                        filter_kind: format!("{:?}", f.filter),
                        filter_index: i as u32,
                        filter_enabled: f.enabled,
                        filter_settings: None,
                    })
                    .collect();
                ObsResponse::ok(rtype, ObsResponseData::SourceFilterList { filters })
            } else if let Some(sidx) = find_strip_idx(studio, source_name) {
                let filters = studio.audio.strips[sidx]
                    .filters
                    .iter()
                    .enumerate()
                    .map(|(i, f)| ObsFilterInfo {
                        filter_name: f.name.clone(),
                        filter_kind: format!("{:?}", f.filter),
                        filter_index: i as u32,
                        filter_enabled: f.enabled,
                        filter_settings: None,
                    })
                    .collect();
                ObsResponse::ok(rtype, ObsResponseData::SourceFilterList { filters })
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Source '{source_name}' not found"),
                )
            }
        }
        ObsRequest::SetSourceFilterEnabled {
            source_name,
            filter_name,
            filter_enabled,
        } => {
            if let Some(src) = studio
                .collection
                .sources
                .values_mut()
                .find(|s| s.name == *source_name)
            {
                if let Some(f) = src
                    .filters
                    .entries
                    .iter_mut()
                    .find(|e| e.name == *filter_name)
                {
                    f.enabled = *filter_enabled;
                    ObsResponse::ok(rtype, ObsResponseData::Empty)
                } else {
                    ObsResponse::err(
                        rtype,
                        status_code::RESOURCE_NOT_FOUND,
                        format!("Filter '{filter_name}' not found on '{source_name}'"),
                    )
                }
            } else if let Some(sidx) = find_strip_idx(studio, source_name) {
                if let Some(f) = studio.audio.strips[sidx]
                    .filters
                    .iter_mut()
                    .find(|e| e.name == *filter_name)
                {
                    f.enabled = *filter_enabled;
                    ObsResponse::ok(rtype, ObsResponseData::Empty)
                } else {
                    ObsResponse::err(
                        rtype,
                        status_code::RESOURCE_NOT_FOUND,
                        format!("Audio filter '{filter_name}' not found on '{source_name}'"),
                    )
                }
            } else {
                ObsResponse::err(
                    rtype,
                    status_code::RESOURCE_NOT_FOUND,
                    format!("Source '{source_name}' not found"),
                )
            }
        }
        ObsRequest::Custom { request_type, .. } => ObsResponse::err(
            request_type.as_str(),
            status_code::UNKNOWN_REQUEST_TYPE,
            format!("Unsupported custom request '{request_type}'"),
        ),
    }
}

/// Dispatches a top-level [`ObsMessage`] (Identify auth, single Request, RequestBatch).
pub fn dispatch_obs_message(
    studio: &mut CabaretStudio,
    auth: &mut ObsAuthSession,
    msg: &ObsMessage,
) -> Option<ObsMessage> {
    match msg {
        ObsMessage::Identify {
            rpc_version,
            authentication,
            ..
        } => {
            if auth.verify_identify(authentication.as_deref()) {
                Some(ObsMessage::Identified {
                    negotiated_rpc_version: (*rpc_version).min(OBS_WS_RPC_VERSION),
                })
            } else {
                None
            }
        }
        ObsMessage::Reidentify { .. } => Some(ObsMessage::Identified {
            negotiated_rpc_version: OBS_WS_RPC_VERSION,
        }),
        ObsMessage::Request {
            request_id,
            request,
        } => {
            if !auth.identified {
                return Some(ObsMessage::RequestResponse {
                    request_id: request_id.clone(),
                    response: ObsResponse::err(
                        request.request_type(),
                        status_code::GENERIC_ERROR,
                        "Connection not identified",
                    ),
                });
            }
            Some(ObsMessage::RequestResponse {
                request_id: request_id.clone(),
                response: dispatch_obs_request(studio, request),
            })
        }
        ObsMessage::RequestBatch {
            request_id,
            halt_on_failure,
            requests,
            ..
        } => {
            if !auth.identified {
                return Some(ObsMessage::RequestBatchResponse {
                    request_id: request_id.clone(),
                    results: vec![ObsResponse::err(
                        "RequestBatch",
                        status_code::GENERIC_ERROR,
                        "Connection not identified",
                    )],
                });
            }
            let halt = halt_on_failure.unwrap_or(false);
            let mut results = Vec::with_capacity(requests.len());
            for item in requests {
                let res = dispatch_obs_request(studio, &item.request);
                let ok = res.status.result;
                results.push(res);
                if halt && !ok {
                    break;
                }
            }
            Some(ObsMessage::RequestBatchResponse {
                request_id: request_id.clone(),
                results,
            })
        }
        _ => None,
    }
}

/// Handles a Browser Source `window.obsstudio` bridge frame (`cabaret-browser:...`).
pub fn dispatch_browser_message(
    studio: &mut CabaretStudio,
    raw_frame: &str,
) -> Option<String> {
    let msg = BrowserToHostMessage::parse_frame(raw_frame).ok()?;
    let reply = match msg {
        BrowserToHostMessage::Ready { plugin_version } => {
            studio
                .browser_events_log
                .push(format!("Browser ready: {plugin_version}"));
            return None;
        }
        BrowserToHostMessage::GetControlLevel { call_id } => HostToBrowserMessage::Response {
            call_id,
            payload: JsonVal::Num(ControlLevel::Advanced.as_u8() as f64),
        },
        BrowserToHostMessage::GetCurrentScene { call_id } => {
            let sid = studio.studio_mode.program_studio;
            let name = studio.studio_name(sid);
            HostToBrowserMessage::Response {
                call_id,
                payload: JsonVal::obj()
                    .with("name", name)
                    .with("width", studio.profile.base_width)
                    .with("height", studio.profile.base_height),
            }
        }
        BrowserToHostMessage::GetStatus { call_id } => HostToBrowserMessage::Response {
            call_id,
            payload: JsonVal::obj()
                .with("streaming", studio.stream_active)
                .with(
                    "recording",
                    studio.recording.state == RecordingState::Recording,
                )
                .with("recordingPaused", studio.recording.state == RecordingState::Paused)
                .with("replaybuffer", !studio.replay_buffer.is_empty())
                .with("virtualcam", studio.virtual_cam.active),
        },
        BrowserToHostMessage::SaveReplayBuffer => {
            let _ = studio.save_replay(15.0);
            studio
                .browser_events_log
                .push(String::from("Browser saved replay buffer"));
            return None;
        }
    };
    Some(reply.format_frame())
}

/// Maps hardware Elgato Stream Deck key & dial events onto Cabaret studio actions.
pub fn dispatch_streamdeck_event(
    studio: &mut CabaretStudio,
    ev: StreamDeckEvent,
) -> Option<String> {
    match ev {
        StreamDeckEvent::KeyDown(0..=3) => {
            let idx = match ev {
                StreamDeckEvent::KeyDown(k) => k as usize,
                _ => 0,
            };
            if let Some(st) = studio.collection.studios.get(idx) {
                let id = st.id;
                let name = st.name.clone();
                studio.set_preview_studio(id);
                return Some(format!("Preview -> {name}"));
            }
            None
        }
        StreamDeckEvent::KeyDown(4) => {
            let pgm = studio.trigger_take(true);
            Some(format!("TAKE -> {}", studio.studio_name(pgm)))
        }
        StreamDeckEvent::KeyDown(5) => {
            if studio.recording.state == RecordingState::Recording {
                studio.stop_recording();
                Some(String::from("Record Stopped"))
            } else {
                studio.start_recording();
                Some(String::from("Record Started"))
            }
        }
        StreamDeckEvent::KeyDown(6) => {
            let _ = studio.save_replay(15.0);
            Some(String::from("Saved 15s Replay"))
        }
        StreamDeckEvent::KeyDown(7) => {
            let strip_id = studio.strips.mic_serenade;
            if let Some(strip) = studio.audio.strip_mut(strip_id) {
                strip.muted = !strip.muted;
                return Some(format!(
                    "Mic {}",
                    if strip.muted { "Muted" } else { "Unmuted" }
                ));
            }
            None
        }
        StreamDeckEvent::DialRotate { dial: 0, delta } => {
            let strip_id = studio.strips.mic_serenade;
            if let Some(strip) = studio.audio.strip_mut(strip_id) {
                strip.fader_db = (strip.fader_db + delta as f32 * 1.5).clamp(-60.0, 12.0);
                return Some(format!("Mic Fader {:.1} dB", strip.fader_db));
            }
            None
        }
        _ => None,
    }
}

/// Executes an out-of-process `cabaret-ops` script batch atomically with snapshot rollback.
pub fn dispatch_script_batch(
    studio: &mut CabaretStudio,
    batch_id: &str,
    atomic: bool,
    actions: &[ScriptAction],
) -> ScriptRpcMessage {
    execute_batch_atomic(studio, batch_id, atomic, actions, |st, act| match act {
        ScriptAction::Obs(req) => {
            let resp = dispatch_obs_request(st, req);
            if resp.status.result {
                Ok(())
            } else {
                Err(resp
                    .status
                    .comment
                    .unwrap_or_else(|| String::from("OBS request failed")))
            }
        }
        ScriptAction::SetProperty {
            script_id,
            name,
            value,
        } => {
            st.log_event(&format!(
                "script[{script_id}] prop {name}={}",
                value.to_json()
            ));
            Ok(())
        }
        ScriptAction::Log { level, message } => {
            st.log_event(&format!("[{level}] {message}"));
            Ok(())
        }
    })
}

/// Runs the OmaTools headless CLI (`cabaret schema|status|list|inspect|ctl|apply`).
pub fn run_cli_args(studio: &mut CabaretStudio, args: &[&str]) -> CliOutput {
    let cmd = match parse_args(args.iter().copied()) {
        Ok(c) => c,
        Err(e) => return CliOutput::err(format!("{e}"), 2),
    };

    match cmd {
        CliCommand::Schema => CliOutput::ok(schema_json()),
        CliCommand::Status { json } => {
            let st_json = build_status_json(studio);
            if json {
                CliOutput::ok(st_json.to_json())
            } else {
                let pgm = studio.studio_name(studio.studio_mode.program_studio);
                let prv = studio.studio_name(studio.studio_mode.preview_studio);
                CliOutput::ok(format!(
                    "Cabaret Program: {pgm} | Preview: {prv} | Stream: {} | Rec: {:?} | VCam: {}",
                    if studio.stream_active { "LIVE" } else { "OFF" },
                    studio.recording.state,
                    if studio.virtual_cam.active { "ON" } else { "OFF" }
                ))
            }
        }
        CliCommand::List { target } => {
            let tgt = target.as_deref().unwrap_or("scenes");
            let val = match tgt {
                "scenes" | "studios" => {
                    let arr: Vec<JsonVal> = studio
                        .collection
                        .studios
                        .iter()
                        .enumerate()
                        .map(|(idx, s)| {
                            JsonVal::obj()
                                .with("index", idx as u64)
                                .with("id", s.id.0)
                                .with("name", s.name.as_str())
                                .with("items", s.items.len() as u64)
                        })
                        .collect();
                    JsonVal::obj().with("scenes", JsonVal::Arr(arr))
                }
                "inputs" => {
                    let strips: Vec<JsonVal> = studio
                        .audio
                        .strips
                        .iter()
                        .map(|s| {
                            JsonVal::obj()
                                .with("id", s.id.0)
                                .with("name", s.name.as_str())
                                .with("muted", s.muted)
                                .with("faderDb", s.fader_db as f64)
                        })
                        .collect();
                    let sources: Vec<JsonVal> = studio
                        .collection
                        .sources
                        .values()
                        .map(|s| {
                            JsonVal::obj()
                                .with("id", s.id.0)
                                .with("name", s.name.as_str())
                                .with("kind", source_kind_tag(&s.kind))
                        })
                        .collect();
                    JsonVal::obj()
                        .with("audioStrips", JsonVal::Arr(strips))
                        .with("sources", JsonVal::Arr(sources))
                }
                "devices" => {
                    let decks: Vec<JsonVal> = studio
                        .attached_streamdecks
                        .iter()
                        .map(|d| JsonVal::Str(format!("{d:?}")))
                        .collect();
                    JsonVal::obj()
                        .with("streamDecks", JsonVal::Arr(decks))
                        .with("virtualCam", studio.virtual_cam.device_name.as_str())
                }
                other => {
                    return CliOutput::err(
                        format!("Unknown list target '{other}' (expected scenes|inputs|devices)"),
                        2,
                    )
                }
            };
            CliOutput::ok(val.to_json())
        }
        CliCommand::Inspect { target } => {
            let name = target
                .as_deref()
                .unwrap_or_else(|| studio.studio_name(studio.studio_mode.program_studio));
            if let Some(st) = studio
                .collection
                .studios
                .iter()
                .find(|s| s.name == name || s.name.eq_ignore_ascii_case(name))
            {
                return CliOutput::ok(studio_summary_json(st).to_json());
            }
            if let Some(strip_idx) = find_strip_idx(studio, name) {
                let s = &studio.audio.strips[strip_idx];
                let obj = JsonVal::obj()
                    .with("name", s.name.as_str())
                    .with("faderDb", s.fader_db as f64)
                    .with("muted", s.muted)
                    .with("truePeakDbtp", s.meter.last_meters.true_peak_dbtp as f64)
                    .with("integratedLufs", s.meter.last_meters.integrated_lufs as f64);
                return CliOutput::ok(obj.to_json());
            }
            CliOutput::err(format!("Target '{name}' not found"), 1)
        }
        CliCommand::Ctl { action, args } => match ctl_to_obs_request(&action, &args) {
            Ok(req) => {
                let resp = dispatch_obs_request(studio, &req);
                let out = JsonVal::obj()
                    .with("requestType", resp.request_type.as_str())
                    .with("ok", resp.status.result)
                    .with("code", resp.status.code as u64)
                    .with_opt("comment", resp.status.comment.as_deref())
                    .with_opt("data", resp.data.to_json_val());
                CliOutput::ok(out.to_json())
            }
            Err(e) => CliOutput::err(format!("{e}"), 2),
        },
        CliCommand::Apply { payload } => apply_cli_payload(studio, &payload),
    }
}

fn apply_cli_payload(studio: &mut CabaretStudio, payload: &JsonVal) -> CliOutput {
    if let Some(scene) = payload.get_str("scene") {
        let resp = dispatch_obs_request(
            studio,
            &ObsRequest::SetCurrentProgramScene {
                scene_name: String::from(scene),
            },
        );
        if !resp.status.result {
            return CliOutput::err(
                resp.status.comment.unwrap_or_else(|| String::from("failed")),
                1,
            );
        }
    }
    if let Some(preview) = payload.get_str("previewScene") {
        let resp = dispatch_obs_request(
            studio,
            &ObsRequest::SetCurrentPreviewScene {
                scene_name: String::from(preview),
            },
        );
        if !resp.status.result {
            return CliOutput::err(
                resp.status.comment.unwrap_or_else(|| String::from("failed")),
                1,
            );
        }
    }
    if let Some(tr) = payload.get_str("transition") {
        match tr {
            "Cut" | "cut" => studio.select_transition_preset(ActiveTransitionPreset::Cut),
            "Fade" | "fade" => studio.select_transition_preset(ActiveTransitionPreset::Fade),
            "Wipe29" | "wipe29" => studio.select_transition_preset(ActiveTransitionPreset::Wipe29),
            "Stinger" | "stinger" => studio.select_transition_preset(ActiveTransitionPreset::Stinger),
            "Move" | "move" => studio.select_transition_preset(ActiveTransitionPreset::Move),
            other => return CliOutput::err(format!("Unknown transition '{other}'"), 2),
        }
    }
    if payload.get_bool("take") == Some(true) {
        studio.trigger_take(true);
    }
    if let Some(reqs) = payload.get_arr("requests") {
        let mut parsed = Vec::with_capacity(reqs.len());
        for r in reqs {
            let Some(rt) = r.get_str("requestType") else {
                return CliOutput::err("Batch request missing requestType", 2);
            };
            match ObsRequest::from_parts(rt, r.get("requestData")) {
                Ok(req) => parsed.push(ScriptAction::Obs(req)),
                Err(e) => return CliOutput::err(format!("{e}"), 2),
            }
        }
        let batch_res = dispatch_script_batch(studio, "cli-apply", true, &parsed);
        return CliOutput::ok(batch_res.to_json_val().to_json());
    }
    CliOutput::ok(build_status_json(studio).to_json())
}

fn studio_summary_json(st: &Studio) -> JsonVal {
    let items: Vec<JsonVal> = st
        .items
        .iter()
        .map(|it| {
            JsonVal::obj()
                .with("id", it.id.0)
                .with("sourceId", it.source_id.0)
                .with("name", it.name.as_str())
                .with("visible", it.visible)
        })
        .collect();
    JsonVal::obj()
        .with("id", st.id.0)
        .with("name", st.name.as_str())
        .with("items", JsonVal::Arr(items))
}

/// Builds complete machine-readable status JSON without exposing any raw Shibari secret strings.
pub fn build_status_json(studio: &CabaretStudio) -> JsonVal {
    let (slide_idx, total_slides, step_idx, total_steps) = studio.presentation.state();
    let encoder_groups = studio.encoder_groups().len() as u64;
    let secret_slots: Vec<JsonVal> = studio
        .active_secrets
        .iter()
        .map(|(slot, _)| JsonVal::Str(format!("{slot}")))
        .collect();

    JsonVal::obj()
        .with("app", "cabaret")
        .with("programScene", studio.studio_name(studio.studio_mode.program_studio))
        .with("previewScene", studio.studio_name(studio.studio_mode.preview_studio))
        .with("studioModeEnabled", studio.studio_mode.enabled)
        .with("transition", studio.transition_preset.label())
        .with("streamingActive", studio.stream_active)
        .with("recordingState", format!("{:?}", studio.recording.state))
        .with("recordingPath", studio.last_recording_path.as_str())
        .with("virtualCamActive", studio.virtual_cam.active)
        .with("virtualCamName", studio.virtual_cam.device_name.as_str())
        .with("encoderGroups", encoder_groups)
        .with("shibariSlots", JsonVal::Arr(secret_slots))
        .with("slideIndex", slide_idx as u64)
        .with("slideCount", total_slides as u64)
        .with("buildStep", step_idx as u64)
        .with("buildSteps", total_steps as u64)
        .with("hideSelfFromCapture", studio.capture_exclusion.cabaret_ui_hidden)
}
