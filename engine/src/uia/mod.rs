//! Windows UI Automation (UIA) 語意引擎、@Ref 快照、動作派發與視覺記憶差異模組。

mod actions;
mod diff;
mod traversal;
mod types;

pub use diff::{SnapshotDiff, VisualMemory, VisualMemorySnapshot};
pub use types::{normalize_ref_key, RefEntry, RefSnapshotMap, UIAActionResult, UIATreeNode};

pub(super) const MAX_RAW_DEPTH: usize = 36;

use std::sync::Mutex;
use windows::Win32::UI::Accessibility::IUIAutomation;

pub struct UIAEngine {
    pub(super) automation: IUIAutomation,
    pub(super) ref_map: Mutex<RefSnapshotMap>,
}

unsafe impl Send for UIAEngine {}
unsafe impl Sync for UIAEngine {}
