//! Out-of-process `obspython` / `obslua` / QuickJS line-delimited JSON RPC (`cabaret-ops`).
//!
//! Inspired by Polyamour's safe `polyops` pattern: external scripts run out-of-process
//! and communicate via single-line JSON-RPC frames with atomic batch commit/rollback
//! and watchdog timeout reporting.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::json::{JsonError, JsonVal};
use crate::obs_ws::{ObsEvent, ObsRequest, ObsWsError};

/// Optional stream header line identifying `cabaret-ops` protocol v1.
pub const CABARET_OPS_HEADER: &str = "cabaret-ops 1";

/// Supported external scripting runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLanguage {
    ObsPython,
    ObsLua,
    QuickJs,
}

impl ScriptLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObsPython => "obspython",
            Self::ObsLua => "obslua",
            Self::QuickJs => "quickjs",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "obspython" | "python" => Some(Self::ObsPython),
            "obslua" | "lua" => Some(Self::ObsLua),
            "quickjs" | "js" => Some(Self::QuickJs),
            _ => None,
        }
    }
}

/// An option entry inside [`ScriptProperty::List`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptListOption {
    pub label: String,
    pub value: String,
}

impl ScriptListOption {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }

    pub fn to_json_val(&self) -> JsonVal {
        JsonVal::obj()
            .with("label", self.label.as_str())
            .with("value", self.value.as_str())
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ScriptRpcError> {
        Ok(Self {
            label: req_str(val, "label")?,
            value: req_str(val, "value")?,
        })
    }
}

/// UI property schema declared by an out-of-process script.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptProperty {
    Bool {
        name: String,
        label: String,
        default_value: bool,
    },
    IntSlider {
        name: String,
        label: String,
        default_value: i64,
        min: i64,
        max: i64,
        step: i64,
    },
    FloatSlider {
        name: String,
        label: String,
        default_value: f64,
        min: f64,
        max: f64,
        step: f64,
    },
    Text {
        name: String,
        label: String,
        default_value: String,
        multiline: bool,
    },
    List {
        name: String,
        label: String,
        default_value: String,
        options: Vec<ScriptListOption>,
    },
    Button {
        name: String,
        label: String,
        callback: String,
    },
}

impl ScriptProperty {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::Bool {
                name,
                label,
                default_value,
            } => JsonVal::obj()
                .with("kind", "bool")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("default", *default_value),
            Self::IntSlider {
                name,
                label,
                default_value,
                min,
                max,
                step,
            } => JsonVal::obj()
                .with("kind", "intSlider")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("default", *default_value)
                .with("min", *min)
                .with("max", *max)
                .with("step", *step),
            Self::FloatSlider {
                name,
                label,
                default_value,
                min,
                max,
                step,
            } => JsonVal::obj()
                .with("kind", "floatSlider")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("default", *default_value)
                .with("min", *min)
                .with("max", *max)
                .with("step", *step),
            Self::Text {
                name,
                label,
                default_value,
                multiline,
            } => JsonVal::obj()
                .with("kind", "text")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("default", default_value.as_str())
                .with("multiline", *multiline),
            Self::List {
                name,
                label,
                default_value,
                options,
            } => JsonVal::obj()
                .with("kind", "list")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("default", default_value.as_str())
                .with(
                    "options",
                    JsonVal::Arr(options.iter().map(ScriptListOption::to_json_val).collect()),
                ),
            Self::Button {
                name,
                label,
                callback,
            } => JsonVal::obj()
                .with("kind", "button")
                .with("name", name.as_str())
                .with("label", label.as_str())
                .with("callback", callback.as_str()),
        }
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ScriptRpcError> {
        let kind = req_str(val, "kind")?;
        let name = req_str(val, "name")?;
        let label = req_str(val, "label")?;
        match kind.as_str() {
            "bool" => Ok(Self::Bool {
                name,
                label,
                default_value: req_bool(val, "default")?,
            }),
            "intSlider" => Ok(Self::IntSlider {
                name,
                label,
                default_value: req_i64(val, "default")?,
                min: req_i64(val, "min")?,
                max: req_i64(val, "max")?,
                step: val.get_i64("step").unwrap_or(1),
            }),
            "floatSlider" => Ok(Self::FloatSlider {
                name,
                label,
                default_value: req_f64(val, "default")?,
                min: req_f64(val, "min")?,
                max: req_f64(val, "max")?,
                step: val.get_f64("step").unwrap_or(0.1),
            }),
            "text" => Ok(Self::Text {
                name,
                label,
                default_value: req_str(val, "default")?,
                multiline: val.get_bool("multiline").unwrap_or(false),
            }),
            "list" => {
                let arr = val
                    .get_arr("options")
                    .ok_or(ScriptRpcError::MissingField("options"))?;
                let mut options = Vec::with_capacity(arr.len());
                for item in arr {
                    options.push(ScriptListOption::from_json_val(item)?);
                }
                Ok(Self::List {
                    name,
                    label,
                    default_value: req_str(val, "default")?,
                    options,
                })
            }
            "button" => Ok(Self::Button {
                name,
                label,
                callback: req_str(val, "callback")?,
            }),
            _ => Err(ScriptRpcError::InvalidField("kind")),
        }
    }
}

/// An action executed inside a [`ScriptRpcMessage::BatchActions`] transaction.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptAction {
    Obs(ObsRequest),
    SetProperty {
        script_id: String,
        name: String,
        value: JsonVal,
    },
    Log {
        level: String,
        message: String,
    },
}

impl ScriptAction {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::Obs(req) => {
                let mut obj = JsonVal::obj()
                    .with("action", "obs")
                    .with("requestType", req.request_type());
                if let Some(data) = req.request_data_json() {
                    obj.set("requestData", data);
                }
                obj
            }
            Self::SetProperty {
                script_id,
                name,
                value,
            } => JsonVal::obj()
                .with("action", "setProperty")
                .with("scriptId", script_id.as_str())
                .with("name", name.as_str())
                .with("value", value.clone()),
            Self::Log { level, message } => JsonVal::obj()
                .with("action", "log")
                .with("level", level.as_str())
                .with("message", message.as_str()),
        }
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ScriptRpcError> {
        let action = req_str(val, "action")?;
        match action.as_str() {
            "obs" => {
                let rt = req_str(val, "requestType")?;
                let req = ObsRequest::from_parts(&rt, val.get("requestData"))?;
                Ok(Self::Obs(req))
            }
            "setProperty" => Ok(Self::SetProperty {
                script_id: req_str(val, "scriptId")?,
                name: req_str(val, "name")?,
                value: val.get("value").cloned().unwrap_or(JsonVal::Null),
            }),
            "log" => Ok(Self::Log {
                level: req_str(val, "level")?,
                message: req_str(val, "message")?,
            }),
            _ => Err(ScriptRpcError::InvalidField("action")),
        }
    }
}

/// Bidirectional line-delimited JSON RPC messages for `cabaret-ops`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptRpcMessage {
    RegisterScript {
        script_id: String,
        name: String,
        version: String,
        language: ScriptLanguage,
        description: String,
    },
    DeclareProperties {
        script_id: String,
        properties: Vec<ScriptProperty>,
    },
    OnTick {
        tick_id: u64,
        dt_secs: f64,
    },
    OnEvent {
        event: ObsEvent,
    },
    PropertyChanged {
        script_id: String,
        name: String,
        value: JsonVal,
    },
    ButtonClicked {
        script_id: String,
        callback: String,
    },
    BatchActions {
        batch_id: String,
        atomic: bool,
        actions: Vec<ScriptAction>,
    },
    BatchResult {
        batch_id: String,
        committed: bool,
        applied_count: usize,
        rolled_back: bool,
        error: Option<String>,
    },
    WatchdogTimeout {
        script_id: String,
        elapsed_ms: u64,
        limit_ms: u64,
    },
    Ack {
        id: Option<String>,
        ok: bool,
        message: Option<String>,
    },
}

/// Parsed line from a `cabaret-ops` stream.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptRpcLine {
    Header,
    Message(ScriptRpcMessage),
}

impl ScriptRpcMessage {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::RegisterScript {
                script_id,
                name,
                version,
                language,
                description,
            } => JsonVal::obj()
                .with("type", "RegisterScript")
                .with("scriptId", script_id.as_str())
                .with("name", name.as_str())
                .with("version", version.as_str())
                .with("language", language.as_str())
                .with("description", description.as_str()),
            Self::DeclareProperties {
                script_id,
                properties,
            } => JsonVal::obj()
                .with("type", "DeclareProperties")
                .with("scriptId", script_id.as_str())
                .with(
                    "properties",
                    JsonVal::Arr(properties.iter().map(ScriptProperty::to_json_val).collect()),
                ),
            Self::OnTick { tick_id, dt_secs } => JsonVal::obj()
                .with("type", "OnTick")
                .with("tickId", *tick_id)
                .with("dtSecs", *dt_secs),
            Self::OnEvent { event } => JsonVal::obj()
                .with("type", "OnEvent")
                .with("event", event.to_json_val()),
            Self::PropertyChanged {
                script_id,
                name,
                value,
            } => JsonVal::obj()
                .with("type", "PropertyChanged")
                .with("scriptId", script_id.as_str())
                .with("name", name.as_str())
                .with("value", value.clone()),
            Self::ButtonClicked {
                script_id,
                callback,
            } => JsonVal::obj()
                .with("type", "ButtonClicked")
                .with("scriptId", script_id.as_str())
                .with("callback", callback.as_str()),
            Self::BatchActions {
                batch_id,
                atomic,
                actions,
            } => JsonVal::obj()
                .with("type", "BatchActions")
                .with("batchId", batch_id.as_str())
                .with("atomic", *atomic)
                .with(
                    "actions",
                    JsonVal::Arr(actions.iter().map(ScriptAction::to_json_val).collect()),
                ),
            Self::BatchResult {
                batch_id,
                committed,
                applied_count,
                rolled_back,
                error,
            } => JsonVal::obj()
                .with("type", "BatchResult")
                .with("batchId", batch_id.as_str())
                .with("committed", *committed)
                .with("appliedCount", *applied_count)
                .with("rolledBack", *rolled_back)
                .with_opt("error", error.as_deref()),
            Self::WatchdogTimeout {
                script_id,
                elapsed_ms,
                limit_ms,
            } => JsonVal::obj()
                .with("type", "WatchdogTimeout")
                .with("scriptId", script_id.as_str())
                .with("elapsedMs", *elapsed_ms)
                .with("limitMs", *limit_ms),
            Self::Ack { id, ok, message } => JsonVal::obj()
                .with("type", "Ack")
                .with_opt("id", id.as_deref())
                .with("ok", *ok)
                .with_opt("message", message.as_deref()),
        }
    }

    /// Formats the message as a single-line JSON-RPC record (without trailing newline).
    pub fn format_line(&self) -> String {
        self.to_json_val().to_json()
    }

    /// Parses a single JSON-RPC line into a [`ScriptRpcMessage`].
    pub fn parse_line(line: &str) -> Result<Self, ScriptRpcError> {
        let trimmed = line.trim();
        let val = JsonVal::parse(trimmed)?;
        Self::from_json_val(&val)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, ScriptRpcError> {
        let kind = req_str(val, "type")?;
        match kind.as_str() {
            "RegisterScript" => {
                let lang_str = req_str(val, "language")?;
                let language = ScriptLanguage::from_str(&lang_str)
                    .ok_or(ScriptRpcError::InvalidField("language"))?;
                Ok(Self::RegisterScript {
                    script_id: req_str(val, "scriptId")?,
                    name: req_str(val, "name")?,
                    version: req_str(val, "version")?,
                    language,
                    description: val
                        .get_str("description")
                        .unwrap_or("")
                        .to_string(),
                })
            }
            "DeclareProperties" => {
                let arr = val
                    .get_arr("properties")
                    .ok_or(ScriptRpcError::MissingField("properties"))?;
                let mut properties = Vec::with_capacity(arr.len());
                for item in arr {
                    properties.push(ScriptProperty::from_json_val(item)?);
                }
                Ok(Self::DeclareProperties {
                    script_id: req_str(val, "scriptId")?,
                    properties,
                })
            }
            "OnTick" => Ok(Self::OnTick {
                tick_id: req_u64(val, "tickId")?,
                dt_secs: req_f64(val, "dtSecs")?,
            }),
            "OnEvent" => {
                let ev = val
                    .get("event")
                    .ok_or(ScriptRpcError::MissingField("event"))?;
                Ok(Self::OnEvent {
                    event: ObsEvent::from_json_val(ev)?,
                })
            }
            "PropertyChanged" => Ok(Self::PropertyChanged {
                script_id: req_str(val, "scriptId")?,
                name: req_str(val, "name")?,
                value: val.get("value").cloned().unwrap_or(JsonVal::Null),
            }),
            "ButtonClicked" => Ok(Self::ButtonClicked {
                script_id: req_str(val, "scriptId")?,
                callback: req_str(val, "callback")?,
            }),
            "BatchActions" => {
                let arr = val
                    .get_arr("actions")
                    .ok_or(ScriptRpcError::MissingField("actions"))?;
                let mut actions = Vec::with_capacity(arr.len());
                for item in arr {
                    actions.push(ScriptAction::from_json_val(item)?);
                }
                Ok(Self::BatchActions {
                    batch_id: req_str(val, "batchId")?,
                    atomic: val.get_bool("atomic").unwrap_or(true),
                    actions,
                })
            }
            "BatchResult" => Ok(Self::BatchResult {
                batch_id: req_str(val, "batchId")?,
                committed: req_bool(val, "committed")?,
                applied_count: req_u64(val, "appliedCount")? as usize,
                rolled_back: req_bool(val, "rolledBack")?,
                error: val.get_str("error").map(ToString::to_string),
            }),
            "WatchdogTimeout" => Ok(Self::WatchdogTimeout {
                script_id: req_str(val, "scriptId")?,
                elapsed_ms: req_u64(val, "elapsedMs")?,
                limit_ms: req_u64(val, "limitMs")?,
            }),
            "Ack" => Ok(Self::Ack {
                id: val.get_str("id").map(ToString::to_string),
                ok: req_bool(val, "ok")?,
                message: val.get_str("message").map(ToString::to_string),
            }),
            _ => Err(ScriptRpcError::InvalidField("type")),
        }
    }
}

/// Parses one line from a `cabaret-ops` stream, skipping empty or `#` comment lines.
pub fn parse_stream_line(line: &str) -> Result<Option<ScriptRpcLine>, ScriptRpcError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }
    if trimmed == CABARET_OPS_HEADER {
        return Ok(Some(ScriptRpcLine::Header));
    }
    ScriptRpcMessage::parse_line(trimmed).map(|msg| Some(ScriptRpcLine::Message(msg)))
}

/// Executes a [`ScriptRpcMessage::BatchActions`] payload against a `Clone` state `S`.
///
/// When `atomic` is `true`, actions run on a cloned snapshot; if any action fails,
/// `state` remains untouched (rolled back) and `BatchResult { committed: false, rolled_back: true, .. }`
/// is returned.
pub fn execute_batch_atomic<S, F>(
    state: &mut S,
    batch_id: impl Into<String>,
    atomic: bool,
    actions: &[ScriptAction],
    mut apply: F,
) -> ScriptRpcMessage
where
    S: Clone,
    F: FnMut(&mut S, &ScriptAction) -> Result<(), String>,
{
    let batch_id = batch_id.into();
    if atomic {
        let mut working = state.clone();
        for action in actions {
            if let Err(err) = apply(&mut working, action) {
                return ScriptRpcMessage::BatchResult {
                    batch_id,
                    committed: false,
                    applied_count: 0,
                    rolled_back: true,
                    error: Some(err),
                };
            }
        }
        *state = working;
        ScriptRpcMessage::BatchResult {
            batch_id,
            committed: true,
            applied_count: actions.len(),
            rolled_back: false,
            error: None,
        }
    } else {
        let mut applied = 0usize;
        for action in actions {
            if let Err(err) = apply(state, action) {
                return ScriptRpcMessage::BatchResult {
                    batch_id,
                    committed: false,
                    applied_count: applied,
                    rolled_back: false,
                    error: Some(err),
                };
            }
            applied += 1;
        }
        ScriptRpcMessage::BatchResult {
            batch_id,
            committed: true,
            applied_count: applied,
            rolled_back: false,
            error: None,
        }
    }
}

/// Errors returned while decoding `cabaret-ops` line-RPC messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptRpcError {
    Json(JsonError),
    ObsWs(ObsWsError),
    MissingField(&'static str),
    InvalidField(&'static str),
}

impl fmt::Display for ScriptRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::ObsWs(e) => write!(f, "{e}"),
            Self::MissingField(field) => write!(f, "missing script RPC field '{field}'"),
            Self::InvalidField(field) => write!(f, "invalid script RPC field '{field}'"),
        }
    }
}

impl From<JsonError> for ScriptRpcError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}

impl From<ObsWsError> for ScriptRpcError {
    fn from(err: ObsWsError) -> Self {
        Self::ObsWs(err)
    }
}

fn req_str(obj: &JsonVal, key: &'static str) -> Result<String, ScriptRpcError> {
    obj.get_str(key)
        .map(ToString::to_string)
        .ok_or(ScriptRpcError::MissingField(key))
}

fn req_bool(obj: &JsonVal, key: &'static str) -> Result<bool, ScriptRpcError> {
    obj.get_bool(key)
        .ok_or(ScriptRpcError::MissingField(key))
}

fn req_i64(obj: &JsonVal, key: &'static str) -> Result<i64, ScriptRpcError> {
    obj.get_i64(key).ok_or(ScriptRpcError::MissingField(key))
}

fn req_u64(obj: &JsonVal, key: &'static str) -> Result<u64, ScriptRpcError> {
    obj.get_u64(key).ok_or(ScriptRpcError::MissingField(key))
}

fn req_f64(obj: &JsonVal, key: &'static str) -> Result<f64, ScriptRpcError> {
    obj.get_f64(key).ok_or(ScriptRpcError::MissingField(key))
}
