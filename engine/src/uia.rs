use crate::desktop::DesktopWindowInfo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{Interface, BSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, ExpandCollapseState, ExpandCollapseState_Collapsed,
    ExpandCollapseState_Expanded, ExpandCollapseState_LeafNode,
    ExpandCollapseState_PartiallyExpanded, IUIAutomation, IUIAutomation2,
    IUIAutomationCacheRequest, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, IUIAutomationLegacyIAccessiblePattern,
    IUIAutomationRangeValuePattern, IUIAutomationScrollItemPattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationTreeWalker,
    IUIAutomationValuePattern, ToggleState, ToggleState_Indeterminate, ToggleState_Off,
    ToggleState_On, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId,
    UIA_ExpandCollapseExpandCollapseStatePropertyId, UIA_ExpandCollapsePatternId,
    UIA_HasKeyboardFocusPropertyId, UIA_InvokePatternId, UIA_IsEnabledPropertyId,
    UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId, UIA_IsPasswordPropertyId,
    UIA_LegacyIAccessiblePatternId, UIA_NamePropertyId, UIA_RangeValuePatternId,
    UIA_RangeValueValuePropertyId, UIA_ScrollItemPatternId, UIA_SelectionItemIsSelectedPropertyId,
    UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ToggleToggleStatePropertyId,
    UIA_ValuePatternId, UIA_ValueValuePropertyId, UIA_CONTROLTYPE_ID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_NULL,
};

/// 最大物理遍歷深度上限，防止異常循環或極深 DOM 造成無窮遞迴
const MAX_RAW_DEPTH: usize = 36;

/// 探測視窗訊息迴圈超時毫秒數
const PUMP_PROBE_TIMEOUT_MS: u32 = 1500;

/// UIA 連線與交易超時毫秒數
const UIA_CONNECTION_TIMEOUT_MS: u32 = 2000;
const UIA_TRANSACTION_TIMEOUT_MS: u32 = 15000;

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
fn control_type_to_string(type_id: i32) -> String {
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

fn toggle_state_to_string(state: ToggleState) -> String {
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

fn expand_state_to_string(state: ExpandCollapseState) -> Option<String> {
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
fn is_actionable_node(control_type: &str, patterns: &[String], w: i32, h: i32) -> bool {
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
fn is_transparent_wrapper(
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

pub struct UIAEngine {
    automation: IUIAutomation,
    ref_map: Mutex<RefSnapshotMap>,
}

unsafe impl Send for UIAEngine {}
unsafe impl Sync for UIAEngine {}

impl UIAEngine {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            // 優先建立 Windows 8+ CUIAutomation8 (IUIAutomation2) 以啟用超時保護
            let automation: IUIAutomation =
                if let Ok(auto8) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) {
                    if let Ok(auto2) = auto8.cast::<IUIAutomation2>() {
                        let _ = auto2.SetConnectionTimeout(UIA_CONNECTION_TIMEOUT_MS);
                        let _ = auto2.SetTransactionTimeout(UIA_TRANSACTION_TIMEOUT_MS);
                    }
                    auto8
                } else {
                    CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                        .map_err(|e| format!("無法建立 IUIAutomation 實例: {:?}", e))?
                };

            Ok(Self {
                automation,
                ref_map: Mutex::new(RefSnapshotMap::default()),
            })
        }
    }

    /// 探測目標 HWND 的訊息迴圈是否正常運作，防止 WM_GETOBJECT 造成無限期 Hang
    fn probe_window_responsive(hwnd_val: isize) -> Result<(), String> {
        unsafe {
            let mut result: usize = 0;
            let lres = SendMessageTimeoutW(
                HWND(hwnd_val as *mut _),
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                PUMP_PROBE_TIMEOUT_MS,
                Some(&mut result),
            );
            if lres.0 == 0 {
                return Err(format!(
                    "目標視窗 (0x{:X}) 執行緒無回應 (WM_NULL 探測超時 {}ms)，已中止 UIA 請求以防止阻塞",
                    hwnd_val, PUMP_PROBE_TIMEOUT_MS
                ));
            }
            Ok(())
        }
    }

    /// 建立批次預載 CacheRequest，將每節點 9+ 次跨進程 RPC 壓縮為單次預取
    unsafe fn build_cache_request(&self) -> Result<IUIAutomationCacheRequest, String> {
        let req = self
            .automation
            .CreateCacheRequest()
            .map_err(|e| format!("CreateCacheRequest 失敗: {:?}", e))?;

        let props = [
            UIA_NamePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_ClassNamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_IsKeyboardFocusablePropertyId,
            UIA_IsPasswordPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_ValueValuePropertyId,
            UIA_ToggleToggleStatePropertyId,
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
            UIA_SelectionItemIsSelectedPropertyId,
            UIA_RangeValueValuePropertyId,
        ];
        for p in props {
            let _ = req.AddProperty(p);
        }

        let patterns = [
            UIA_InvokePatternId,
            UIA_ValuePatternId,
            UIA_TogglePatternId,
            UIA_SelectionItemPatternId,
            UIA_ExpandCollapsePatternId,
            UIA_ScrollItemPatternId,
            UIA_RangeValuePatternId,
            UIA_LegacyIAccessiblePatternId,
        ];
        for pat in patterns {
            let _ = req.AddPattern(pat);
        }

        Ok(req)
    }

    /// 從指定的 HWND 掃描完整的 UIA 語意階層樹（含 CacheRequest 加速、透明容器穿透與 @Ref 自動編號）
    pub fn fetch_tree_from_hwnd(&self, hwnd_val: isize, max_depth: usize) -> Result<UIATreeNode, String> {
        Self::probe_window_responsive(hwnd_val)?;

        unsafe {
            let cache_req = self.build_cache_request()?;
            let elem = self
                .automation
                .ElementFromHandleBuildCache(HWND(hwnd_val as *mut _), &cache_req)
                .or_else(|_| self.automation.ElementFromHandle(HWND(hwnd_val as *mut _)))
                .map_err(|e| format!("ElementFromHandle 失敗 (0x{:X}): {:?}", hwnd_val, e))?;

            let walker = self
                .automation
                .ControlViewWalker()
                .map_err(|e| format!("ControlViewWalker 失敗: {:?}", e))?;

            let mut ref_counter: usize = 0;
            let mut occurrence_tracker: HashMap<(String, String, String), usize> = HashMap::new();
            let mut entries: HashMap<String, RefEntry> = HashMap::new();

            let root_node = self.build_node_recursive(
                hwnd_val,
                &elem,
                &walker,
                &cache_req,
                0,
                0,
                max_depth,
                &mut ref_counter,
                &mut occurrence_tracker,
                &mut entries,
            )?;

            // 更新引擎內的最新 Ref 映射表
            if let Ok(mut lock) = self.ref_map.lock() {
                lock.snapshot_seq += 1;
                lock.hwnd = hwnd_val;
                lock.entries = entries;
            }

            Ok(root_node)
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn build_node_recursive(
        &self,
        hwnd_val: isize,
        elem: &IUIAutomationElement,
        walker: &IUIAutomationTreeWalker,
        cache_req: &IUIAutomationCacheRequest,
        logical_depth: usize,
        raw_depth: usize,
        max_depth: usize,
        ref_counter: &mut usize,
        occurrence_tracker: &mut HashMap<(String, String, String), usize>,
        entries: &mut HashMap<String, RefEntry>,
    ) -> Result<UIATreeNode, String> {
        let name = elem
            .CachedName()
            .or_else(|_| elem.CurrentName())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();
        let class_name = elem
            .CachedClassName()
            .or_else(|_| elem.CurrentClassName())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();
        let id = elem
            .CachedAutomationId()
            .or_else(|_| elem.CurrentAutomationId())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();
        let ctrl_type_id = elem
            .CachedControlType()
            .or_else(|_| elem.CurrentControlType())
            .unwrap_or(UIA_CONTROLTYPE_ID(0))
            .0;
        let control_type = control_type_to_string(ctrl_type_id);

        let rect = elem
            .CachedBoundingRectangle()
            .or_else(|_| elem.CurrentBoundingRectangle())
            .unwrap_or(RECT::default());
        let w = (rect.right - rect.left).max(0);
        let h = (rect.bottom - rect.top).max(0);
        let center_x = rect.left + w / 2;
        let center_y = rect.top + h / 2;

        let is_enabled = elem
            .CachedIsEnabled()
            .or_else(|_| elem.CurrentIsEnabled())
            .map(|b| b.as_bool())
            .unwrap_or(true);
        let is_focused = elem
            .CachedHasKeyboardFocus()
            .or_else(|_| elem.CurrentHasKeyboardFocus())
            .map(|b| b.as_bool())
            .unwrap_or(false);
        let is_offscreen = elem
            .CachedIsOffscreen()
            .or_else(|_| elem.CurrentIsOffscreen())
            .map(|b| b.as_bool())
            .unwrap_or(false);
        let is_password = elem
            .CachedIsPassword()
            .or_else(|_| elem.CurrentIsPassword())
            .map(|b| b.as_bool())
            .unwrap_or(false);

        // 從預取 Cache 讀取支援的 Pattern 與當前狀態值（0 額外跨進程開銷）
        let mut supported_patterns = Vec::new();
        let mut value: Option<String> = None;
        let mut toggle_state: Option<String> = None;
        let mut expand_state: Option<String> = None;
        let mut is_selected: Option<bool> = None;

        if elem.GetCachedPattern(UIA_InvokePatternId).is_ok() {
            supported_patterns.push("Invoke".to_string());
        }
        if let Ok(val_pat) = elem.GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            supported_patterns.push("Value".to_string());
            if is_password {
                value = Some("***".to_string());
            } else if let Ok(v) = val_pat.CachedValue().or_else(|_| val_pat.CurrentValue()) {
                let s = v.to_string();
                if !s.is_empty() {
                    value = Some(s.chars().take(120).collect());
                }
            }
        }
        if let Ok(rng_pat) = elem.GetCachedPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) {
            supported_patterns.push("RangeValue".to_string());
            if value.is_none() {
                if let Ok(v) = rng_pat.CachedValue().or_else(|_| rng_pat.CurrentValue()) {
                    value = Some(format!("{:.2}", v));
                }
            }
        }
        if let Ok(tog_pat) = elem.GetCachedPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            supported_patterns.push("Toggle".to_string());
            if let Ok(st) = tog_pat.CachedToggleState().or_else(|_| tog_pat.CurrentToggleState()) {
                toggle_state = Some(toggle_state_to_string(st));
            }
        }
        if let Ok(sel_pat) = elem.GetCachedPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            supported_patterns.push("SelectionItem".to_string());
            if let Ok(sel) = sel_pat.CachedIsSelected().or_else(|_| sel_pat.CurrentIsSelected()) {
                is_selected = Some(sel.as_bool());
            }
        }
        if let Ok(exp_pat) = elem.GetCachedPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
            if let Ok(st) = exp_pat
                .CachedExpandCollapseState()
                .or_else(|_| exp_pat.CurrentExpandCollapseState())
            {
                if let Some(st_str) = expand_state_to_string(st) {
                    supported_patterns.push("ExpandCollapse".to_string());
                    expand_state = Some(st_str);
                }
            }
        }
        if elem.GetCachedPattern(UIA_ScrollItemPatternId).is_ok() {
            supported_patterns.push("ScrollItem".to_string());
        }
        if let Ok(leg_pat) = elem.GetCachedPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId) {
            if let Ok(def_act) = leg_pat.CachedDefaultAction().or_else(|_| leg_pat.CurrentDefaultAction()) {
                if !def_act.to_string().trim().is_empty() && !supported_patterns.contains(&"Invoke".to_string()) {
                    supported_patterns.push("LegacyIAccessible".to_string());
                }
            }
        }

        // 分配 @Ref 代號給可互動元素
        let key_tuple = (control_type.clone(), id.clone(), name.clone());
        let occ_idx = *occurrence_tracker
            .entry(key_tuple)
            .and_modify(|c| *c += 1)
            .or_insert(0);

        let ref_id = if raw_depth > 0 && is_actionable_node(&control_type, &supported_patterns, w, h) {
            *ref_counter += 1;
            let r_str = format!("@{}", *ref_counter);
            entries.insert(
                r_str.clone(),
                RefEntry {
                    ref_id: r_str.clone(),
                    hwnd: hwnd_val,
                    automation_id: id.clone(),
                    name: name.clone(),
                    control_type: control_type.clone(),
                    class_name: class_name.clone(),
                    bounding_rect: [rect.left, rect.top, w, h],
                    center_point: [center_x, center_y],
                    supported_patterns: supported_patterns.clone(),
                    occurrence_index: occ_idx,
                },
            );
            Some(r_str)
        } else {
            None
        };

        let mut children = Vec::new();
        if logical_depth < max_depth && raw_depth < MAX_RAW_DEPTH {
            let first_child_res = walker
                .GetFirstChildElementBuildCache(elem, cache_req)
                .or_else(|_| walker.GetFirstChildElement(elem));

            if let Ok(first_child) = first_child_res {
                let mut current = Some(first_child);
                while let Some(child_elem) = current {
                    let next_sibling = walker
                        .GetNextSiblingElementBuildCache(&child_elem, cache_req)
                        .or_else(|_| walker.GetNextSiblingElement(&child_elem))
                        .ok();

                    // 先檢查子節點是否為透明包裝層 (Web Wrapper)
                    let child_ctrl = child_elem
                        .CachedControlType()
                        .or_else(|_| child_elem.CurrentControlType())
                        .unwrap_or(UIA_CONTROLTYPE_ID(0))
                        .0;
                    let child_ctrl_str = control_type_to_string(child_ctrl);
                    let child_name = child_elem
                        .CachedName()
                        .or_else(|_| child_elem.CurrentName())
                        .unwrap_or_else(|_| BSTR::new())
                        .to_string();
                    let child_id = child_elem
                        .CachedAutomationId()
                        .or_else(|_| child_elem.CurrentAutomationId())
                        .unwrap_or_else(|_| BSTR::new())
                        .to_string();

                    let child_is_wrapper_candidate = matches!(child_ctrl_str.as_str(), "Group" | "Custom" | "Pane")
                        && child_name.trim().is_empty()
                        && child_id.trim().is_empty();

                    // 若為透明容器，不增加 logical_depth，並將其內部有效子節點直接摺疊提升至當前層級
                    let next_logical_depth = if child_is_wrapper_candidate {
                        logical_depth
                    } else {
                        logical_depth + 1
                    };

                    if let Ok(child_node) = self.build_node_recursive(
                        hwnd_val,
                        &child_elem,
                        walker,
                        cache_req,
                        next_logical_depth,
                        raw_depth + 1,
                        max_depth,
                        ref_counter,
                        occurrence_tracker,
                        entries,
                    ) {
                        if is_transparent_wrapper(
                            raw_depth + 1,
                            &child_node.control_type,
                            &child_node.name,
                            &child_node.id,
                            &child_node.value,
                            &child_node.supported_patterns,
                        ) && child_node.ref_id.is_none()
                        {
                            // 直接提升子節點，消除冗餘包裝層
                            children.extend(child_node.children);
                        } else {
                            children.push(child_node);
                        }
                    }

                    current = next_sibling;
                }
            }
        }

        Ok(UIATreeNode {
            ref_id,
            id,
            name,
            class_name,
            control_type,
            bounding_rect: [rect.left, rect.top, w, h],
            center_point: [center_x, center_y],
            is_enabled,
            is_focused,
            is_offscreen,
            value,
            toggle_state,
            expand_state,
            is_selected,
            supported_patterns,
            children,
        })
    }

    /// 執行基於 @Ref 或元素名稱的多級語意動作鏈 (Activation Chain)
    /// 支援 subaction: "act" / "invoke" / "set_value" / "toggle" / "expand" / "collapse" / "select" / "focus" / "scroll_into_view"
    pub fn execute_uia_action(
        &self,
        explicit_hwnd: isize,
        target: &str,
        subaction: &str,
        value: Option<&str>,
    ) -> Result<UIAActionResult, String> {
        // 1. 檢查是否為 @Ref 引用
        let norm_ref = normalize_ref_key(target);
        let ref_entry_opt = if let Some(ref r_key) = norm_ref {
            let lock = self.ref_map.lock().unwrap();
            lock.entries.get(r_key).cloned()
        } else {
            None
        };

        let target_hwnd = if let Some(ref entry) = ref_entry_opt {
            if explicit_hwnd != 0 {
                explicit_hwnd
            } else {
                entry.hwnd
            }
        } else {
            explicit_hwnd
        };

        if target_hwnd == 0 {
            return Err("未指定有效的目標視窗 HWND，請先執行 `uia` 快照或傳入 `--hwnd`".into());
        }

        Self::probe_window_responsive(target_hwnd)?;

        unsafe {
            let cache_req = self.build_cache_request()?;
            let root = self
                .automation
                .ElementFromHandleBuildCache(HWND(target_hwnd as *mut _), &cache_req)
                .or_else(|_| self.automation.ElementFromHandle(HWND(target_hwnd as *mut _)))
                .map_err(|e| format!("ElementFromHandle 失敗 (0x{:X}): {:?}", target_hwnd, e))?;

            let walker = self
                .automation
                .ControlViewWalker()
                .map_err(|e| format!("ControlViewWalker 失敗: {:?}", e))?;

            // 2. 在活體樹中定位目標 IUIAutomationElement
            let found_elem = if let Some(ref entry) = ref_entry_opt {
                let mut occ_count = 0usize;
                let mut best_fallback: Option<(IUIAutomationElement, i64)> = None;
                self.find_element_by_ref_entry(
                    &root,
                    &walker,
                    &cache_req,
                    entry,
                    0,
                    &mut occ_count,
                    &mut best_fallback,
                )?
                .or_else(|| best_fallback.map(|(el, _)| el))
            } else {
                if norm_ref.is_some() {
                    return Err(format!(
                        "找不到 Ref '{}' 的快照記錄，請先執行 `cockpit_cli.py uia` 重新整理快照",
                        target
                    ));
                }
                self.find_element_by_query(&root, &walker, &cache_req, target, 0)?
            };

            let elem = match found_elem {
                Some(el) => el,
                None => {
                    // 若活體節點已消失但有 Ref 歷史座標，回傳座標供 CLI 決定是否 fallback
                    if let Some(entry) = ref_entry_opt {
                        return Ok(UIAActionResult {
                            success: false,
                            target: target.to_string(),
                            resolved_name: entry.name,
                            control_type: entry.control_type,
                            subaction: subaction.to_string(),
                            mechanism: "stale_ref_fallback_uv".into(),
                            fallback_required: true,
                            center_point: entry.center_point,
                            bounding_rect: entry.bounding_rect,
                            verified_value: None,
                            message: "UIA 節點已變更或無法直接定位，建議自動回退至快照 (u, v) 座標操作".into(),
                        });
                    }
                    return Err(format!("在視窗 0x{:X} 中找不到匹配 '{}' 的 UIA 元素", target_hwnd, target));
                }
            };

            // 讀取定位到的元素最新屬性與座標
            let resolved_name = elem
                .CachedName()
                .or_else(|_| elem.CurrentName())
                .unwrap_or_else(|_| BSTR::new())
                .to_string();
            let ctrl_id = elem
                .CachedControlType()
                .or_else(|_| elem.CurrentControlType())
                .unwrap_or(UIA_CONTROLTYPE_ID(0))
                .0;
            let control_type = control_type_to_string(ctrl_id);
            let rect = elem
                .CachedBoundingRectangle()
                .or_else(|_| elem.CurrentBoundingRectangle())
                .unwrap_or(RECT::default());
            let w = (rect.right - rect.left).max(0);
            let h = (rect.bottom - rect.top).max(0);
            let center_point = [rect.left + w / 2, rect.top + h / 2];
            let bounding_rect = [rect.left, rect.top, w, h];

            // 若元素在畫面外且支援 ScrollItem，先自動捲動至可見區域
            let is_offscreen = elem
                .CachedIsOffscreen()
                .or_else(|_| elem.CurrentIsOffscreen())
                .map(|b| b.as_bool())
                .unwrap_or(false);
            if is_offscreen {
                if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_ScrollItemPatternId) {
                    if let Ok(scroll_item) = pat_unk.cast::<IUIAutomationScrollItemPattern>() {
                        let _ = scroll_item.ScrollIntoView();
                    }
                }
            }

            let action_lower = subaction.trim().to_ascii_lowercase();
            match action_lower.as_str() {
                "act" | "invoke" | "click" => {
                    // Rung 1: InvokePattern::Invoke()
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_InvokePatternId) {
                        if let Ok(inv) = pat_unk.cast::<IUIAutomationInvokePattern>() {
                            inv.Invoke().map_err(|e| format!("InvokePattern::Invoke 失敗: {:?}", e))?;
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: "InvokePattern.Invoke".into(),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: None,
                                message: "已透過 UIA InvokePattern.Invoke() 靜默觸發".into(),
                            });
                        }
                    }

                    // Rung 2: TogglePattern::Toggle() (CheckBox / Switch / ToggleButton)
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_TogglePatternId) {
                        if let Ok(tog) = pat_unk.cast::<IUIAutomationTogglePattern>() {
                            tog.Toggle().map_err(|e| format!("TogglePattern::Toggle 失敗: {:?}", e))?;
                            let new_st = tog
                                .CurrentToggleState()
                                .map(toggle_state_to_string)
                                .ok();
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: "TogglePattern.Toggle".into(),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: new_st,
                                message: "已透過 UIA TogglePattern.Toggle() 靜默切換狀態".into(),
                            });
                        }
                    }

                    // Rung 3: SelectionItemPattern::Select() (TabItem / ListItem / RadioButton)
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_SelectionItemPatternId) {
                        if let Ok(sel) = pat_unk.cast::<IUIAutomationSelectionItemPattern>() {
                            sel.Select().map_err(|e| format!("SelectionItemPattern::Select 失敗: {:?}", e))?;
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: "SelectionItemPattern.Select".into(),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: Some("Selected".into()),
                                message: "已透過 UIA SelectionItemPattern.Select() 靜默選取".into(),
                            });
                        }
                    }

                    // Rung 4: ExpandCollapsePattern (MenuItem / ComboBox / TreeItem)
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_ExpandCollapsePatternId) {
                        if let Ok(exp) = pat_unk.cast::<IUIAutomationExpandCollapsePattern>() {
                            let cur = exp.CurrentExpandCollapseState().unwrap_or(ExpandCollapseState_Collapsed);
                            if cur == ExpandCollapseState_Collapsed {
                                if exp.Expand().is_ok() {
                                    return Ok(UIAActionResult {
                                        success: true,
                                        target: target.to_string(),
                                        resolved_name,
                                        control_type,
                                        subaction: subaction.to_string(),
                                        mechanism: "ExpandCollapsePattern.Expand".into(),
                                        fallback_required: false,
                                        center_point,
                                        bounding_rect,
                                        verified_value: Some("Expanded".into()),
                                        message: "已透過 UIA ExpandCollapsePattern.Expand() 展開".into(),
                                    });
                                }
                            } else if cur == ExpandCollapseState_Expanded && exp.Collapse().is_ok() {
                                return Ok(UIAActionResult {
                                    success: true,
                                    target: target.to_string(),
                                    resolved_name,
                                    control_type,
                                    subaction: subaction.to_string(),
                                    mechanism: "ExpandCollapsePattern.Collapse".into(),
                                    fallback_required: false,
                                    center_point,
                                    bounding_rect,
                                    verified_value: Some("Collapsed".into()),
                                    message: "已透過 UIA ExpandCollapsePattern.Collapse() 收合".into(),
                                });
                            }
                        }
                    }

                    // Rung 5: LegacyIAccessiblePattern::DoDefaultAction()
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_LegacyIAccessiblePatternId) {
                        if let Ok(leg) = pat_unk.cast::<IUIAutomationLegacyIAccessiblePattern>() {
                            if leg.DoDefaultAction().is_ok() {
                                return Ok(UIAActionResult {
                                    success: true,
                                    target: target.to_string(),
                                    resolved_name,
                                    control_type,
                                    subaction: subaction.to_string(),
                                    mechanism: "LegacyIAccessible.DoDefaultAction".into(),
                                    fallback_required: false,
                                    center_point,
                                    bounding_rect,
                                    verified_value: None,
                                    message: "已透過 LegacyIAccessible.DoDefaultAction() 觸發".into(),
                                });
                            }
                        }
                    }

                    // Rung 6: 無可用 COM Pattern，回傳座標讓上層無縫切換 (u, v) 虛擬游標點擊
                    Ok(UIAActionResult {
                        success: false,
                        target: target.to_string(),
                        resolved_name,
                        control_type,
                        subaction: subaction.to_string(),
                        mechanism: "fallback_uv_required".into(),
                        fallback_required: true,
                        center_point,
                        bounding_rect,
                        verified_value: None,
                        message: "該控制項未暴露 Invoke/Toggle Pattern，已準備中心座標供自動降級點擊".into(),
                    })
                }

                "set_value" | "fill" | "clear" => {
                    let write_str = if action_lower == "clear" {
                        ""
                    } else {
                        value.unwrap_or("")
                    };

                    // Rung 1: ValuePattern::SetValue
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_ValuePatternId) {
                        if let Ok(val_pat) = pat_unk.cast::<IUIAutomationValuePattern>() {
                            let is_ro = val_pat.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(false);
                            if !is_ro && val_pat.SetValue(&BSTR::from(write_str)).is_ok() {
                                let is_pwd = elem.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false);
                                let verified = if is_pwd {
                                    Some("***".into())
                                } else {
                                    val_pat.CurrentValue().ok().map(|b| b.to_string())
                                };
                                return Ok(UIAActionResult {
                                    success: true,
                                    target: target.to_string(),
                                    resolved_name,
                                    control_type,
                                    subaction: subaction.to_string(),
                                    mechanism: "ValuePattern.SetValue".into(),
                                    fallback_required: false,
                                    center_point,
                                    bounding_rect,
                                    verified_value: verified,
                                    message: format!("已透過 UIA ValuePattern.SetValue 瞬間寫入 ({} 字元)", write_str.chars().count()),
                                });
                            }
                        }
                    }

                    // Rung 2: RangeValuePattern::SetValue (針對數值滑桿/Spinner)
                    if let Ok(num_val) = write_str.trim().parse::<f64>() {
                        if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_RangeValuePatternId) {
                            if let Ok(rng_pat) = pat_unk.cast::<IUIAutomationRangeValuePattern>() {
                                if rng_pat.SetValue(num_val).is_ok() {
                                    let verified = rng_pat.CurrentValue().ok().map(|v| format!("{:.2}", v));
                                    return Ok(UIAActionResult {
                                        success: true,
                                        target: target.to_string(),
                                        resolved_name,
                                        control_type,
                                        subaction: subaction.to_string(),
                                        mechanism: "RangeValuePattern.SetValue".into(),
                                        fallback_required: false,
                                        center_point,
                                        bounding_rect,
                                        verified_value: verified,
                                        message: format!("已透過 UIA RangeValuePattern.SetValue 設定數值 {}", num_val),
                                    });
                                }
                            }
                        }
                    }

                    // Rung 3: LegacyIAccessiblePattern::SetValue
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_LegacyIAccessiblePatternId) {
                        if let Ok(leg_pat) = pat_unk.cast::<IUIAutomationLegacyIAccessiblePattern>() {
                            let wide: Vec<u16> = write_str.encode_utf16().chain(std::iter::once(0)).collect();
                            if leg_pat.SetValue(PCWSTR(wide.as_ptr())).is_ok() {
                                return Ok(UIAActionResult {
                                    success: true,
                                    target: target.to_string(),
                                    resolved_name,
                                    control_type,
                                    subaction: subaction.to_string(),
                                    mechanism: "LegacyIAccessible.SetValue".into(),
                                    fallback_required: false,
                                    center_point,
                                    bounding_rect,
                                    verified_value: Some(write_str.to_string()),
                                    message: "已透過 LegacyIAccessible.SetValue 寫入文字".into(),
                                });
                            }
                        }
                    }

                    // Rung 4: 先呼叫 SetFocus，回傳 fallback_required 讓上層自動點擊 + WM_CHAR 輸入
                    let _ = elem.SetFocus();
                    Ok(UIAActionResult {
                        success: false,
                        target: target.to_string(),
                        resolved_name,
                        control_type,
                        subaction: subaction.to_string(),
                        mechanism: "fallback_type_required".into(),
                        fallback_required: true,
                        center_point,
                        bounding_rect,
                        verified_value: None,
                        message: "該編輯區不支援直接 SetValue (如 RichEdit/自繪輸入框)，已聚焦並準備降級由 WM_CHAR 注入".into(),
                    })
                }

                "toggle" => {
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_TogglePatternId) {
                        if let Ok(tog) = pat_unk.cast::<IUIAutomationTogglePattern>() {
                            tog.Toggle().map_err(|e| format!("Toggle 失敗: {:?}", e))?;
                            let st = tog.CurrentToggleState().map(toggle_state_to_string).ok();
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: "TogglePattern.Toggle".into(),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: st,
                                message: "已透過 TogglePattern.Toggle() 切換狀態".into(),
                            });
                        }
                    }
                    Ok(UIAActionResult {
                        success: false,
                        target: target.to_string(),
                        resolved_name,
                        control_type,
                        subaction: subaction.to_string(),
                        mechanism: "fallback_uv_required".into(),
                        fallback_required: true,
                        center_point,
                        bounding_rect,
                        verified_value: None,
                        message: "該控制項不支援 TogglePattern，準備使用 (u, v) 點擊切換".into(),
                    })
                }

                "expand" | "collapse" => {
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_ExpandCollapsePatternId) {
                        if let Ok(exp) = pat_unk.cast::<IUIAutomationExpandCollapsePattern>() {
                            if action_lower == "expand" {
                                exp.Expand().map_err(|e| format!("Expand 失敗: {:?}", e))?;
                            } else {
                                exp.Collapse().map_err(|e| format!("Collapse 失敗: {:?}", e))?;
                            }
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: format!("ExpandCollapsePattern.{}", if action_lower == "expand" { "Expand" } else { "Collapse" }),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: Some(if action_lower == "expand" { "Expanded".into() } else { "Collapsed".into() }),
                                message: format!("已執行 ExpandCollapsePattern::{}()", action_lower),
                            });
                        }
                    }
                    Ok(UIAActionResult {
                        success: false,
                        target: target.to_string(),
                        resolved_name,
                        control_type,
                        subaction: subaction.to_string(),
                        mechanism: "fallback_uv_required".into(),
                        fallback_required: true,
                        center_point,
                        bounding_rect,
                        verified_value: None,
                        message: "該控制項不支援 ExpandCollapsePattern，準備使用 (u, v) 點擊".into(),
                    })
                }

                "focus" => {
                    elem.SetFocus().map_err(|e| format!("SetFocus 失敗: {:?}", e))?;
                    Ok(UIAActionResult {
                        success: true,
                        target: target.to_string(),
                        resolved_name,
                        control_type,
                        subaction: subaction.to_string(),
                        mechanism: "IUIAutomationElement.SetFocus".into(),
                        fallback_required: false,
                        center_point,
                        bounding_rect,
                        verified_value: None,
                        message: "已成功聚焦目標元素".into(),
                    })
                }

                "scroll_into_view" => {
                    if let Ok(pat_unk) = elem.GetCurrentPattern(UIA_ScrollItemPatternId) {
                        if let Ok(scr) = pat_unk.cast::<IUIAutomationScrollItemPattern>() {
                            scr.ScrollIntoView().map_err(|e| format!("ScrollIntoView 失敗: {:?}", e))?;
                            return Ok(UIAActionResult {
                                success: true,
                                target: target.to_string(),
                                resolved_name,
                                control_type,
                                subaction: subaction.to_string(),
                                mechanism: "ScrollItemPattern.ScrollIntoView".into(),
                                fallback_required: false,
                                center_point,
                                bounding_rect,
                                verified_value: None,
                                message: "已將目標元素捲動至可見視野".into(),
                            });
                        }
                    }
                    Err("目標元素不支援 ScrollItemPattern".into())
                }

                _ => Err(format!("未知的 UIA 動作類型: {}", subaction)),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn find_element_by_ref_entry(
        &self,
        elem: &IUIAutomationElement,
        walker: &IUIAutomationTreeWalker,
        cache_req: &IUIAutomationCacheRequest,
        entry: &RefEntry,
        raw_depth: usize,
        occ_count: &mut usize,
        best_fallback: &mut Option<(IUIAutomationElement, i64)>,
    ) -> Result<Option<IUIAutomationElement>, String> {
        if raw_depth > MAX_RAW_DEPTH {
            return Ok(None);
        }

        let ctrl_id = elem
            .CachedControlType()
            .or_else(|_| elem.CurrentControlType())
            .unwrap_or(UIA_CONTROLTYPE_ID(0))
            .0;
        let ctrl_str = control_type_to_string(ctrl_id);
        let id_str = elem
            .CachedAutomationId()
            .or_else(|_| elem.CurrentAutomationId())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();
        let name_str = elem
            .CachedName()
            .or_else(|_| elem.CurrentName())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();

        if raw_depth > 0 && ctrl_str == entry.control_type {
            // 精確身份匹配 (Role + AutomationId + Name + 出現序號)
            let id_matches = entry.automation_id.is_empty() || id_str == entry.automation_id;
            let name_matches = name_str == entry.name;

            if id_matches && name_matches {
                if *occ_count == entry.occurrence_index {
                    return Ok(Some(elem.clone()));
                }
                *occ_count += 1;
            }

            // 幾何距離近鄰備援（若動態標籤文字稍微改變，取同類型且中心點距離 < 45px 最近者）
            if id_matches {
                if let Ok(r) = elem.CachedBoundingRectangle().or_else(|_| elem.CurrentBoundingRectangle()) {
                    let cx = r.left + (r.right - r.left) / 2;
                    let cy = r.top + (r.bottom - r.top) / 2;
                    let dx = (cx - entry.center_point[0]) as i64;
                    let dy = (cy - entry.center_point[1]) as i64;
                    let dist_sq = dx * dx + dy * dy;
                    if dist_sq <= 45 * 45 {
                        let update = match best_fallback {
                            Some((_, best_d)) => dist_sq < *best_d,
                            None => true,
                        };
                        if update {
                            *best_fallback = Some((elem.clone(), dist_sq));
                        }
                    }
                }
            }
        }

        let first_child_res = walker
            .GetFirstChildElementBuildCache(elem, cache_req)
            .or_else(|_| walker.GetFirstChildElement(elem));

        if let Ok(first_child) = first_child_res {
            let mut current = Some(first_child);
            while let Some(child) = current {
                let next = walker
                    .GetNextSiblingElementBuildCache(&child, cache_req)
                    .or_else(|_| walker.GetNextSiblingElement(&child))
                    .ok();
                if let Some(found) = self.find_element_by_ref_entry(
                    &child,
                    walker,
                    cache_req,
                    entry,
                    raw_depth + 1,
                    occ_count,
                    best_fallback,
                )? {
                    return Ok(Some(found));
                }
                current = next;
            }
        }

        Ok(None)
    }

    unsafe fn find_element_by_query(
        &self,
        elem: &IUIAutomationElement,
        walker: &IUIAutomationTreeWalker,
        cache_req: &IUIAutomationCacheRequest,
        query: &str,
        raw_depth: usize,
    ) -> Result<Option<IUIAutomationElement>, String> {
        if raw_depth > MAX_RAW_DEPTH {
            return Ok(None);
        }

        let name = elem
            .CachedName()
            .or_else(|_| elem.CurrentName())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();
        let id = elem
            .CachedAutomationId()
            .or_else(|_| elem.CurrentAutomationId())
            .unwrap_or_else(|_| BSTR::new())
            .to_string();

        if raw_depth > 0
            && (!name.is_empty()
                && (name.eq_ignore_ascii_case(query) || name.to_lowercase().contains(&query.to_lowercase()))
                || (!id.is_empty() && id.eq_ignore_ascii_case(query)))
        {
            return Ok(Some(elem.clone()));
        }

        let first_child_res = walker
            .GetFirstChildElementBuildCache(elem, cache_req)
            .or_else(|_| walker.GetFirstChildElement(elem));

        if let Ok(first_child) = first_child_res {
            let mut current = Some(first_child);
            while let Some(child) = current {
                let next = walker
                    .GetNextSiblingElementBuildCache(&child, cache_req)
                    .or_else(|_| walker.GetNextSiblingElement(&child))
                    .ok();
                if let Some(found) = self.find_element_by_query(&child, walker, cache_req, query, raw_depth + 1)? {
                    return Ok(Some(found));
                }
                current = next;
            }
        }

        Ok(None)
    }

    /// 舊版相容 API：根據目標名稱或 @Ref 觸發動作
    pub fn invoke_element_by_name(&self, hwnd_val: isize, target_name: &str) -> Result<bool, String> {
        let res = self.execute_uia_action(hwnd_val, target_name, "act", None)?;
        Ok(res.success)
    }
}

/// Visual Memory 快照結構體
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualMemorySnapshot {
    pub timestamp_ms: u64,
    pub step: usize,
    pub windows: Vec<DesktopWindowInfo>,
    pub uia_root: Option<UIATreeNode>,
}

/// 快照差異結果（含新增/移除/狀態與數值變更明細）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub added_windows: Vec<DesktopWindowInfo>,
    pub closed_windows: Vec<DesktopWindowInfo>,
    pub new_dialogs: Vec<DesktopWindowInfo>,
    pub added_elements: Vec<String>,
    pub removed_elements: Vec<String>,
    pub state_changed_elements: Vec<String>,
}

/// Visual Memory 管理器（保持最近 50 步感知記錄）
pub struct VisualMemory {
    snapshots: Mutex<Vec<VisualMemorySnapshot>>,
}

impl VisualMemory {
    pub fn new() -> Self {
        Self {
            snapshots: Mutex::new(Vec::new()),
        }
    }

    pub fn record_step(
        &self,
        windows: Vec<DesktopWindowInfo>,
        uia_root: Option<UIATreeNode>,
    ) -> VisualMemorySnapshot {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut lock = self.snapshots.lock().unwrap();
        let step = lock.len() + 1;
        let snap = VisualMemorySnapshot {
            timestamp_ms: now_ms,
            step,
            windows,
            uia_root,
        };
        lock.push(snap.clone());

        if lock.len() > 50 {
            lock.remove(0);
        }

        snap
    }

    /// 取得最新一步相對於前一步的差異比對（含控制項新增、消失與 Value/Toggle/Enabled 狀態變化）
    pub fn compute_diff_with_previous(&self) -> Option<SnapshotDiff> {
        let lock = self.snapshots.lock().unwrap();
        if lock.len() < 2 {
            return None;
        }

        let prev = &lock[lock.len() - 2];
        let curr = &lock[lock.len() - 1];

        let prev_hwnds: HashSet<isize> = prev.windows.iter().map(|w| w.hwnd).collect();
        let curr_hwnds: HashSet<isize> = curr.windows.iter().map(|w| w.hwnd).collect();

        let added_windows: Vec<DesktopWindowInfo> = curr
            .windows
            .iter()
            .filter(|w| !prev_hwnds.contains(&w.hwnd))
            .cloned()
            .collect();

        let closed_windows: Vec<DesktopWindowInfo> = prev
            .windows
            .iter()
            .filter(|w| !curr_hwnds.contains(&w.hwnd))
            .cloned()
            .collect();

        let new_dialogs: Vec<DesktopWindowInfo> = added_windows
            .iter()
            .filter(|w| {
                w.class_name == "#32770"
                    || w.title.contains("Save")
                    || w.title.contains("確認")
                    || w.title.contains("另存")
                    || w.title.contains("Error")
                    || w.title.contains("錯誤")
            })
            .cloned()
            .collect();

        let mut added_elements = Vec::new();
        let mut removed_elements = Vec::new();
        let mut state_changed_elements = Vec::new();

        if let (Some(p_tree), Some(c_tree)) = (&prev.uia_root, &curr.uia_root) {
            let p_flat = p_tree.flatten();
            let c_flat = c_tree.flatten();

            let mut p_map: HashMap<(String, String, String), UIATreeNode> = HashMap::new();
            for n in &p_flat {
                if !n.name.is_empty() || !n.id.is_empty() {
                    p_map.insert((n.control_type.clone(), n.id.clone(), n.name.clone()), n.clone());
                }
            }

            let mut c_map: HashMap<(String, String, String), UIATreeNode> = HashMap::new();
            for n in &c_flat {
                if !n.name.is_empty() || !n.id.is_empty() {
                    c_map.insert((n.control_type.clone(), n.id.clone(), n.name.clone()), n.clone());
                }
            }

            for (k, c_node) in &c_map {
                let ref_tag = c_node.ref_id.as_deref().unwrap_or("-");
                let label = if !c_node.name.is_empty() {
                    &c_node.name
                } else {
                    &c_node.id
                };
                if let Some(p_node) = p_map.get(k) {
                    if p_node.value != c_node.value {
                        state_changed_elements.push(format!(
                            "[{}] {} '{}': value {:?} -> {:?}",
                            ref_tag, c_node.control_type, label, p_node.value, c_node.value
                        ));
                    }
                    if p_node.toggle_state != c_node.toggle_state {
                        state_changed_elements.push(format!(
                            "[{}] {} '{}': toggle {:?} -> {:?}",
                            ref_tag, c_node.control_type, label, p_node.toggle_state, c_node.toggle_state
                        ));
                    }
                    if p_node.expand_state != c_node.expand_state {
                        state_changed_elements.push(format!(
                            "[{}] {} '{}': expand {:?} -> {:?}",
                            ref_tag, c_node.control_type, label, p_node.expand_state, c_node.expand_state
                        ));
                    }
                    if p_node.is_enabled != c_node.is_enabled {
                        state_changed_elements.push(format!(
                            "[{}] {} '{}': enabled {} -> {}",
                            ref_tag, c_node.control_type, label, p_node.is_enabled, c_node.is_enabled
                        ));
                    }
                } else {
                    added_elements.push(format!("[{}] {}: {}", ref_tag, c_node.control_type, label));
                }
            }

            for (k, p_node) in &p_map {
                if !c_map.contains_key(k) {
                    let label = if !p_node.name.is_empty() {
                        &p_node.name
                    } else {
                        &p_node.id
                    };
                    removed_elements.push(format!("{}: {}", p_node.control_type, label));
                }
            }
        }

        Some(SnapshotDiff {
            added_windows,
            closed_windows,
            new_dialogs,
            added_elements,
            removed_elements,
            state_changed_elements,
        })
    }
}
