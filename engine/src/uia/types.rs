use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use windows::Win32::UI::Accessibility::{
    ExpandCollapseState, ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded,
    ExpandCollapseState_LeafNode, ExpandCollapseState_PartiallyExpanded, ToggleState,
    ToggleState_Indeterminate, ToggleState_Off, ToggleState_On,
};

/// UIA 語意樹節點（含 @Ref 標籤、豐富狀態與 Cache 預載資訊）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIATreeNode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    pub id: String,
    pub name: String,
    pub class_name: String,
    pub control_type: String,
    pub bounding_rect: [i32; 4], // [left, top, width, height]
    pub center_point: [i32; 2],  // [center_x, center_y]
    #[serde(default = "default_true")]
    pub is_enabled: bool,
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub is_offscreen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toggle_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expand_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_selected: Option<bool>,
    pub supported_patterns: Vec<String>,
    pub children: Vec<UIATreeNode>,
}

fn default_true() -> bool {
    true
}

impl UIATreeNode {
    pub fn find_by_name(&self, target_name: &str) -> Option<&UIATreeNode> {
        if self.name.eq_ignore_ascii_case(target_name) || self.name.contains(target_name) {
            return Some(self);
        }
        for ch in &self.children {
            if let Some(found) = ch.find_by_name(target_name) {
                return Some(found);
            }
        }
        None
    }

    pub fn flatten(&self) -> Vec<UIATreeNode> {
        let mut list = Vec::new();
        list.push(self.clone());
        for ch in &self.children {
            list.extend(ch.flatten());
        }
        list
    }
}

/// 單一 @Ref 節點快照定位證據（Strict Ref Resolution 依據）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefEntry {
    pub ref_id: String,
    pub hwnd: isize,
    pub automation_id: String,
    pub name: String,
    pub control_type: String,
    pub class_name: String,
    pub bounding_rect: [i32; 4],
    pub center_point: [i32; 2],
    pub supported_patterns: Vec<String>,
    /// 同視窗內相同 (control_type, automation_id, name) 的出現序號（解決同名按鈕消歧義）
    pub occurrence_index: usize,
}

/// 記憶體中的 Ref 映射表
#[derive(Debug, Clone, Default)]
pub struct RefSnapshotMap {
    pub snapshot_seq: u64,
    pub hwnd: isize,
    pub entries: HashMap<String, RefEntry>,
}

/// 語意動作執行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIAActionResult {
    pub success: bool,
    pub target: String,
    pub resolved_name: String,
    pub control_type: String,
    pub subaction: String,
    /// 實際觸發機制：如 "InvokePattern.Invoke", "ValuePattern.SetValue", "TogglePattern.Toggle", "fallback_uv_required"
    pub mechanism: String,
    pub fallback_required: bool,
    pub center_point: [i32; 2],
    pub bounding_rect: [i32; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_value: Option<String>,
    pub message: String,
}

/// 將 UIA ControlType ID 轉換為易讀字串
pub(super) fn control_type_to_string(type_id: i32) -> String {
    match type_id {
        50000 => "Button".into(),
        50001 => "Calendar".into(),
        50002 => "CheckBox".into(),
        50003 => "ComboBox".into(),
        50004 => "Edit".into(),
        50005 => "Hyperlink".into(),
        50006 => "Image".into(),
        50007 => "ListItem".into(),
        50008 => "List".into(),
        50009 => "Menu".into(),
        50010 => "MenuBar".into(),
        50011 => "MenuItem".into(),
        50012 => "ProgressBar".into(),
        50013 => "RadioButton".into(),
        50014 => "ScrollBar".into(),
        50015 => "Slider".into(),
        50016 => "Spinner".into(),
        50017 => "StatusBar".into(),
        50018 => "Tab".into(),
        50019 => "TabItem".into(),
        50020 => "Text".into(),
        50021 => "ToolBar".into(),
        50022 => "ToolTip".into(),
        50023 => "Tree".into(),
        50024 => "TreeItem".into(),
        50025 => "Custom".into(),
        50026 => "Group".into(),
        50027 => "Thumb".into(),
        50028 => "DataGrid".into(),
        50029 => "DataItem".into(),
        50030 => "Document".into(),
        50031 => "SplitButton".into(),
        50032 => "Window".into(),
        50033 => "Pane".into(),
        50034 => "Header".into(),
        50035 => "HeaderItem".into(),
        50036 => "Table".into(),
        50037 => "TitleBar".into(),
        50038 => "Separator".into(),
        _ => format!("Control_{}", type_id),
    }
}

pub(super) fn toggle_state_to_string(state: ToggleState) -> String {
    if state == ToggleState_Off {
        "Off".into()
    } else if state == ToggleState_On {
        "On".into()
    } else if state == ToggleState_Indeterminate {
        "Indeterminate".into()
    } else {
        format!("State_{}", state.0)
    }
}

pub(super) fn expand_state_to_string(state: ExpandCollapseState) -> Option<String> {
    if state == ExpandCollapseState_Collapsed {
        Some("Collapsed".into())
    } else if state == ExpandCollapseState_Expanded {
        Some("Expanded".into())
    } else if state == ExpandCollapseState_PartiallyExpanded {
        Some("PartiallyExpanded".into())
    } else if state == ExpandCollapseState_LeafNode {
        None
    } else {
        None
    }
}

/// 判斷控制項是否為值得分配 @Ref 的可互動元素
pub(super) fn is_actionable_node(control_type: &str, patterns: &[String], w: i32, h: i32) -> bool {
    if w <= 0 || h <= 0 {
        return false;
    }
    let has_non_scroll_pattern = patterns.iter().any(|p| p != "ScrollItem");
    if has_non_scroll_pattern {
        return true;
    }
    matches!(
        control_type,
        "Button"
            | "SplitButton"
            | "CheckBox"
            | "RadioButton"
            | "ComboBox"
            | "Edit"
            | "Document"
            | "Hyperlink"
            | "MenuItem"
            | "TabItem"
            | "ListItem"
            | "TreeItem"
            | "Slider"
            | "Spinner"
            | "DataItem"
    )
}

/// 判斷是否為 Chromium / Electron / 原生 UI 中的透明無語意包裝層 (Web Wrapper)
/// 穿透此類節點可避免深度配額被 10+ 層匿名 Pane/Group 吃光
pub(super) fn is_transparent_wrapper(
    raw_depth: usize,
    control_type: &str,
    name: &str,
    id: &str,
    value: &Option<String>,
    patterns: &[String],
) -> bool {
    if raw_depth == 0 {
        return false;
    }
    let is_container = matches!(control_type, "Group" | "Custom" | "Pane");
    if !is_container {
        return false;
    }
    if !name.trim().is_empty() || !id.trim().is_empty() {
        return false;
    }
    if let Some(v) = value {
        if !v.trim().is_empty() {
            return false;
        }
    }
    // Chromium 常在所有空群組掛上 ScrollItem，單純 ScrollItem 不視為具語意節點
    let has_meaningful_action = patterns.iter().any(|p| p != "ScrollItem");
    !has_meaningful_action
}

/// 正規化使用者傳入的 Ref 字串（例如 "@1", "1", "@e1", "e1" 統一轉為 "@1"）
pub fn normalize_ref_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix("@e") {
        if !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("@{}", stripped));
        }
    }
    if let Some(stripped) = trimmed.strip_prefix('@') {
        if !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("@{}", stripped));
        }
    }
    None
}
