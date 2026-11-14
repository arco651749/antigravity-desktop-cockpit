use super::types::{
    control_type_to_string, expand_state_to_string, is_actionable_node, is_transparent_wrapper,
    toggle_state_to_string, RefEntry, RefSnapshotMap, UIATreeNode,
};
use super::{UIAEngine, MAX_RAW_DEPTH};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest,
    IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationRangeValuePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationTreeWalker,
    IUIAutomationValuePattern, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ClassNamePropertyId,
    UIA_ControlTypePropertyId, UIA_ExpandCollapseExpandCollapseStatePropertyId,
    UIA_ExpandCollapsePatternId, UIA_HasKeyboardFocusPropertyId, UIA_InvokePatternId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId,
    UIA_IsPasswordPropertyId, UIA_LegacyIAccessiblePatternId, UIA_NamePropertyId,
    UIA_RangeValuePatternId, UIA_RangeValueValuePropertyId, UIA_ScrollItemPatternId,
    UIA_SelectionItemIsSelectedPropertyId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
    UIA_ToggleToggleStatePropertyId, UIA_ValuePatternId, UIA_ValueValuePropertyId,
    UIA_CONTROLTYPE_ID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_NULL,
};

/// 探測視窗訊息迴圈超時毫秒數
const PUMP_PROBE_TIMEOUT_MS: u32 = 1500;

/// UIA 連線與交易超時毫秒數
const UIA_CONNECTION_TIMEOUT_MS: u32 = 2000;
const UIA_TRANSACTION_TIMEOUT_MS: u32 = 15000;

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
    pub(super) fn probe_window_responsive(hwnd_val: isize) -> Result<(), String> {
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
    pub(super) unsafe fn build_cache_request(&self) -> Result<IUIAutomationCacheRequest, String> {
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
}
