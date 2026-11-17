//! OBS WebSocket v5 Protocol (Port `4455`, RPC v1).
//!
//! Provides SHA-256 + Base64 challenge/response authentication, OpCodes,
//! wire message framing, and typed requests, responses, and events compatible
//! with Elgato Stream Deck, Bitfocus Companion, and Touch Portal.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use sha2::{Digest, Sha256};

use crate::json::{JsonError, JsonVal};

/// Default TCP port for OBS WebSocket v5.
pub const OBS_WS_DEFAULT_PORT: u16 = 4455;
/// OBS WebSocket v5 RPC protocol version.
pub const OBS_WS_RPC_VERSION: u32 = 1;

/// Standard OBS WebSocket v5 request status codes.
pub mod status_code {
    pub const SUCCESS: u32 = 100;
    pub const MISSING_REQUEST_TYPE: u32 = 203;
    pub const UNKNOWN_REQUEST_TYPE: u32 = 204;
    pub const GENERIC_ERROR: u32 = 205;
    pub const MISSING_REQUEST_FIELD: u32 = 300;
    pub const INVALID_REQUEST_FIELD: u32 = 400;
    pub const RESOURCE_NOT_FOUND: u32 = 600;
    pub const OUTPUT_RUNNING: u32 = 500;
    pub const OUTPUT_NOT_RUNNING: u32 = 501;
}

/// OBS WebSocket v5 event subscription bitmask flags.
pub mod event_subscription {
    pub const NONE: u32 = 0;
    pub const GENERAL: u32 = 1 << 0;
    pub const CONFIG: u32 = 1 << 1;
    pub const SCENES: u32 = 1 << 2;
    pub const INPUTS: u32 = 1 << 3;
    pub const TRANSITIONS: u32 = 1 << 4;
    pub const FILTERS: u32 = 1 << 5;
    pub const OUTPUTS: u32 = 1 << 6;
    pub const SCENE_ITEMS: u32 = 1 << 7;
    pub const MEDIA_INPUTS: u32 = 1 << 8;
    pub const VENDORS: u32 = 1 << 9;
    pub const UI: u32 = 1 << 10;
    pub const ALL: u32 = GENERAL
        | CONFIG
        | SCENES
        | INPUTS
        | TRANSITIONS
        | FILTERS
        | OUTPUTS
        | SCENE_ITEMS
        | MEDIA_INPUTS
        | VENDORS
        | UI;
}

/// Error returned when encoding or decoding OBS WebSocket v5 messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsWsError {
    Json(JsonError),
    MissingField(&'static str),
    InvalidField(&'static str),
    UnknownOpCode(u64),
}

impl fmt::Display for ObsWsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::MissingField(field) => write!(f, "missing OBS WebSocket field '{field}'"),
            Self::InvalidField(field) => write!(f, "invalid OBS WebSocket field '{field}'"),
            Self::UnknownOpCode(op) => write!(f, "unknown OBS WebSocket opcode {op}"),
        }
    }
}

impl From<JsonError> for ObsWsError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes arbitrary bytes into standard Base64 with `=` padding.
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = input.get(i + 1).copied();
        let b2 = input.get(i + 2).copied();

        let n0 = (b0 >> 2) & 0x3f;
        let n1 = ((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4);
        let n2 = ((b1.unwrap_or(0) & 0x0f) << 2) | (b2.unwrap_or(0) >> 6);
        let n3 = b2.unwrap_or(0) & 0x3f;

        out.push(B64_ALPHABET[usize::from(n0)] as char);
        out.push(B64_ALPHABET[usize::from(n1)] as char);
        out.push(if b1.is_some() {
            B64_ALPHABET[usize::from(n2)] as char
        } else {
            '='
        });
        out.push(if b2.is_some() {
            B64_ALPHABET[usize::from(n3)] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

/// Computes the OBS WebSocket v5 authentication response string:
/// `base64(sha256(base64(sha256(password + salt)) + challenge))`.
pub fn compute_obs_auth(password: &str, salt: &str, challenge: &str) -> String {
    let mut secret_hasher = Sha256::new();
    secret_hasher.update(password.as_bytes());
    secret_hasher.update(salt.as_bytes());
    let secret = base64_encode(&secret_hasher.finalize());

    let mut auth_hasher = Sha256::new();
    auth_hasher.update(secret.as_bytes());
    auth_hasher.update(challenge.as_bytes());
    base64_encode(&auth_hasher.finalize())
}

/// OBS WebSocket v5 message OpCodes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsOpCode {
    Hello = 0,
    Identify = 1,
    Identified = 2,
    Reidentify = 3,
    Event = 5,
    Request = 6,
    RequestResponse = 7,
    RequestBatch = 8,
    RequestBatchResponse = 9,
}

impl ObsOpCode {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Hello),
            1 => Some(Self::Identify),
            2 => Some(Self::Identified),
            3 => Some(Self::Reidentify),
            5 => Some(Self::Event),
            6 => Some(Self::Request),
            7 => Some(Self::RequestResponse),
            8 => Some(Self::RequestBatch),
            9 => Some(Self::RequestBatchResponse),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Authentication challenge sent in `Hello` (OpCode 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsAuthChallenge {
    pub challenge: String,
    pub salt: String,
}

/// Batch execution strategy for `RequestBatch` (OpCode 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsBatchExecutionType {
    None,
    SerialRealtime,
    SerialFrame,
    Parallel,
}

impl ObsBatchExecutionType {
    pub fn to_i64(self) -> i64 {
        match self {
            Self::None => -1,
            Self::SerialRealtime => 0,
            Self::SerialFrame => 1,
            Self::Parallel => 2,
        }
    }

    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            -1 => Some(Self::None),
            0 => Some(Self::SerialRealtime),
            1 => Some(Self::SerialFrame),
            2 => Some(Self::Parallel),
            _ => None,
        }
    }
}

/// A single item in an `ObsMessage::RequestBatch`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObsBatchRequest {
    pub request_id: Option<String>,
    pub request: ObsRequest,
}

/// Top-level OBS WebSocket v5 envelope (`{ "op": <u8>, "d": { ... } }`).
#[derive(Debug, Clone, PartialEq)]
pub enum ObsMessage {
    Hello {
        obs_web_socket_version: String,
        rpc_version: u32,
        authentication: Option<ObsAuthChallenge>,
    },
    Identify {
        rpc_version: u32,
        authentication: Option<String>,
        event_subscriptions: Option<u32>,
    },
    Identified {
        negotiated_rpc_version: u32,
    },
    Reidentify {
        event_subscriptions: Option<u32>,
    },
    Event(ObsEvent),
    Request {
        request_id: String,
        request: ObsRequest,
    },
    RequestResponse {
        request_id: String,
        response: ObsResponse,
    },
    RequestBatch {
        request_id: String,
        halt_on_failure: Option<bool>,
        execution_type: Option<ObsBatchExecutionType>,
        requests: Vec<ObsBatchRequest>,
    },
    RequestBatchResponse {
        request_id: String,
        results: Vec<ObsResponse>,
    },
}

impl ObsMessage {
    /// Returns the wire `ObsOpCode` for this message.
    pub fn opcode(&self) -> ObsOpCode {
        match self {
            Self::Hello { .. } => ObsOpCode::Hello,
            Self::Identify { .. } => ObsOpCode::Identify,
            Self::Identified { .. } => ObsOpCode::Identified,
            Self::Reidentify { .. } => ObsOpCode::Reidentify,
            Self::Event(_) => ObsOpCode::Event,
            Self::Request { .. } => ObsOpCode::Request,
            Self::RequestResponse { .. } => ObsOpCode::RequestResponse,
            Self::RequestBatch { .. } => ObsOpCode::RequestBatch,
            Self::RequestBatchResponse { .. } => ObsOpCode::RequestBatchResponse,
        }
    }

    /// Serializes the envelope into a `JsonVal`.
    pub fn to_json_val(&self) -> JsonVal {
        let d = match self {
            Self::Hello {
                obs_web_socket_version,
                rpc_version,
                authentication,
            } => {
                let mut obj = JsonVal::obj()
                    .with("obsWebSocketVersion", obs_web_socket_version.as_str())
                    .with("rpcVersion", *rpc_version);
                if let Some(auth) = authentication {
                    obj.set(
                        "authentication",
                        JsonVal::obj()
                            .with("challenge", auth.challenge.as_str())
                            .with("salt", auth.salt.as_str()),
                    );
                }
                obj
            }
            Self::Identify {
                rpc_version,
                authentication,
                event_subscriptions,
            } => JsonVal::obj()
                .with("rpcVersion", *rpc_version)
                .with_opt("authentication", authentication.as_deref())
                .with_opt("eventSubscriptions", *event_subscriptions),
            Self::Identified {
                negotiated_rpc_version,
            } => JsonVal::obj().with("negotiatedRpcVersion", *negotiated_rpc_version),
            Self::Reidentify {
                event_subscriptions,
            } => JsonVal::obj().with_opt("eventSubscriptions", *event_subscriptions),
            Self::Event(ev) => ev.to_json_val(),
            Self::Request {
                request_id,
                request,
            } => {
                let mut obj = JsonVal::obj()
                    .with("requestType", request.request_type())
                    .with("requestId", request_id.as_str());
                if let Some(data) = request.request_data_json() {
                    obj.set("requestData", data);
                }
                obj
            }
            Self::RequestResponse {
                request_id,
                response,
            } => response.to_json_val(request_id),
            Self::RequestBatch {
                request_id,
                halt_on_failure,
                execution_type,
                requests,
            } => {
                let items: Vec<JsonVal> = requests
                    .iter()
                    .map(|item| {
                        let mut o = JsonVal::obj()
                            .with("requestType", item.request.request_type())
                            .with_opt("requestId", item.request_id.as_deref());
                        if let Some(data) = item.request.request_data_json() {
                            o.set("requestData", data);
                        }
                        o
                    })
                    .collect();
                JsonVal::obj()
                    .with("requestId", request_id.as_str())
                    .with_opt("haltOnFailure", *halt_on_failure)
                    .with_opt("executionType", execution_type.map(|e| e.to_i64()))
                    .with("requests", JsonVal::Arr(items))
            }
            Self::RequestBatchResponse {
                request_id,
                results,
            } => {
                let items: Vec<JsonVal> = results
                    .iter()
                    .enumerate()
                    .map(|(idx, resp)| resp.to_json_val(&format!("{request_id}:{idx}")))
                    .collect();
                JsonVal::obj()
                    .with("requestId", request_id.as_str())
                    .with("results", JsonVal::Arr(items))
            }
        };

        JsonVal::obj()
            .with("op", self.opcode().as_u8())
            .with("d", d)
    }

    /// Serializes the envelope into a JSON string.
    pub fn to_json(&self) -> String {
        self.to_json_val().to_json()
    }

    /// Parses an OBS WebSocket v5 envelope from a JSON string.
    pub fn from_json(input: &str) -> Result<Self, ObsWsError> {
        let val = JsonVal::parse(input)?;
        Self::from_json_val(&val)
    }

    /// Decodes an OBS WebSocket v5 envelope from a parsed `JsonVal`.
    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        let op_num = val
            .get_u64("op")
            .ok_or(ObsWsError::MissingField("op"))?;
        let op_u8 = u8::try_from(op_num).map_err(|_| ObsWsError::UnknownOpCode(op_num))?;
        let op = ObsOpCode::from_u8(op_u8).ok_or(ObsWsError::UnknownOpCode(op_num))?;
        let d = val.get("d").ok_or(ObsWsError::MissingField("d"))?;

        match op {
            ObsOpCode::Hello => {
                let version = req_str(d, "obsWebSocketVersion")?;
                let rpc_version = req_u32(d, "rpcVersion")?;
                let authentication = match d.get("authentication") {
                    Some(auth) if !auth.is_null() => Some(ObsAuthChallenge {
                        challenge: req_str(auth, "challenge")?,
                        salt: req_str(auth, "salt")?,
                    }),
                    _ => None,
                };
                Ok(Self::Hello {
                    obs_web_socket_version: version,
                    rpc_version,
                    authentication,
                })
            }
            ObsOpCode::Identify => Ok(Self::Identify {
                rpc_version: req_u32(d, "rpcVersion")?,
                authentication: d.get_str("authentication").map(ToString::to_string),
                event_subscriptions: d.get_u32("eventSubscriptions"),
            }),
            ObsOpCode::Identified => Ok(Self::Identified {
                negotiated_rpc_version: req_u32(d, "negotiatedRpcVersion")?,
            }),
            ObsOpCode::Reidentify => Ok(Self::Reidentify {
                event_subscriptions: d.get_u32("eventSubscriptions"),
            }),
            ObsOpCode::Event => Ok(Self::Event(ObsEvent::from_json_val(d)?)),
            ObsOpCode::Request => {
                let request_id = req_str(d, "requestId")?;
                let request_type = req_str(d, "requestType")?;
                let request = ObsRequest::from_parts(&request_type, d.get("requestData"))?;
                Ok(Self::Request {
                    request_id,
                    request,
                })
            }
            ObsOpCode::RequestResponse => {
                let request_id = req_str(d, "requestId")?;
                let response = ObsResponse::from_json_val(d)?;
                Ok(Self::RequestResponse {
                    request_id,
                    response,
                })
            }
            ObsOpCode::RequestBatch => {
                let request_id = req_str(d, "requestId")?;
                let halt_on_failure = d.get_bool("haltOnFailure");
                let execution_type = match d.get_i64("executionType") {
                    Some(v) => Some(
                        ObsBatchExecutionType::from_i64(v)
                            .ok_or(ObsWsError::InvalidField("executionType"))?,
                    ),
                    None => None,
                };
                let arr = d
                    .get_arr("requests")
                    .ok_or(ObsWsError::MissingField("requests"))?;
                let mut requests = Vec::with_capacity(arr.len());
                for item in arr {
                    let rt = req_str(item, "requestType")?;
                    let req = ObsRequest::from_parts(&rt, item.get("requestData"))?;
                    requests.push(ObsBatchRequest {
                        request_id: item.get_str("requestId").map(ToString::to_string),
                        request: req,
                    });
                }
                Ok(Self::RequestBatch {
                    request_id,
                    halt_on_failure,
                    execution_type,
                    requests,
                })
            }
            ObsOpCode::RequestBatchResponse => {
                let request_id = req_str(d, "requestId")?;
                let arr = d
                    .get_arr("results")
                    .ok_or(ObsWsError::MissingField("results"))?;
                let mut results = Vec::with_capacity(arr.len());
                for item in arr {
                    results.push(ObsResponse::from_json_val(item)?);
                }
                Ok(Self::RequestBatchResponse {
                    request_id,
                    results,
                })
            }
        }
    }
}

/// Transform fields for `SetSceneItemTransform`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneItemTransform {
    pub position_x: Option<f64>,
    pub position_y: Option<f64>,
    pub rotation: Option<f64>,
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
    pub crop_left: Option<i64>,
    pub crop_right: Option<i64>,
    pub crop_top: Option<i64>,
    pub crop_bottom: Option<i64>,
    pub bounds_type: Option<String>,
    pub bounds_width: Option<f64>,
    pub bounds_height: Option<f64>,
    pub alignment: Option<u32>,
    pub bounds_alignment: Option<u32>,
}

impl SceneItemTransform {
    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with_opt("positionX", self.position_x)
            .with_opt("positionY", self.position_y)
            .with_opt("rotation", self.rotation)
            .with_opt("scaleX", self.scale_x)
            .with_opt("scaleY", self.scale_y)
            .with_opt("cropLeft", self.crop_left)
            .with_opt("cropRight", self.crop_right)
            .with_opt("cropTop", self.crop_top)
            .with_opt("cropBottom", self.crop_bottom)
            .with_opt("boundsType", self.bounds_type.as_deref())
            .with_opt("boundsWidth", self.bounds_width)
            .with_opt("boundsHeight", self.bounds_height)
            .with_opt("alignment", self.alignment)
            .with_opt("boundsAlignment", self.bounds_alignment)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        if val.as_obj().is_none() {
            return Err(ObsWsError::InvalidField("sceneItemTransform"));
        }
        Ok(Self {
            position_x: val.get_f64("positionX"),
            position_y: val.get_f64("positionY"),
            rotation: val.get_f64("rotation"),
            scale_x: val.get_f64("scaleX"),
            scale_y: val.get_f64("scaleY"),
            crop_left: val.get_i64("cropLeft"),
            crop_right: val.get_i64("cropRight"),
            crop_top: val.get_i64("cropTop"),
            crop_bottom: val.get_i64("cropBottom"),
            bounds_type: val.get_str("boundsType").map(ToString::to_string),
            bounds_width: val.get_f64("boundsWidth"),
            bounds_height: val.get_f64("boundsHeight"),
            alignment: val.get_u32("alignment"),
            bounds_alignment: val.get_u32("boundsAlignment"),
        })
    }
}

/// Typed OBS WebSocket v5 requests.
#[derive(Debug, Clone, PartialEq)]
pub enum ObsRequest {
    // General
    GetVersion,

    // Scenes & Studio Mode
    GetSceneList,
    SetCurrentProgramScene { scene_name: String },
    SetCurrentPreviewScene { scene_name: String },
    GetStudioModeEnabled,
    SetStudioModeEnabled { studio_mode_enabled: bool },
    TriggerStudioModeTransition,

    // Inputs & Audio
    GetInputList { input_kind: Option<String> },
    GetInputMute { input_name: String },
    SetInputMute { input_name: String, input_muted: bool },
    ToggleInputMute { input_name: String },
    GetInputVolume { input_name: String },
    SetInputVolume {
        input_name: String,
        input_volume_mul: Option<f64>,
        input_volume_db: Option<f64>,
    },

    // SceneItems
    GetSceneItemList { scene_name: String },
    SetSceneItemEnabled {
        scene_name: String,
        scene_item_id: i64,
        scene_item_enabled: bool,
    },
    SetSceneItemTransform {
        scene_name: String,
        scene_item_id: i64,
        scene_item_transform: SceneItemTransform,
    },

    // Outputs
    GetStreamStatus,
    StartStream,
    StopStream,
    ToggleStream,
    GetRecordStatus,
    StartRecord,
    StopRecord,
    ToggleRecordPause,
    SaveReplayBuffer,
    GetVirtualCamStatus,
    ToggleVirtualCam,

    // Filters
    GetSourceFilterList { source_name: String },
    SetSourceFilterEnabled {
        source_name: String,
        filter_name: String,
        filter_enabled: bool,
    },

    // Forward-compatible extension point
    Custom {
        request_type: String,
        request_data: Option<JsonVal>,
    },
}

impl ObsRequest {
    /// Returns the OBS WebSocket v5 `requestType` string.
    pub fn request_type(&self) -> &str {
        match self {
            Self::GetVersion => "GetVersion",
            Self::GetSceneList => "GetSceneList",
            Self::SetCurrentProgramScene { .. } => "SetCurrentProgramScene",
            Self::SetCurrentPreviewScene { .. } => "SetCurrentPreviewScene",
            Self::GetStudioModeEnabled => "GetStudioModeEnabled",
            Self::SetStudioModeEnabled { .. } => "SetStudioModeEnabled",
            Self::TriggerStudioModeTransition => "TriggerStudioModeTransition",
            Self::GetInputList { .. } => "GetInputList",
            Self::GetInputMute { .. } => "GetInputMute",
            Self::SetInputMute { .. } => "SetInputMute",
            Self::ToggleInputMute { .. } => "ToggleInputMute",
            Self::GetInputVolume { .. } => "GetInputVolume",
            Self::SetInputVolume { .. } => "SetInputVolume",
            Self::GetSceneItemList { .. } => "GetSceneItemList",
            Self::SetSceneItemEnabled { .. } => "SetSceneItemEnabled",
            Self::SetSceneItemTransform { .. } => "SetSceneItemTransform",
            Self::GetStreamStatus => "GetStreamStatus",
            Self::StartStream => "StartStream",
            Self::StopStream => "StopStream",
            Self::ToggleStream => "ToggleStream",
            Self::GetRecordStatus => "GetRecordStatus",
            Self::StartRecord => "StartRecord",
            Self::StopRecord => "StopRecord",
            Self::ToggleRecordPause => "ToggleRecordPause",
            Self::SaveReplayBuffer => "SaveReplayBuffer",
            Self::GetVirtualCamStatus => "GetVirtualCamStatus",
            Self::ToggleVirtualCam => "ToggleVirtualCam",
            Self::GetSourceFilterList { .. } => "GetSourceFilterList",
            Self::SetSourceFilterEnabled { .. } => "SetSourceFilterEnabled",
            Self::Custom { request_type, .. } => request_type.as_str(),
        }
    }

    /// Builds the `requestData` JSON object, or `None` when the request takes no fields.
    pub fn request_data_json(&self) -> Option<JsonVal> {
        match self {
            Self::GetVersion
            | Self::GetSceneList
            | Self::GetStudioModeEnabled
            | Self::TriggerStudioModeTransition
            | Self::GetStreamStatus
            | Self::StartStream
            | Self::StopStream
            | Self::ToggleStream
            | Self::GetRecordStatus
            | Self::StartRecord
            | Self::StopRecord
            | Self::ToggleRecordPause
            | Self::SaveReplayBuffer
            | Self::GetVirtualCamStatus
            | Self::ToggleVirtualCam => None,
            Self::SetCurrentProgramScene { scene_name }
            | Self::SetCurrentPreviewScene { scene_name }
            | Self::GetSceneItemList { scene_name } => {
                Some(JsonVal::obj().with("sceneName", scene_name.as_str()))
            }
            Self::SetStudioModeEnabled {
                studio_mode_enabled,
            } => Some(JsonVal::obj().with("studioModeEnabled", *studio_mode_enabled)),
            Self::GetInputList { input_kind } => {
                input_kind
                    .as_deref()
                    .map(|kind| JsonVal::obj().with("inputKind", kind))
            }
            Self::GetInputMute { input_name }
            | Self::ToggleInputMute { input_name }
            | Self::GetInputVolume { input_name } => {
                Some(JsonVal::obj().with("inputName", input_name.as_str()))
            }
            Self::SetInputMute {
                input_name,
                input_muted,
            } => Some(
                JsonVal::obj()
                    .with("inputName", input_name.as_str())
                    .with("inputMuted", *input_muted),
            ),
            Self::SetInputVolume {
                input_name,
                input_volume_mul,
                input_volume_db,
            } => Some(
                JsonVal::obj()
                    .with("inputName", input_name.as_str())
                    .with_opt("inputVolumeMul", *input_volume_mul)
                    .with_opt("inputVolumeDb", *input_volume_db),
            ),
            Self::SetSceneItemEnabled {
                scene_name,
                scene_item_id,
                scene_item_enabled,
            } => Some(
                JsonVal::obj()
                    .with("sceneName", scene_name.as_str())
                    .with("sceneItemId", *scene_item_id)
                    .with("sceneItemEnabled", *scene_item_enabled),
            ),
            Self::SetSceneItemTransform {
                scene_name,
                scene_item_id,
                scene_item_transform,
            } => Some(
                JsonVal::obj()
                    .with("sceneName", scene_name.as_str())
                    .with("sceneItemId", *scene_item_id)
                    .with("sceneItemTransform", scene_item_transform.to_json_val()),
            ),
            Self::GetSourceFilterList { source_name } => {
                Some(JsonVal::obj().with("sourceName", source_name.as_str()))
            }
            Self::SetSourceFilterEnabled {
                source_name,
                filter_name,
                filter_enabled,
            } => Some(
                JsonVal::obj()
                    .with("sourceName", source_name.as_str())
                    .with("filterName", filter_name.as_str())
                    .with("filterEnabled", *filter_enabled),
            ),
            Self::Custom { request_data, .. } => request_data.clone(),
        }
    }

    /// Decodes an `ObsRequest` from `requestType` and optional `requestData`.
    pub fn from_parts(request_type: &str, data: Option<&JsonVal>) -> Result<Self, ObsWsError> {
        let empty = JsonVal::obj();
        let d = data.unwrap_or(&empty);
        match request_type {
            "GetVersion" => Ok(Self::GetVersion),
            "GetSceneList" => Ok(Self::GetSceneList),
            "SetCurrentProgramScene" => Ok(Self::SetCurrentProgramScene {
                scene_name: req_str(d, "sceneName")?,
            }),
            "SetCurrentPreviewScene" => Ok(Self::SetCurrentPreviewScene {
                scene_name: req_str(d, "sceneName")?,
            }),
            "GetStudioModeEnabled" => Ok(Self::GetStudioModeEnabled),
            "SetStudioModeEnabled" => Ok(Self::SetStudioModeEnabled {
                studio_mode_enabled: req_bool(d, "studioModeEnabled")?,
            }),
            "TriggerStudioModeTransition" => Ok(Self::TriggerStudioModeTransition),
            "GetInputList" => Ok(Self::GetInputList {
                input_kind: d.get_str("inputKind").map(ToString::to_string),
            }),
            "GetInputMute" => Ok(Self::GetInputMute {
                input_name: req_str(d, "inputName")?,
            }),
            "SetInputMute" => Ok(Self::SetInputMute {
                input_name: req_str(d, "inputName")?,
                input_muted: req_bool(d, "inputMuted")?,
            }),
            "ToggleInputMute" => Ok(Self::ToggleInputMute {
                input_name: req_str(d, "inputName")?,
            }),
            "GetInputVolume" => Ok(Self::GetInputVolume {
                input_name: req_str(d, "inputName")?,
            }),
            "SetInputVolume" => Ok(Self::SetInputVolume {
                input_name: req_str(d, "inputName")?,
                input_volume_mul: d.get_f64("inputVolumeMul"),
                input_volume_db: d.get_f64("inputVolumeDb"),
            }),
            "GetSceneItemList" => Ok(Self::GetSceneItemList {
                scene_name: req_str(d, "sceneName")?,
            }),
            "SetSceneItemEnabled" => Ok(Self::SetSceneItemEnabled {
                scene_name: req_str(d, "sceneName")?,
                scene_item_id: req_i64(d, "sceneItemId")?,
                scene_item_enabled: req_bool(d, "sceneItemEnabled")?,
            }),
            "SetSceneItemTransform" => {
                let t = d
                    .get("sceneItemTransform")
                    .ok_or(ObsWsError::MissingField("sceneItemTransform"))?;
                Ok(Self::SetSceneItemTransform {
                    scene_name: req_str(d, "sceneName")?,
                    scene_item_id: req_i64(d, "sceneItemId")?,
                    scene_item_transform: SceneItemTransform::from_json_val(t)?,
                })
            }
            "GetStreamStatus" => Ok(Self::GetStreamStatus),
            "StartStream" => Ok(Self::StartStream),
            "StopStream" => Ok(Self::StopStream),
            "ToggleStream" => Ok(Self::ToggleStream),
            "GetRecordStatus" => Ok(Self::GetRecordStatus),
            "StartRecord" => Ok(Self::StartRecord),
            "StopRecord" => Ok(Self::StopRecord),
            "ToggleRecordPause" => Ok(Self::ToggleRecordPause),
            "SaveReplayBuffer" => Ok(Self::SaveReplayBuffer),
            "GetVirtualCamStatus" => Ok(Self::GetVirtualCamStatus),
            "ToggleVirtualCam" => Ok(Self::ToggleVirtualCam),
            "GetSourceFilterList" => Ok(Self::GetSourceFilterList {
                source_name: req_str(d, "sourceName")?,
            }),
            "SetSourceFilterEnabled" => Ok(Self::SetSourceFilterEnabled {
                source_name: req_str(d, "sourceName")?,
                filter_name: req_str(d, "filterName")?,
                filter_enabled: req_bool(d, "filterEnabled")?,
            }),
            other => Ok(Self::Custom {
                request_type: other.to_string(),
                request_data: data.cloned(),
            }),
        }
    }
}

/// Status block returned inside `RequestResponse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsRequestStatus {
    pub result: bool,
    pub code: u32,
    pub comment: Option<String>,
}

impl ObsRequestStatus {
    pub fn ok() -> Self {
        Self {
            result: true,
            code: status_code::SUCCESS,
            comment: None,
        }
    }

    pub fn err(code: u32, comment: impl Into<String>) -> Self {
        Self {
            result: false,
            code,
            comment: Some(comment.into()),
        }
    }

    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("result", self.result)
            .with("code", self.code)
            .with_opt("comment", self.comment.as_deref())
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        Ok(Self {
            result: req_bool(val, "result")?,
            code: req_u32(val, "code")?,
            comment: val.get_str("comment").map(ToString::to_string),
        })
    }
}

/// Scene metadata returned by `GetSceneList` and `SceneListChanged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsSceneInfo {
    pub scene_name: String,
    pub scene_index: u32,
}

impl ObsSceneInfo {
    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("sceneName", self.scene_name.as_str())
            .with("sceneIndex", self.scene_index)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        Ok(Self {
            scene_name: req_str(val, "sceneName")?,
            scene_index: req_u32(val, "sceneIndex")?,
        })
    }
}

/// Input metadata returned by `GetInputList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsInputInfo {
    pub input_name: String,
    pub input_kind: String,
    pub unversioned_input_kind: String,
}

impl ObsInputInfo {
    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("inputName", self.input_name.as_str())
            .with("inputKind", self.input_kind.as_str())
            .with("unversionedInputKind", self.unversioned_input_kind.as_str())
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        Ok(Self {
            input_name: req_str(val, "inputName")?,
            input_kind: req_str(val, "inputKind")?,
            unversioned_input_kind: req_str(val, "unversionedInputKind")?,
        })
    }
}

/// Scene item metadata returned by `GetSceneItemList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsSceneItemInfo {
    pub scene_item_id: i64,
    pub scene_item_index: u32,
    pub source_name: String,
    pub scene_item_enabled: bool,
    pub is_group: Option<bool>,
}

impl ObsSceneItemInfo {
    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("sceneItemId", self.scene_item_id)
            .with("sceneItemIndex", self.scene_item_index)
            .with("sourceName", self.source_name.as_str())
            .with("sceneItemEnabled", self.scene_item_enabled)
            .with_opt("isGroup", self.is_group)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        Ok(Self {
            scene_item_id: req_i64(val, "sceneItemId")?,
            scene_item_index: req_u32(val, "sceneItemIndex")?,
            source_name: req_str(val, "sourceName")?,
            scene_item_enabled: req_bool(val, "sceneItemEnabled")?,
            is_group: val.get_bool("isGroup"),
        })
    }
}

/// Filter metadata returned by `GetSourceFilterList`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObsFilterInfo {
    pub filter_name: String,
    pub filter_kind: String,
    pub filter_index: u32,
    pub filter_enabled: bool,
    pub filter_settings: Option<JsonVal>,
}

impl ObsFilterInfo {
    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("filterName", self.filter_name.as_str())
            .with("filterKind", self.filter_kind.as_str())
            .with("filterIndex", self.filter_index)
            .with("filterEnabled", self.filter_enabled)
            .with_opt("filterSettings", self.filter_settings.clone())
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        Ok(Self {
            filter_name: req_str(val, "filterName")?,
            filter_kind: req_str(val, "filterKind")?,
            filter_index: req_u32(val, "filterIndex")?,
            filter_enabled: req_bool(val, "filterEnabled")?,
            filter_settings: val.get("filterSettings").cloned(),
        })
    }
}

/// Typed OBS WebSocket v5 response payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ObsResponseData {
    Empty,
    SceneList {
        current_program_scene_name: Option<String>,
        current_preview_scene_name: Option<String>,
        scenes: Vec<ObsSceneInfo>,
    },
    StudioModeEnabled {
        studio_mode_enabled: bool,
    },
    InputList {
        inputs: Vec<ObsInputInfo>,
    },
    InputMute {
        input_muted: bool,
    },
    InputVolume {
        input_volume_mul: f64,
        input_volume_db: f64,
    },
    SceneItemList {
        scene_items: Vec<ObsSceneItemInfo>,
    },
    StreamStatus {
        output_active: bool,
        output_reconnecting: bool,
        output_timecode: String,
        output_duration: u64,
        output_congestion: f64,
        output_bytes: u64,
        output_skipped_frames: u64,
        output_total_frames: u64,
    },
    OutputActive {
        output_active: bool,
    },
    RecordStatus {
        output_active: bool,
        output_paused: bool,
        output_timecode: String,
        output_duration: u64,
        output_bytes: u64,
    },
    StopRecord {
        output_path: String,
    },
    SourceFilterList {
        filters: Vec<ObsFilterInfo>,
    },
    Version {
        obs_version: String,
        obs_web_socket_version: String,
        rpc_version: u32,
        available_requests: Vec<String>,
        supported_image_formats: Vec<String>,
        platform: String,
        platform_description: String,
    },
    Raw(JsonVal),
}

impl ObsResponseData {
    pub fn to_json_val(&self) -> Option<JsonVal> {
        match self {
            Self::Empty => None,
            Self::SceneList {
                current_program_scene_name,
                current_preview_scene_name,
                scenes,
            } => Some(
                JsonVal::obj()
                    .with_opt(
                        "currentProgramSceneName",
                        current_program_scene_name.as_deref(),
                    )
                    .with_opt(
                        "currentPreviewSceneName",
                        current_preview_scene_name.as_deref(),
                    )
                    .with(
                        "scenes",
                        JsonVal::Arr(scenes.iter().map(ObsSceneInfo::to_json_val).collect()),
                    ),
            ),
            Self::StudioModeEnabled {
                studio_mode_enabled,
            } => Some(JsonVal::obj().with("studioModeEnabled", *studio_mode_enabled)),
            Self::InputList { inputs } => Some(JsonVal::obj().with(
                "inputs",
                JsonVal::Arr(inputs.iter().map(ObsInputInfo::to_json_val).collect()),
            )),
            Self::InputMute { input_muted } => {
                Some(JsonVal::obj().with("inputMuted", *input_muted))
            }
            Self::InputVolume {
                input_volume_mul,
                input_volume_db,
            } => Some(
                JsonVal::obj()
                    .with("inputVolumeMul", *input_volume_mul)
                    .with("inputVolumeDb", *input_volume_db),
            ),
            Self::SceneItemList { scene_items } => Some(JsonVal::obj().with(
                "sceneItems",
                JsonVal::Arr(
                    scene_items
                        .iter()
                        .map(ObsSceneItemInfo::to_json_val)
                        .collect(),
                ),
            )),
            Self::StreamStatus {
                output_active,
                output_reconnecting,
                output_timecode,
                output_duration,
                output_congestion,
                output_bytes,
                output_skipped_frames,
                output_total_frames,
            } => Some(
                JsonVal::obj()
                    .with("outputActive", *output_active)
                    .with("outputReconnecting", *output_reconnecting)
                    .with("outputTimecode", output_timecode.as_str())
                    .with("outputDuration", *output_duration)
                    .with("outputCongestion", *output_congestion)
                    .with("outputBytes", *output_bytes)
                    .with("outputSkippedFrames", *output_skipped_frames)
                    .with("outputTotalFrames", *output_total_frames),
            ),
            Self::OutputActive { output_active } => {
                Some(JsonVal::obj().with("outputActive", *output_active))
            }
            Self::RecordStatus {
                output_active,
                output_paused,
                output_timecode,
                output_duration,
                output_bytes,
            } => Some(
                JsonVal::obj()
                    .with("outputActive", *output_active)
                    .with("outputPaused", *output_paused)
                    .with("outputTimecode", output_timecode.as_str())
                    .with("outputDuration", *output_duration)
                    .with("outputBytes", *output_bytes),
            ),
            Self::StopRecord { output_path } => {
                Some(JsonVal::obj().with("outputPath", output_path.as_str()))
            }
            Self::SourceFilterList { filters } => Some(JsonVal::obj().with(
                "filters",
                JsonVal::Arr(filters.iter().map(ObsFilterInfo::to_json_val).collect()),
            )),
            Self::Version {
                obs_version,
                obs_web_socket_version,
                rpc_version,
                available_requests,
                supported_image_formats,
                platform,
                platform_description,
            } => Some(
                JsonVal::obj()
                    .with("obsVersion", obs_version.as_str())
                    .with("obsWebSocketVersion", obs_web_socket_version.as_str())
                    .with("rpcVersion", *rpc_version)
                    .with(
                        "availableRequests",
                        JsonVal::Arr(
                            available_requests
                                .iter()
                                .map(|s| JsonVal::Str(s.clone()))
                                .collect(),
                        ),
                    )
                    .with(
                        "supportedImageFormats",
                        JsonVal::Arr(
                            supported_image_formats
                                .iter()
                                .map(|s| JsonVal::Str(s.clone()))
                                .collect(),
                        ),
                    )
                    .with("platform", platform.as_str())
                    .with("platformDescription", platform_description.as_str()),
            ),
            Self::Raw(v) => Some(v.clone()),
        }
    }

    pub fn from_parts(request_type: &str, data: Option<&JsonVal>) -> Result<Self, ObsWsError> {
        let Some(d) = data else {
            return Ok(Self::Empty);
        };
        if d.is_null() {
            return Ok(Self::Empty);
        }
        match request_type {
            "GetSceneList" => {
                let arr = d
                    .get_arr("scenes")
                    .ok_or(ObsWsError::MissingField("scenes"))?;
                let mut scenes = Vec::with_capacity(arr.len());
                for item in arr {
                    scenes.push(ObsSceneInfo::from_json_val(item)?);
                }
                Ok(Self::SceneList {
                    current_program_scene_name: d
                        .get_str("currentProgramSceneName")
                        .map(ToString::to_string),
                    current_preview_scene_name: d
                        .get_str("currentPreviewSceneName")
                        .map(ToString::to_string),
                    scenes,
                })
            }
            "GetStudioModeEnabled" => Ok(Self::StudioModeEnabled {
                studio_mode_enabled: req_bool(d, "studioModeEnabled")?,
            }),
            "GetInputList" => {
                let arr = d
                    .get_arr("inputs")
                    .ok_or(ObsWsError::MissingField("inputs"))?;
                let mut inputs = Vec::with_capacity(arr.len());
                for item in arr {
                    inputs.push(ObsInputInfo::from_json_val(item)?);
                }
                Ok(Self::InputList { inputs })
            }
            "GetInputMute" | "ToggleInputMute" => Ok(Self::InputMute {
                input_muted: req_bool(d, "inputMuted")?,
            }),
            "GetInputVolume" => Ok(Self::InputVolume {
                input_volume_mul: req_f64(d, "inputVolumeMul")?,
                input_volume_db: req_f64(d, "inputVolumeDb")?,
            }),
            "GetSceneItemList" => {
                let arr = d
                    .get_arr("sceneItems")
                    .ok_or(ObsWsError::MissingField("sceneItems"))?;
                let mut scene_items = Vec::with_capacity(arr.len());
                for item in arr {
                    scene_items.push(ObsSceneItemInfo::from_json_val(item)?);
                }
                Ok(Self::SceneItemList { scene_items })
            }
            "GetStreamStatus" => Ok(Self::StreamStatus {
                output_active: req_bool(d, "outputActive")?,
                output_reconnecting: req_bool(d, "outputReconnecting")?,
                output_timecode: req_str(d, "outputTimecode")?,
                output_duration: req_u64(d, "outputDuration")?,
                output_congestion: req_f64(d, "outputCongestion")?,
                output_bytes: req_u64(d, "outputBytes")?,
                output_skipped_frames: req_u64(d, "outputSkippedFrames")?,
                output_total_frames: req_u64(d, "outputTotalFrames")?,
            }),
            "ToggleStream" | "GetVirtualCamStatus" | "ToggleVirtualCam" => Ok(Self::OutputActive {
                output_active: req_bool(d, "outputActive")?,
            }),
            "GetRecordStatus" => Ok(Self::RecordStatus {
                output_active: req_bool(d, "outputActive")?,
                output_paused: req_bool(d, "outputPaused")?,
                output_timecode: req_str(d, "outputTimecode")?,
                output_duration: req_u64(d, "outputDuration")?,
                output_bytes: req_u64(d, "outputBytes")?,
            }),
            "StopRecord" => Ok(Self::StopRecord {
                output_path: req_str(d, "outputPath")?,
            }),
            "GetSourceFilterList" => {
                let arr = d
                    .get_arr("filters")
                    .ok_or(ObsWsError::MissingField("filters"))?;
                let mut filters = Vec::with_capacity(arr.len());
                for item in arr {
                    filters.push(ObsFilterInfo::from_json_val(item)?);
                }
                Ok(Self::SourceFilterList { filters })
            }
            "GetVersion" => Ok(Self::Version {
                obs_version: req_str(d, "obsVersion")?,
                obs_web_socket_version: req_str(d, "obsWebSocketVersion")?,
                rpc_version: req_u32(d, "rpcVersion")?,
                available_requests: string_array(d, "availableRequests")?,
                supported_image_formats: string_array(d, "supportedImageFormats")?,
                platform: req_str(d, "platform")?,
                platform_description: req_str(d, "platformDescription")?,
            }),
            _ => Ok(Self::Raw(d.clone())),
        }
    }
}

/// Typed OBS WebSocket v5 response (`RequestResponse`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObsResponse {
    pub request_type: String,
    pub status: ObsRequestStatus,
    pub data: ObsResponseData,
}

impl ObsResponse {
    pub fn ok(request_type: impl Into<String>, data: ObsResponseData) -> Self {
        Self {
            request_type: request_type.into(),
            status: ObsRequestStatus::ok(),
            data,
        }
    }

    pub fn err(request_type: impl Into<String>, code: u32, comment: impl Into<String>) -> Self {
        Self {
            request_type: request_type.into(),
            status: ObsRequestStatus::err(code, comment),
            data: ObsResponseData::Empty,
        }
    }

    pub fn to_json_val(&self, request_id: &str) -> JsonVal {
        let mut obj = JsonVal::obj()
            .with("requestType", self.request_type.as_str())
            .with("requestId", request_id)
            .with("requestStatus", self.status.to_json_val());
        if let Some(data) = self.data.to_json_val() {
            obj.set("responseData", data);
        }
        obj
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        let request_type = req_str(val, "requestType")?;
        let status_val = val
            .get("requestStatus")
            .ok_or(ObsWsError::MissingField("requestStatus"))?;
        let status = ObsRequestStatus::from_json_val(status_val)?;
        let data = if status.result {
            ObsResponseData::from_parts(&request_type, val.get("responseData"))?
        } else {
            ObsResponseData::Empty
        };
        Ok(Self {
            request_type,
            status,
            data,
        })
    }
}

/// Typed OBS WebSocket v5 event payload (`OpCode::Event`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObsEvent {
    pub event_intent: Option<u32>,
    pub kind: ObsEventKind,
}

/// Typed OBS WebSocket v5 event variants.
#[derive(Debug, Clone, PartialEq)]
pub enum ObsEventKind {
    CurrentProgramSceneChanged { scene_name: String },
    CurrentPreviewSceneChanged { scene_name: String },
    StudioModeStateChanged { studio_mode_enabled: bool },
    SceneListChanged { scenes: Vec<ObsSceneInfo> },
    InputMuteStateChanged { input_name: String, input_muted: bool },
    InputVolumeChanged {
        input_name: String,
        input_volume_mul: f64,
        input_volume_db: f64,
    },
    SceneItemEnableStateChanged {
        scene_name: String,
        scene_item_id: i64,
        scene_item_enabled: bool,
    },
    StreamStateChanged {
        output_active: bool,
        output_state: String,
    },
    RecordStateChanged {
        output_active: bool,
        output_state: String,
        output_path: Option<String>,
    },
    VirtualcamStateChanged {
        output_active: bool,
        output_state: String,
    },
    ReplayBufferSaved { saved_replay_path: String },
    SourceFilterEnableStateChanged {
        source_name: String,
        filter_name: String,
        filter_enabled: bool,
    },
    ExitStarted,
    Custom {
        event_type: String,
        event_data: Option<JsonVal>,
    },
}

impl ObsEvent {
    pub fn new(kind: ObsEventKind) -> Self {
        let intent = kind.default_intent();
        Self {
            event_intent: Some(intent),
            kind,
        }
    }

    pub fn to_json_val(&self) -> JsonVal {
        let mut obj = JsonVal::obj()
            .with("eventType", self.kind.event_type())
            .with_opt("eventIntent", self.event_intent);
        if let Some(data) = self.kind.event_data_json() {
            obj.set("eventData", data);
        }
        obj
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ObsWsError> {
        let event_type = req_str(val, "eventType")?;
        let event_intent = val.get_u32("eventIntent");
        let kind = ObsEventKind::from_parts(&event_type, val.get("eventData"))?;
        Ok(Self { event_intent, kind })
    }
}

impl ObsEventKind {
    pub fn event_type(&self) -> &str {
        match self {
            Self::CurrentProgramSceneChanged { .. } => "CurrentProgramSceneChanged",
            Self::CurrentPreviewSceneChanged { .. } => "CurrentPreviewSceneChanged",
            Self::StudioModeStateChanged { .. } => "StudioModeStateChanged",
            Self::SceneListChanged { .. } => "SceneListChanged",
            Self::InputMuteStateChanged { .. } => "InputMuteStateChanged",
            Self::InputVolumeChanged { .. } => "InputVolumeChanged",
            Self::SceneItemEnableStateChanged { .. } => "SceneItemEnableStateChanged",
            Self::StreamStateChanged { .. } => "StreamStateChanged",
            Self::RecordStateChanged { .. } => "RecordStateChanged",
            Self::VirtualcamStateChanged { .. } => "VirtualcamStateChanged",
            Self::ReplayBufferSaved { .. } => "ReplayBufferSaved",
            Self::SourceFilterEnableStateChanged { .. } => "SourceFilterEnableStateChanged",
            Self::ExitStarted => "ExitStarted",
            Self::Custom { event_type, .. } => event_type.as_str(),
        }
    }

    pub fn default_intent(&self) -> u32 {
        match self {
            Self::CurrentProgramSceneChanged { .. }
            | Self::CurrentPreviewSceneChanged { .. }
            | Self::SceneListChanged { .. } => event_subscription::SCENES,
            Self::StudioModeStateChanged { .. } => event_subscription::UI,
            Self::InputMuteStateChanged { .. } | Self::InputVolumeChanged { .. } => {
                event_subscription::INPUTS
            }
            Self::SceneItemEnableStateChanged { .. } => event_subscription::SCENE_ITEMS,
            Self::StreamStateChanged { .. }
            | Self::RecordStateChanged { .. }
            | Self::VirtualcamStateChanged { .. }
            | Self::ReplayBufferSaved { .. } => event_subscription::OUTPUTS,
            Self::SourceFilterEnableStateChanged { .. } => event_subscription::FILTERS,
            Self::ExitStarted | Self::Custom { .. } => event_subscription::GENERAL,
        }
    }

    pub fn event_data_json(&self) -> Option<JsonVal> {
        match self {
            Self::CurrentProgramSceneChanged { scene_name }
            | Self::CurrentPreviewSceneChanged { scene_name } => {
                Some(JsonVal::obj().with("sceneName", scene_name.as_str()))
            }
            Self::StudioModeStateChanged {
                studio_mode_enabled,
            } => Some(JsonVal::obj().with("studioModeEnabled", *studio_mode_enabled)),
            Self::SceneListChanged { scenes } => Some(JsonVal::obj().with(
                "scenes",
                JsonVal::Arr(scenes.iter().map(ObsSceneInfo::to_json_val).collect()),
            )),
            Self::InputMuteStateChanged {
                input_name,
                input_muted,
            } => Some(
                JsonVal::obj()
                    .with("inputName", input_name.as_str())
                    .with("inputMuted", *input_muted),
            ),
            Self::InputVolumeChanged {
                input_name,
                input_volume_mul,
                input_volume_db,
            } => Some(
                JsonVal::obj()
                    .with("inputName", input_name.as_str())
                    .with("inputVolumeMul", *input_volume_mul)
                    .with("inputVolumeDb", *input_volume_db),
            ),
            Self::SceneItemEnableStateChanged {
                scene_name,
                scene_item_id,
                scene_item_enabled,
            } => Some(
                JsonVal::obj()
                    .with("sceneName", scene_name.as_str())
                    .with("sceneItemId", *scene_item_id)
                    .with("sceneItemEnabled", *scene_item_enabled),
            ),
            Self::StreamStateChanged {
                output_active,
                output_state,
            }
            | Self::VirtualcamStateChanged {
                output_active,
                output_state,
            } => Some(
                JsonVal::obj()
                    .with("outputActive", *output_active)
                    .with("outputState", output_state.as_str()),
            ),
            Self::RecordStateChanged {
                output_active,
                output_state,
                output_path,
            } => Some(
                JsonVal::obj()
                    .with("outputActive", *output_active)
                    .with("outputState", output_state.as_str())
                    .with_opt("outputPath", output_path.as_deref()),
            ),
            Self::ReplayBufferSaved { saved_replay_path } => {
                Some(JsonVal::obj().with("savedReplayPath", saved_replay_path.as_str()))
            }
            Self::SourceFilterEnableStateChanged {
                source_name,
                filter_name,
                filter_enabled,
            } => Some(
                JsonVal::obj()
                    .with("sourceName", source_name.as_str())
                    .with("filterName", filter_name.as_str())
                    .with("filterEnabled", *filter_enabled),
            ),
            Self::ExitStarted => None,
            Self::Custom { event_data, .. } => event_data.clone(),
        }
    }

    pub fn from_parts(event_type: &str, data: Option<&JsonVal>) -> Result<Self, ObsWsError> {
        let empty = JsonVal::obj();
        let d = data.unwrap_or(&empty);
        match event_type {
            "CurrentProgramSceneChanged" => Ok(Self::CurrentProgramSceneChanged {
                scene_name: req_str(d, "sceneName")?,
            }),
            "CurrentPreviewSceneChanged" => Ok(Self::CurrentPreviewSceneChanged {
                scene_name: req_str(d, "sceneName")?,
            }),
            "StudioModeStateChanged" => Ok(Self::StudioModeStateChanged {
                studio_mode_enabled: req_bool(d, "studioModeEnabled")?,
            }),
            "SceneListChanged" => {
                let arr = d
                    .get_arr("scenes")
                    .ok_or(ObsWsError::MissingField("scenes"))?;
                let mut scenes = Vec::with_capacity(arr.len());
                for item in arr {
                    scenes.push(ObsSceneInfo::from_json_val(item)?);
                }
                Ok(Self::SceneListChanged { scenes })
            }
            "InputMuteStateChanged" => Ok(Self::InputMuteStateChanged {
                input_name: req_str(d, "inputName")?,
                input_muted: req_bool(d, "inputMuted")?,
            }),
            "InputVolumeChanged" => Ok(Self::InputVolumeChanged {
                input_name: req_str(d, "inputName")?,
                input_volume_mul: req_f64(d, "inputVolumeMul")?,
                input_volume_db: req_f64(d, "inputVolumeDb")?,
            }),
            "SceneItemEnableStateChanged" => Ok(Self::SceneItemEnableStateChanged {
                scene_name: req_str(d, "sceneName")?,
                scene_item_id: req_i64(d, "sceneItemId")?,
                scene_item_enabled: req_bool(d, "sceneItemEnabled")?,
            }),
            "StreamStateChanged" => Ok(Self::StreamStateChanged {
                output_active: req_bool(d, "outputActive")?,
                output_state: req_str(d, "outputState")?,
            }),
            "RecordStateChanged" => Ok(Self::RecordStateChanged {
                output_active: req_bool(d, "outputActive")?,
                output_state: req_str(d, "outputState")?,
                output_path: d.get_str("outputPath").map(ToString::to_string),
            }),
            "VirtualcamStateChanged" => Ok(Self::VirtualcamStateChanged {
                output_active: req_bool(d, "outputActive")?,
                output_state: req_str(d, "outputState")?,
            }),
            "ReplayBufferSaved" => Ok(Self::ReplayBufferSaved {
                saved_replay_path: req_str(d, "savedReplayPath")?,
            }),
            "SourceFilterEnableStateChanged" => Ok(Self::SourceFilterEnableStateChanged {
                source_name: req_str(d, "sourceName")?,
                filter_name: req_str(d, "filterName")?,
                filter_enabled: req_bool(d, "filterEnabled")?,
            }),
            "ExitStarted" => Ok(Self::ExitStarted),
            other => Ok(Self::Custom {
                event_type: other.to_string(),
                event_data: data.cloned(),
            }),
        }
    }
}

fn req_str(obj: &JsonVal, key: &'static str) -> Result<String, ObsWsError> {
    obj.get_str(key)
        .map(ToString::to_string)
        .ok_or(ObsWsError::MissingField(key))
}

fn req_bool(obj: &JsonVal, key: &'static str) -> Result<bool, ObsWsError> {
    obj.get_bool(key).ok_or(ObsWsError::MissingField(key))
}

fn req_u32(obj: &JsonVal, key: &'static str) -> Result<u32, ObsWsError> {
    obj.get_u32(key).ok_or(ObsWsError::MissingField(key))
}

fn req_u64(obj: &JsonVal, key: &'static str) -> Result<u64, ObsWsError> {
    obj.get_u64(key).ok_or(ObsWsError::MissingField(key))
}

fn req_i64(obj: &JsonVal, key: &'static str) -> Result<i64, ObsWsError> {
    obj.get_i64(key).ok_or(ObsWsError::MissingField(key))
}

fn req_f64(obj: &JsonVal, key: &'static str) -> Result<f64, ObsWsError> {
    obj.get_f64(key).ok_or(ObsWsError::MissingField(key))
}

fn string_array(obj: &JsonVal, key: &'static str) -> Result<Vec<String>, ObsWsError> {
    let arr = obj.get_arr(key).ok_or(ObsWsError::MissingField(key))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or(ObsWsError::InvalidField(key))?;
        out.push(s.to_string());
    }
    Ok(out)
}
