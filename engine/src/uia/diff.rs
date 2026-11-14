use super::types::UIATreeNode;
use crate::desktop::DesktopWindowInfo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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
