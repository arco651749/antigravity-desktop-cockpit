use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum ClientCommand {
    #[serde(rename = "launch_isolated")]
    LaunchIsolated {
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        args: Option<Vec<String>>,
    },
    #[serde(rename = "get_isolated_windows")]
    GetIsolatedWindows,
    #[serde(rename = "get_uia_tree")]
    GetUIATree {
        #[serde(default)]
        hwnd: Option<isize>,
        #[serde(default)]
        depth: Option<usize>,
    },
    #[serde(rename = "uia_invoke")]
    UIAInvoke {
        #[serde(default)]
        hwnd: Option<isize>,
        name: String,
    },
    #[serde(rename = "uia_action")]
    UIAAction {
        #[serde(default)]
        hwnd: Option<isize>,
        target: String,
        #[serde(default = "default_uia_subaction")]
        subaction: String,
        #[serde(default)]
        value: Option<String>,
    },
    #[serde(rename = "get_visual_diff")]
    GetVisualDiff,
    #[serde(rename = "set_capture_mode")]
    SetCaptureMode { mode: String },
    #[serde(rename = "kill_isolated_app")]
    KillIsolatedApp {
        #[serde(default)]
        pid: Option<u32>,
    },
    #[serde(rename = "get_windows")]
    GetWindows,
    #[serde(rename = "get_monitors")]
    GetMonitors,
    #[serde(rename = "dispatch_to_virtual")]
    DispatchToVirtual { hwnd: isize },
    #[serde(rename = "restore_to_primary")]
    RestoreToPrimary { hwnd: isize },
    #[serde(rename = "plug_in_virtual_display")]
    PlugInVirtualDisplay,
    #[serde(rename = "plug_out_virtual_display")]
    PlugOutVirtualDisplay,
    #[serde(rename = "get_clipboard")]
    GetClipboard,
    #[serde(rename = "set_clipboard")]
    SetClipboard { text: String },
    #[serde(rename = "set_window")]
    SetWindow { hwnd: isize },
    #[serde(rename = "set_fps")]
    SetFps { fps: u32 },
    #[serde(rename = "restore_primary_focus")]
    RestorePrimaryFocus,
    #[serde(rename = "mouse")]
    Mouse {
        #[serde(default = "default_source")]
        source: String,
        #[serde(default = "default_mode")]
        mode: String,
        subaction: String,
        #[serde(default = "default_button")]
        button: String,
        #[serde(default)]
        u: f64,
        #[serde(default)]
        v: f64,
        #[serde(default)]
        delta: i32,
    },
    #[serde(rename = "key")]
    Key {
        #[serde(default = "default_source")]
        source: String,
        #[serde(default = "default_mode")]
        mode: String,
        subaction: String,
        #[serde(default)]
        vk: u16,
        #[serde(default)]
        char: Option<char>,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
    #[serde(rename = "type_text")]
    TypeText {
        text: String,
        #[serde(default)]
        hwnd: Option<isize>,
    },
    #[serde(rename = "launch")]
    Launch {
        path: String,
        #[serde(default)]
        args: Option<Vec<String>>,
        #[serde(default = "default_target_screen")]
        target_screen: String,
        #[serde(default)]
        working_dir: Option<String>,
        #[serde(default = "default_true")]
        wait_for_window: bool,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename = "close", alias = "close_window", alias = "terminate")]
    Close {
        #[serde(default)]
        hwnd: Option<isize>,
        #[serde(default)]
        pid: Option<u32>,
        #[serde(default)]
        force: bool,
        #[serde(default = "default_close_timeout_ms")]
        timeout_ms: Option<u64>,
    },
}

fn default_target_screen() -> String {
    "virtual".to_string()
}
fn default_true() -> bool {
    true
}
fn default_timeout_ms() -> Option<u64> {
    Some(4000)
}
fn default_close_timeout_ms() -> Option<u64> {
    Some(2500)
}
fn default_source() -> String {
    "human".to_string()
}
fn default_button() -> String {
    "left".to_string()
}
fn default_mode() -> String {
    "direct".to_string()
}
fn default_uia_subaction() -> String {
    "act".to_string()
}

#[derive(Debug, Serialize)]
pub struct ServerEvent {
    pub event: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ServerEvent {
    pub fn ok(
        event: impl Into<String>,
        message: Option<String>,
        data: Option<serde_json::Value>,
    ) -> Self {
        Self {
            event: event.into(),
            status: "ok".into(),
            message,
            data,
        }
    }

    pub fn err(event: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            status: "error".into(),
            message: Some(message.into()),
            data: None,
        }
    }
}
