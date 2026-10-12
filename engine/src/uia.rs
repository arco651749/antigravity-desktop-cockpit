use crate::desktop::DesktopWindowInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationTreeWalker, UIA_CONTROLTYPE_ID, UIA_InvokePatternId,
    UIA_ValuePatternId, UIA_TogglePatternId, UIA_SelectionItemPatternId,
};

/// UIA 語意樹節點
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIATreeNode {
    pub id: String,
    pub name: String,
    pub class_name: String,
    pub control_type: String,
    pub bounding_rect: [i32; 4], // [left, top, width, height]
    pub center_point: [i32; 2],  // [center_x, center_y]
    pub supported_patterns: Vec<String>,
    pub children: Vec<UIATreeNode>,
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

pub struct UIAEngine {
    automation: IUIAutomation,
}

unsafe impl Send for UIAEngine {}
unsafe impl Sync for UIAEngine {}

impl UIAEngine {
    pub fn new() -> Result<Self, String> {
        unsafe {
            // 初始化多執行緒 COM 套間
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("無法建立 IUIAutomation 實例: {:?}", e))?;
            Ok(Self { automation })
        }
    }

    /// 從指定的 HWND 掃描完整的 UIA 語意階層樹
    pub fn fetch_tree_from_hwnd(&self, hwnd_val: isize, max_depth: usize) -> Result<UIATreeNode, String> {
        unsafe {
            let elem = self
                .automation
                .ElementFromHandle(HWND(hwnd_val as *mut _))
                .map_err(|e| format!("ElementFromHandle 失敗 (0x{:X}): {:?}", hwnd_val, e))?;

            let walker = self
                .automation
                .ControlViewWalker()
                .map_err(|e| format!("ControlViewWalker 失敗: {:?}", e))?;

            self.build_node_recursive(&elem, &walker, 0, max_depth)
        }
    }

    unsafe fn build_node_recursive(
        &self,
        elem: &IUIAutomationElement,
        walker: &IUIAutomationTreeWalker,
        depth: usize,
        max_depth: usize,
    ) -> Result<UIATreeNode, String> {
        let name = elem.CurrentName().unwrap_or_else(|_| BSTR::new()).to_string();
        let class_name = elem.CurrentClassName().unwrap_or_else(|_| BSTR::new()).to_string();
        let id = elem.CurrentAutomationId().unwrap_or_else(|_| BSTR::new()).to_string();
        let ctrl_type_id = elem.CurrentControlType().unwrap_or(UIA_CONTROLTYPE_ID(0)).0;
        let control_type = control_type_to_string(ctrl_type_id);

        let rect = elem.CurrentBoundingRectangle().unwrap_or(RECT::default());
        let w = (rect.right - rect.left).max(0);
        let h = (rect.bottom - rect.top).max(0);
        let center_x = rect.left + w / 2;
        let center_y = rect.top + h / 2;

        // 偵測支援的 Pattern
        let mut supported_patterns = Vec::new();
        if elem.GetCurrentPattern(UIA_InvokePatternId).is_ok() {
            supported_patterns.push("Invoke".to_string());
        }
        if elem.GetCurrentPattern(UIA_ValuePatternId).is_ok() {
            supported_patterns.push("Value".to_string());
        }
        if elem.GetCurrentPattern(UIA_TogglePatternId).is_ok() {
            supported_patterns.push("Toggle".to_string());
        }
        if elem.GetCurrentPattern(UIA_SelectionItemPatternId).is_ok() {
            supported_patterns.push("SelectionItem".to_string());
        }

        let mut children = Vec::new();
        if depth < max_depth {
            if let Ok(first_child) = walker.GetFirstChildElement(elem) {
                let mut current = Some(first_child);
                while let Some(child_elem) = current {
                    if let Ok(child_node) = self.build_node_recursive(&child_elem, walker, depth + 1, max_depth) {
                        children.push(child_node);
                    }
                    current = walker.GetNextSiblingElement(&child_elem).ok();
                }
            }
        }

        Ok(UIATreeNode {
            id,
            name,
            class_name,
            control_type,
            bounding_rect: [rect.left, rect.top, w, h],
            center_point: [center_x, center_y],
            supported_patterns,
            children,
        })
    }

    /// 根據目標名稱直接觸發 InvokePattern (靜默呼叫按鈕，如 "No"、"Cancel"、"Save")
    pub fn invoke_element_by_name(&self, hwnd_val: isize, target_name: &str) -> Result<bool, String> {
        unsafe {
            let root = self
                .automation
                .ElementFromHandle(HWND(hwnd_val as *mut _))
                .map_err(|e| format!("ElementFromHandle 失敗: {:?}", e))?;

            let walker = self
                .automation
                .ControlViewWalker()
                .map_err(|e| format!("ControlViewWalker 失敗: {:?}", e))?;

            self.invoke_recursive(&root, &walker, target_name)
        }
    }

    unsafe fn invoke_recursive(
        &self,
        elem: &IUIAutomationElement,
        walker: &IUIAutomationTreeWalker,
        target_name: &str,
    ) -> Result<bool, String> {
        let name = elem.CurrentName().unwrap_or_else(|_| BSTR::new()).to_string();
        if name.eq_ignore_ascii_case(target_name) || name.contains(target_name) {
            if let Ok(pattern_unk) = elem.GetCurrentPattern(UIA_InvokePatternId) {
                if let Ok(invoke_pattern) = pattern_unk.cast::<IUIAutomationInvokePattern>() {
                    println!("[UIA] 成功找到目標 '{}' 並執行 InvokePattern::Invoke()！", name);
                    invoke_pattern.Invoke().map_err(|e| format!("Invoke 失敗: {:?}", e))?;
                    return Ok(true);
                }
            }
        }

        if let Ok(first_child) = walker.GetFirstChildElement(elem) {
            let mut current = Some(first_child);
            while let Some(child) = current {
                if self.invoke_recursive(&child, walker, target_name)? {
                    return Ok(true);
                }
                current = walker.GetNextSiblingElement(&child).ok();
            }
        }

        Ok(false)
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

/// 快照差異結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub added_windows: Vec<DesktopWindowInfo>,
    pub closed_windows: Vec<DesktopWindowInfo>,
    pub new_dialogs: Vec<DesktopWindowInfo>,
    pub added_elements: Vec<String>,
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

        // 保留最近 50 步
        if lock.len() > 50 {
            lock.remove(0);
        }

        snap
    }

    /// 取得最新一步相對於前一步的差異比對
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
            .filter(|w| w.class_name == "#32770" || w.title.contains("Save") || w.title.contains("確認"))
            .cloned()
            .collect();

        let mut added_elements = Vec::new();
        if let (Some(p_tree), Some(c_tree)) = (&prev.uia_root, &curr.uia_root) {
            let p_names: HashSet<String> = p_tree.flatten().into_iter().map(|n| n.name).collect();
            for item in c_tree.flatten() {
                if !item.name.is_empty() && !p_names.contains(&item.name) {
                    added_elements.push(format!("{}: {}", item.control_type, item.name));
                }
            }
        }

        Some(SnapshotDiff {
            added_windows,
            closed_windows,
            new_dialogs,
            added_elements,
        })
    }
}
