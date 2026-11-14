use super::types::{
    control_type_to_string, normalize_ref_key, toggle_state_to_string, RefEntry, UIAActionResult,
};
use super::{UIAEngine, MAX_RAW_DEPTH};
use windows::core::{Interface, BSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Accessibility::{
    ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded, IUIAutomationCacheRequest,
    IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationRangeValuePattern,
    IUIAutomationScrollItemPattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
    IUIAutomationTreeWalker, IUIAutomationValuePattern, UIA_ExpandCollapsePatternId,
    UIA_InvokePatternId, UIA_LegacyIAccessiblePatternId, UIA_RangeValuePatternId,
    UIA_ScrollItemPatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
    UIA_CONTROLTYPE_ID,
};

impl UIAEngine {
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
