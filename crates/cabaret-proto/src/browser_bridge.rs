//! `window.obsstudio` Browser Source JavaScript Bridge (`cabaret-browser:` postMessage protocol).
//!
//! Generates the injected `window.obsstudio` bootstrap script and parses/formats
//! `cabaret-browser:` JSON postMessage frames, including `Interact` mouse/wheel/key
//! forwarding events.

use alloc::format;
use alloc::string::{String, ToString};
use core::fmt;

use crate::json::{JsonError, JsonVal};

/// Wire prefix for all Cabaret browser source `postMessage` frames.
pub const BROWSER_BRIDGE_PREFIX: &str = "cabaret-browser:";
/// Reported `window.obsstudio.pluginVersion`.
pub const OBSSTUDIO_PLUGIN_VERSION: &str = "30.0.0-cabaret";

/// Injected JavaScript bootstrap defining `window.obsstudio` and the `cabaret-browser:`
/// `postMessage` transport.
pub const OBSSTUDIO_INIT_JS: &str = r#"(function(){
  if (window.obsstudio && window.obsstudio.__cabaret) return;
  var PREFIX = "cabaret-browser:";
  var nextId = 1;
  var pending = Object.create(null);
  function send(msg) {
    var frame = PREFIX + JSON.stringify(msg);
    if (window.parent && window.parent !== window) {
      window.parent.postMessage(frame, "*");
    } else {
      window.postMessage(frame, "*");
    }
  }
  function rpc(type, cb) {
    var callId = "c" + (nextId++);
    if (typeof cb === "function") pending[callId] = cb;
    send({ kind: type, callId: callId });
  }
  window.addEventListener("message", function(ev) {
    if (typeof ev.data !== "string" || ev.data.indexOf(PREFIX) !== 0) return;
    var msg;
    try { msg = JSON.parse(ev.data.slice(PREFIX.length)); } catch (_) { return; }
    if (msg.kind === "response" && msg.callId && pending[msg.callId]) {
      var cb = pending[msg.callId];
      delete pending[msg.callId];
      cb(msg.payload);
    } else if (msg.kind === "event" && msg.eventName) {
      var ce = new CustomEvent(msg.eventName, { detail: msg.detail || null });
      window.dispatchEvent(ce);
      if (msg.eventName === "obsSourceActiveChanged" && typeof obs.onActiveChange === "function" && msg.detail) {
        obs.onActiveChange(!!msg.detail.active);
      } else if (msg.eventName === "obsSourceVisibleChanged" && typeof obs.onVisibilityChange === "function" && msg.detail) {
        obs.onVisibilityChange(!!msg.detail.visible);
      }
    }
  });
  var obs = {
    __cabaret: true,
    pluginVersion: "30.0.0-cabaret",
    getControlLevel: function(cb) { rpc("getControlLevel", cb); },
    getCurrentScene: function(cb) { rpc("getCurrentScene", cb); },
    getStatus: function(cb) { rpc("getStatus", cb); },
    saveReplayBuffer: function() { send({ kind: "saveReplayBuffer" }); },
    onActiveChange: null,
    onVisibilityChange: null
  };
  window.obsstudio = obs;
  send({ kind: "ready", pluginVersion: obs.pluginVersion });
})();"#;

/// Generates an initialized `window.obsstudio` bootstrap script that embeds an initial
/// [`ControlLevel`] hint alongside [`OBSSTUDIO_INIT_JS`].
pub fn generate_init_js(default_control_level: ControlLevel) -> String {
    format!(
        "window.__cabaretDefaultControlLevel={};\n{}",
        default_control_level.as_u8(),
        OBSSTUDIO_INIT_JS
    )
}

/// OBS Browser Source control-level permission tiers (`0..=5`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlLevel {
    None = 0,
    ReadObs = 1,
    ReadUser = 2,
    Basic = 3,
    Advanced = 4,
    All = 5,
}

impl ControlLevel {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::ReadObs),
            2 => Some(Self::ReadUser),
            3 => Some(Self::Basic),
            4 => Some(Self::Advanced),
            5 => Some(Self::All),
            _ => None,
        }
    }
}

/// Modifier key bitflags for forwarded `Interact` input events.
pub mod modifiers {
    pub const NONE: u32 = 0;
    pub const SHIFT: u32 = 1 << 0;
    pub const CTRL: u32 = 1 << 1;
    pub const ALT: u32 = 1 << 2;
    pub const META: u32 = 1 << 3;
}

/// Mouse buttons for `InteractEvent::MouseDown` / `MouseUp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "middle" => Some(Self::Middle),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// Input events forwarded from Cabaret's Interact window into a Browser Source.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractEvent {
    MouseMove {
        x: f64,
        y: f64,
        modifiers: u32,
    },
    MouseDown {
        x: f64,
        y: f64,
        button: MouseButton,
        click_count: u8,
        modifiers: u32,
    },
    MouseUp {
        x: f64,
        y: f64,
        button: MouseButton,
        click_count: u8,
        modifiers: u32,
    },
    Wheel {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: u32,
    },
    KeyDown {
        key: String,
        code: String,
        key_code: u32,
        modifiers: u32,
        repeat: bool,
    },
    KeyUp {
        key: String,
        code: String,
        key_code: u32,
        modifiers: u32,
    },
    Char {
        text: String,
        modifiers: u32,
    },
    Focus {
        focused: bool,
    },
}

impl InteractEvent {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::MouseMove { x, y, modifiers } => JsonVal::obj()
                .with("event", "mouseMove")
                .with("x", *x)
                .with("y", *y)
                .with("modifiers", *modifiers),
            Self::MouseDown {
                x,
                y,
                button,
                click_count,
                modifiers,
            } => JsonVal::obj()
                .with("event", "mouseDown")
                .with("x", *x)
                .with("y", *y)
                .with("button", button.as_str())
                .with("clickCount", *click_count)
                .with("modifiers", *modifiers),
            Self::MouseUp {
                x,
                y,
                button,
                click_count,
                modifiers,
            } => JsonVal::obj()
                .with("event", "mouseUp")
                .with("x", *x)
                .with("y", *y)
                .with("button", button.as_str())
                .with("clickCount", *click_count)
                .with("modifiers", *modifiers),
            Self::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                modifiers,
            } => JsonVal::obj()
                .with("event", "wheel")
                .with("x", *x)
                .with("y", *y)
                .with("deltaX", *delta_x)
                .with("deltaY", *delta_y)
                .with("modifiers", *modifiers),
            Self::KeyDown {
                key,
                code,
                key_code,
                modifiers,
                repeat,
            } => JsonVal::obj()
                .with("event", "keyDown")
                .with("key", key.as_str())
                .with("code", code.as_str())
                .with("keyCode", *key_code)
                .with("modifiers", *modifiers)
                .with("repeat", *repeat),
            Self::KeyUp {
                key,
                code,
                key_code,
                modifiers,
            } => JsonVal::obj()
                .with("event", "keyUp")
                .with("key", key.as_str())
                .with("code", code.as_str())
                .with("keyCode", *key_code)
                .with("modifiers", *modifiers),
            Self::Char { text, modifiers } => JsonVal::obj()
                .with("event", "char")
                .with("text", text.as_str())
                .with("modifiers", *modifiers),
            Self::Focus { focused } => JsonVal::obj()
                .with("event", "focus")
                .with("focused", *focused),
        }
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, BridgeError> {
        let ev = val
            .get_str("event")
            .ok_or(BridgeError::MissingField("event"))?;
        match ev {
            "mouseMove" => Ok(Self::MouseMove {
                x: req_f64(val, "x")?,
                y: req_f64(val, "y")?,
                modifiers: val.get_u32("modifiers").unwrap_or(0),
            }),
            "mouseDown" => {
                let button_str = val
                    .get_str("button")
                    .ok_or(BridgeError::MissingField("button"))?;
                let button = MouseButton::from_str(button_str)
                    .ok_or(BridgeError::InvalidField("button"))?;
                let click_count = u8::try_from(val.get_u64("clickCount").unwrap_or(1))
                    .map_err(|_| BridgeError::InvalidField("clickCount"))?;
                Ok(Self::MouseDown {
                    x: req_f64(val, "x")?,
                    y: req_f64(val, "y")?,
                    button,
                    click_count,
                    modifiers: val.get_u32("modifiers").unwrap_or(0),
                })
            }
            "mouseUp" => {
                let button_str = val
                    .get_str("button")
                    .ok_or(BridgeError::MissingField("button"))?;
                let button = MouseButton::from_str(button_str)
                    .ok_or(BridgeError::InvalidField("button"))?;
                let click_count = u8::try_from(val.get_u64("clickCount").unwrap_or(1))
                    .map_err(|_| BridgeError::InvalidField("clickCount"))?;
                Ok(Self::MouseUp {
                    x: req_f64(val, "x")?,
                    y: req_f64(val, "y")?,
                    button,
                    click_count,
                    modifiers: val.get_u32("modifiers").unwrap_or(0),
                })
            }
            "wheel" => Ok(Self::Wheel {
                x: req_f64(val, "x")?,
                y: req_f64(val, "y")?,
                delta_x: req_f64(val, "deltaX")?,
                delta_y: req_f64(val, "deltaY")?,
                modifiers: val.get_u32("modifiers").unwrap_or(0),
            }),
            "keyDown" => Ok(Self::KeyDown {
                key: req_str(val, "key")?,
                code: req_str(val, "code")?,
                key_code: val.get_u32("keyCode").unwrap_or(0),
                modifiers: val.get_u32("modifiers").unwrap_or(0),
                repeat: val.get_bool("repeat").unwrap_or(false),
            }),
            "keyUp" => Ok(Self::KeyUp {
                key: req_str(val, "key")?,
                code: req_str(val, "code")?,
                key_code: val.get_u32("keyCode").unwrap_or(0),
                modifiers: val.get_u32("modifiers").unwrap_or(0),
            }),
            "char" => Ok(Self::Char {
                text: req_str(val, "text")?,
                modifiers: val.get_u32("modifiers").unwrap_or(0),
            }),
            "focus" => Ok(Self::Focus {
                focused: val
                    .get_bool("focused")
                    .ok_or(BridgeError::MissingField("focused"))?,
            }),
            _ => Err(BridgeError::InvalidField("event")),
        }
    }
}

/// Messages sent from the Browser Source JS runtime to the Cabaret host.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserToHostMessage {
    Ready { plugin_version: String },
    GetControlLevel { call_id: String },
    GetCurrentScene { call_id: String },
    GetStatus { call_id: String },
    SaveReplayBuffer,
}

impl BrowserToHostMessage {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::Ready { plugin_version } => JsonVal::obj()
                .with("kind", "ready")
                .with("pluginVersion", plugin_version.as_str()),
            Self::GetControlLevel { call_id } => JsonVal::obj()
                .with("kind", "getControlLevel")
                .with("callId", call_id.as_str()),
            Self::GetCurrentScene { call_id } => JsonVal::obj()
                .with("kind", "getCurrentScene")
                .with("callId", call_id.as_str()),
            Self::GetStatus { call_id } => JsonVal::obj()
                .with("kind", "getStatus")
                .with("callId", call_id.as_str()),
            Self::SaveReplayBuffer => JsonVal::obj().with("kind", "saveReplayBuffer"),
        }
    }

    pub fn format_frame(&self) -> String {
        format!("{BROWSER_BRIDGE_PREFIX}{}", self.to_json_val().to_json())
    }

    pub fn parse_frame(frame: &str) -> Result<Self, BridgeError> {
        let json_str = strip_bridge_prefix(frame)?;
        let val = JsonVal::parse(json_str)?;
        Self::from_json_val(&val)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, BridgeError> {
        let kind = val
            .get_str("kind")
            .ok_or(BridgeError::MissingField("kind"))?;
        match kind {
            "ready" => Ok(Self::Ready {
                plugin_version: req_str(val, "pluginVersion")?,
            }),
            "getControlLevel" => Ok(Self::GetControlLevel {
                call_id: req_str(val, "callId")?,
            }),
            "getCurrentScene" => Ok(Self::GetCurrentScene {
                call_id: req_str(val, "callId")?,
            }),
            "getStatus" => Ok(Self::GetStatus {
                call_id: req_str(val, "callId")?,
            }),
            "saveReplayBuffer" => Ok(Self::SaveReplayBuffer),
            _ => Err(BridgeError::InvalidField("kind")),
        }
    }
}

/// Messages sent from the Cabaret host to the Browser Source JS runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum HostToBrowserMessage {
    Response {
        call_id: String,
        payload: JsonVal,
    },
    DispatchEvent {
        event_name: String,
        detail: Option<JsonVal>,
    },
    Interact(InteractEvent),
}

impl HostToBrowserMessage {
    pub fn to_json_val(&self) -> JsonVal {
        match self {
            Self::Response { call_id, payload } => JsonVal::obj()
                .with("kind", "response")
                .with("callId", call_id.as_str())
                .with("payload", payload.clone()),
            Self::DispatchEvent { event_name, detail } => JsonVal::obj()
                .with("kind", "event")
                .with("eventName", event_name.as_str())
                .with_opt("detail", detail.clone()),
            Self::Interact(ev) => JsonVal::obj()
                .with("kind", "interact")
                .with("interact", ev.to_json_val()),
        }
    }

    pub fn format_frame(&self) -> String {
        format!("{BROWSER_BRIDGE_PREFIX}{}", self.to_json_val().to_json())
    }

    pub fn parse_frame(frame: &str) -> Result<Self, BridgeError> {
        let json_str = strip_bridge_prefix(frame)?;
        let val = JsonVal::parse(json_str)?;
        Self::from_json_val(&val)
    }

    pub fn from_json_val(val: &JsonVal) -> Result<Self, BridgeError> {
        let kind = val
            .get_str("kind")
            .ok_or(BridgeError::MissingField("kind"))?;
        match kind {
            "response" => Ok(Self::Response {
                call_id: req_str(val, "callId")?,
                payload: val.get("payload").cloned().unwrap_or(JsonVal::Null),
            }),
            "event" => Ok(Self::DispatchEvent {
                event_name: req_str(val, "eventName")?,
                detail: val.get("detail").cloned(),
            }),
            "interact" => {
                let inner = val
                    .get("interact")
                    .ok_or(BridgeError::MissingField("interact"))?;
                Ok(Self::Interact(InteractEvent::from_json_val(inner)?))
            }
            _ => Err(BridgeError::InvalidField("kind")),
        }
    }
}

/// Errors returned when parsing `cabaret-browser:` postMessage frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    MissingPrefix,
    Json(JsonError),
    MissingField(&'static str),
    InvalidField(&'static str),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => write!(f, "frame missing '{BROWSER_BRIDGE_PREFIX}' prefix"),
            Self::Json(e) => write!(f, "{e}"),
            Self::MissingField(field) => write!(f, "missing browser bridge field '{field}'"),
            Self::InvalidField(field) => write!(f, "invalid browser bridge field '{field}'"),
        }
    }
}

impl From<JsonError> for BridgeError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}

/// Strips the `cabaret-browser:` prefix from a raw `postMessage` string.
pub fn strip_bridge_prefix(frame: &str) -> Result<&str, BridgeError> {
    frame
        .strip_prefix(BROWSER_BRIDGE_PREFIX)
        .ok_or(BridgeError::MissingPrefix)
}

fn req_str(obj: &JsonVal, key: &'static str) -> Result<String, BridgeError> {
    obj.get_str(key)
        .map(ToString::to_string)
        .ok_or(BridgeError::MissingField(key))
}

fn req_f64(obj: &JsonVal, key: &'static str) -> Result<f64, BridgeError> {
    obj.get_f64(key).ok_or(BridgeError::MissingField(key))
}
