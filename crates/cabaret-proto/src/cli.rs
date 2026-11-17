//! OmaTools Headless Agent CLI & self-describing JSON Schema (`SKILL.md`).
//!
//! Parses `cabaret` CLI invocations (`ctl`, `list`, `status`, `schema`, `inspect`, `apply`),
//! translates `ctl` actions to [`ObsRequest`], and emits the machine-readable [`schema()`].

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::json::{JsonError, JsonVal};
use crate::obs_ws::{ObsRequest, OBS_WS_DEFAULT_PORT, OBS_WS_RPC_VERSION};

/// Parsed `cabaret` headless CLI command.
#[derive(Debug, Clone, PartialEq)]
pub enum CliCommand {
    /// `cabaret ctl <action> [args...]`
    Ctl { action: String, args: Vec<String> },
    /// `cabaret list [target]` (e.g. `scenes`, `inputs`, `filters`, `devices`)
    List { target: Option<String> },
    /// `cabaret status [--json]`
    Status { json: bool },
    /// `cabaret schema`
    Schema,
    /// `cabaret inspect [target]`
    Inspect { target: Option<String> },
    /// `cabaret apply <json>`
    Apply { payload: JsonVal },
}

/// Error returned when parsing CLI arguments or translating `ctl` actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    EmptyArgs,
    UnknownCommand(String),
    UnknownFlag(String),
    MissingArg(&'static str),
    UnexpectedArg(String),
    InvalidArg { arg: &'static str, detail: String },
    InvalidJson(JsonError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyArgs => f.write_str("missing command (expected ctl|list|status|schema|inspect|apply)"),
            Self::UnknownCommand(cmd) => write!(f, "unknown cabaret command '{cmd}'"),
            Self::UnknownFlag(flag) => write!(f, "unknown flag '{flag}'"),
            Self::MissingArg(name) => write!(f, "missing required argument <{name}>"),
            Self::UnexpectedArg(arg) => write!(f, "unexpected argument '{arg}'"),
            Self::InvalidArg { arg, detail } => write!(f, "invalid argument <{arg}>: {detail}"),
            Self::InvalidJson(err) => write!(f, "invalid JSON payload: {err}"),
        }
    }
}

impl From<JsonError> for CliError {
    fn from(err: JsonError) -> Self {
        Self::InvalidJson(err)
    }
}

/// Parses CLI tokens into a [`CliCommand`].
///
/// Accepts slices with or without a leading `"cabaret"` binary name
/// (for example `["cabaret", "status", "--json"]` or `["status", "--json"]`).
pub fn parse_args<'a, I>(args: I) -> Result<CliCommand, CliError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut iter = args.into_iter().peekable();
    let Some(first) = iter.peek().copied() else {
        return Err(CliError::EmptyArgs);
    };
    if is_binary_name(first) {
        iter.next();
    }
    let sub = iter.next().ok_or(CliError::EmptyArgs)?;
    match sub {
        "ctl" => {
            let action = iter
                .next()
                .ok_or(CliError::MissingArg("action"))?
                .to_string();
            let rest = iter.map(ToString::to_string).collect();
            Ok(CliCommand::Ctl { action, args: rest })
        }
        "list" => {
            let target = iter.next().map(ToString::to_string);
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArg(extra.to_string()));
            }
            Ok(CliCommand::List { target })
        }
        "status" => {
            let mut json = false;
            for flag in iter {
                match flag {
                    "--json" => json = true,
                    other if other.starts_with('-') => {
                        return Err(CliError::UnknownFlag(other.to_string()));
                    }
                    other => return Err(CliError::UnexpectedArg(other.to_string())),
                }
            }
            Ok(CliCommand::Status { json })
        }
        "schema" => {
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArg(extra.to_string()));
            }
            Ok(CliCommand::Schema)
        }
        "inspect" => {
            let target = iter.next().map(ToString::to_string);
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArg(extra.to_string()));
            }
            Ok(CliCommand::Inspect { target })
        }
        "apply" => {
            let raw = iter.next().ok_or(CliError::MissingArg("json"))?;
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArg(extra.to_string()));
            }
            let payload = JsonVal::parse(raw)?;
            Ok(CliCommand::Apply { payload })
        }
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

/// Converts a [`CliCommand::Ctl`] into an [`ObsRequest`] when it matches a built-in action.
pub fn ctl_to_obs_request(action: &str, args: &[String]) -> Result<ObsRequest, CliError> {
    match action {
        "version" => Ok(ObsRequest::GetVersion),
        "scene" => Ok(ObsRequest::SetCurrentProgramScene {
            scene_name: req_arg(args, 0, "scene_name")?,
        }),
        "preview" => Ok(ObsRequest::SetCurrentPreviewScene {
            scene_name: req_arg(args, 0, "scene_name")?,
        }),
        "studio-mode" => Ok(ObsRequest::SetStudioModeEnabled {
            studio_mode_enabled: parse_bool(&req_arg(args, 0, "enabled")?, "enabled")?,
        }),
        "transition" => Ok(ObsRequest::TriggerStudioModeTransition),
        "mute" => Ok(ObsRequest::SetInputMute {
            input_name: req_arg(args, 0, "input_name")?,
            input_muted: match args.get(1) {
                Some(v) => parse_bool(v, "muted")?,
                None => true,
            },
        }),
        "unmute" => Ok(ObsRequest::SetInputMute {
            input_name: req_arg(args, 0, "input_name")?,
            input_muted: false,
        }),
        "toggle-mute" => Ok(ObsRequest::ToggleInputMute {
            input_name: req_arg(args, 0, "input_name")?,
        }),
        "volume" => {
            let input_name = req_arg(args, 0, "input_name")?;
            let db_raw = req_arg(args, 1, "volume_db")?;
            let db = db_raw.parse::<f64>().map_err(|_| CliError::InvalidArg {
                arg: "volume_db",
                detail: format!("expected float dB value, got '{db_raw}'"),
            })?;
            Ok(ObsRequest::SetInputVolume {
                input_name,
                input_volume_mul: None,
                input_volume_db: Some(db),
            })
        }
        "item-enable" => {
            let scene_name = req_arg(args, 0, "scene_name")?;
            let id_raw = req_arg(args, 1, "scene_item_id")?;
            let scene_item_id = id_raw.parse::<i64>().map_err(|_| CliError::InvalidArg {
                arg: "scene_item_id",
                detail: format!("expected integer id, got '{id_raw}'"),
            })?;
            let enabled = parse_bool(&req_arg(args, 2, "enabled")?, "enabled")?;
            Ok(ObsRequest::SetSceneItemEnabled {
                scene_name,
                scene_item_id,
                scene_item_enabled: enabled,
            })
        }
        "stream-start" => Ok(ObsRequest::StartStream),
        "stream-stop" => Ok(ObsRequest::StopStream),
        "stream-toggle" => Ok(ObsRequest::ToggleStream),
        "record-start" => Ok(ObsRequest::StartRecord),
        "record-stop" => Ok(ObsRequest::StopRecord),
        "record-pause" => Ok(ObsRequest::ToggleRecordPause),
        "replay-save" => Ok(ObsRequest::SaveReplayBuffer),
        "vcam-toggle" => Ok(ObsRequest::ToggleVirtualCam),
        "filter-enable" => Ok(ObsRequest::SetSourceFilterEnabled {
            source_name: req_arg(args, 0, "source_name")?,
            filter_name: req_arg(args, 1, "filter_name")?,
            filter_enabled: parse_bool(&req_arg(args, 2, "enabled")?, "enabled")?,
        }),
        other => Err(CliError::UnknownCommand(format!("ctl {other}"))),
    }
}

/// Returns the self-describing OmaTools JSON schema for AI agents (`SKILL.md`).
pub fn schema() -> JsonVal {
    let commands = vec![
        cmd_doc(
            "ctl",
            "Execute a control action against the active Cabaret control plane",
            &["<action>", "[arg...]"],
            &[],
            "cabaret ctl scene \"Main\"",
        ),
        cmd_doc(
            "list",
            "List scenes, inputs, filters, or attached Stream Deck devices",
            &["[scenes|inputs|devices]"],
            &[],
            "cabaret list scenes",
        ),
        cmd_doc(
            "status",
            "Inspect streaming, recording, virtual cam, and active scene status",
            &[],
            &["--json"],
            "cabaret status --json",
        ),
        cmd_doc(
            "schema",
            "Emit this machine-readable OmaTools JSON schema",
            &[],
            &[],
            "cabaret schema",
        ),
        cmd_doc(
            "inspect",
            "Inspect detailed properties for a scene, input, or script",
            &["[target]"],
            &[],
            "cabaret inspect \"Main\"",
        ),
        cmd_doc(
            "apply",
            "Apply a declarative JSON state patch or atomic batch of ObsRequests",
            &["<json>"],
            &[],
            "cabaret apply '{\"scene\":\"Main\"}'",
        ),
    ];

    let ctl_actions = vec![
        action_doc("version", &[], "Query OBS WebSocket & Cabaret version"),
        action_doc("scene", &["scene_name"], "Switch current program scene"),
        action_doc("preview", &["scene_name"], "Switch studio-mode preview scene"),
        action_doc("studio-mode", &["enabled"], "Enable or disable Studio Mode"),
        action_doc("transition", &[], "Trigger Studio Mode transition to program"),
        action_doc("mute", &["input_name", "[muted]"], "Mute (or set mute state on) an audio input"),
        action_doc("unmute", &["input_name"], "Unmute an audio input"),
        action_doc("toggle-mute", &["input_name"], "Toggle mute on an audio input"),
        action_doc("volume", &["input_name", "volume_db"], "Set input volume in dB"),
        action_doc("item-enable", &["scene_name", "scene_item_id", "enabled"], "Show or hide a scene item"),
        action_doc("stream-start", &[], "Start streaming output"),
        action_doc("stream-stop", &[], "Stop streaming output"),
        action_doc("stream-toggle", &[], "Toggle streaming output"),
        action_doc("record-start", &[], "Start recording output"),
        action_doc("record-stop", &[], "Stop recording output"),
        action_doc("record-pause", &[], "Toggle recording pause"),
        action_doc("replay-save", &[], "Save the replay buffer"),
        action_doc("vcam-toggle", &[], "Toggle virtual camera output"),
        action_doc("filter-enable", &["source_name", "filter_name", "enabled"], "Enable or disable a source filter"),
    ];

    JsonVal::obj()
        .with("tool", "cabaret")
        .with("version", "0.1.0")
        .with("protocol", "obs-ws-v5")
        .with("defaultPort", OBS_WS_DEFAULT_PORT)
        .with("rpcVersion", OBS_WS_RPC_VERSION)
        .with("commands", JsonVal::Arr(commands))
        .with("ctlActions", JsonVal::Arr(ctl_actions))
}

/// Returns [`schema()`] serialized as a compact JSON string.
pub fn schema_json() -> String {
    schema().to_json()
}

fn cmd_doc(
    name: &str,
    description: &str,
    args: &[&str],
    flags: &[&str],
    example: &str,
) -> JsonVal {
    JsonVal::obj()
        .with("name", name)
        .with("description", description)
        .with(
            "args",
            JsonVal::Arr(args.iter().map(|s| JsonVal::Str((*s).to_string())).collect()),
        )
        .with(
            "flags",
            JsonVal::Arr(flags.iter().map(|s| JsonVal::Str((*s).to_string())).collect()),
        )
        .with("example", example)
}

fn action_doc(name: &str, params: &[&str], description: &str) -> JsonVal {
    JsonVal::obj()
        .with("action", name)
        .with(
            "params",
            JsonVal::Arr(params.iter().map(|s| JsonVal::Str((*s).to_string())).collect()),
        )
        .with("description", description)
}

fn is_binary_name(s: &str) -> bool {
    let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
    base == "cabaret" || base == "cabaret.exe"
}

fn req_arg(args: &[String], idx: usize, name: &'static str) -> Result<String, CliError> {
    args.get(idx)
        .cloned()
        .ok_or(CliError::MissingArg(name))
}

fn parse_bool(s: &str, arg: &'static str) -> Result<bool, CliError> {
    match s {
        "true" | "1" | "on" | "yes" => Ok(true),
        "false" | "0" | "off" | "no" => Ok(false),
        _ => Err(CliError::InvalidArg {
            arg,
            detail: format!("expected boolean (true|false|1|0|on|off), got '{s}'"),
        }),
    }
}
