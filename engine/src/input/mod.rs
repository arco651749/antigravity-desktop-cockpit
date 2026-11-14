//! 零干擾輸入仲裁、游標借用 RAII 守衛、鍵鼠注入與 Win32 剪貼簿模組。

mod clipboard;
mod keyboard;
mod mouse;

pub use clipboard::{get_clipboard_text, set_clipboard_text};

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::System::StationsAndDesktops::{GetThreadDesktop, SetThreadDesktop};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, GetWindowThreadProcessId, PostMessageW, SetForegroundWindow,
};

pub fn ensure_desktop_for_hwnd(hwnd: HWND) {
    unsafe {
        let tid = GetWindowThreadProcessId(hwnd, None);
        if tid != 0 {
            if let Ok(hdesk) = GetThreadDesktop(tid) {
                let _ = SetThreadDesktop(hdesk);
                return;
            }
        }
        crate::capture::ensure_default_desktop();
    }
}

pub(super) const WM_SETFOCUS: u32 = 0x0007;
#[allow(dead_code)]
pub(super) const WM_CLOSE: u32 = 0x0010;
pub(super) const WM_MOUSEMOVE: u32 = 0x0200;
pub(super) const WM_LBUTTONDOWN: u32 = 0x0201;
pub(super) const WM_LBUTTONUP: u32 = 0x0202;
pub(super) const WM_RBUTTONDOWN: u32 = 0x0204;
pub(super) const WM_RBUTTONUP: u32 = 0x0205;
pub(super) const WM_MBUTTONDOWN: u32 = 0x0207;
pub(super) const WM_MBUTTONUP: u32 = 0x0208;
pub(super) const WM_MOUSEWHEEL: u32 = 0x020A;
pub(super) const WM_KEYDOWN: u32 = 0x0100;
pub(super) const WM_KEYUP: u32 = 0x0101;
pub(super) const WM_CHAR: u32 = 0x0102;

pub(super) const MK_LBUTTON: usize = 0x0001;
pub(super) const MK_RBUTTON: usize = 0x0002;
pub(super) const MK_MBUTTON: usize = 0x0010;

#[link(name = "user32")]
extern "system" {
    pub(super) fn ScreenToClient(hwnd: HWND, lppoint: *mut POINT) -> BOOL;
    pub(super) fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
    pub(super) fn IsWindowVisible(hwnd: HWND) -> BOOL;
    pub(super) fn SetCursorPos(x: i32, y: i32) -> BOOL;
    pub(super) fn GetCursorPos(lppoint: *mut POINT) -> BOOL;
    pub(super) fn GetForegroundWindow() -> HWND;
    pub(super) fn AttachThreadInput(idattach: u32, idattachto: u32, fattach: BOOL) -> BOOL;
}

/// RAII 游標與焦點借用守衛：確保離開作用域時（包含 panic 或提早返回）必定歸還主螢幕狀態
pub struct CursorBorrowGuard<'a> {
    arbiter: &'a InputArbiter,
    target_hwnd: HWND,
    restore_focus: bool,
}

impl<'a> CursorBorrowGuard<'a> {
    /// 建立守衛並立即快照主螢幕游標與前景視窗
    pub unsafe fn new(arbiter: &'a InputArbiter, target_hwnd: HWND, restore_focus: bool) -> Self {
        arbiter.snapshot_primary_state(target_hwnd);
        Self {
            arbiter,
            target_hwnd,
            restore_focus,
        }
    }

    /// 動態調整釋放守衛時是否一併歸還前景視窗焦點
    pub fn set_restore_focus(&mut self, restore_focus: bool) {
        self.restore_focus = restore_focus;
    }
}

impl Drop for CursorBorrowGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            if self.restore_focus {
                self.arbiter.restore_primary_state(self.target_hwnd);
            } else {
                self.arbiter.restore_primary_cursor_only();
            }
        }
    }
}

pub struct InputArbiter {
    last_user_input_ms: AtomicU64,
    last_clicked_child: AtomicIsize,
    saved_host_cursor: std::sync::Mutex<Option<POINT>>,
    saved_host_foreground: AtomicIsize,
    pub is_borrowing_cursor: AtomicBool,
    pub ai_cursor: std::sync::Mutex<(f64, f64)>,
}

impl InputArbiter {
    pub fn new() -> Self {
        Self {
            last_user_input_ms: AtomicU64::new(0),
            last_clicked_child: AtomicIsize::new(0),
            saved_host_cursor: std::sync::Mutex::new(None),
            saved_host_foreground: AtomicIsize::new(0),
            is_borrowing_cursor: AtomicBool::new(false),
            ai_cursor: std::sync::Mutex::new((0.5, 0.5)),
        }
    }

    /// 建立 RAII 游標與焦點借用守衛，離開作用域時自動還原主螢幕狀態
    pub unsafe fn borrow_cursor(&self, target_hwnd: HWND, restore_focus: bool) -> CursorBorrowGuard<'_> {
        CursorBorrowGuard::new(self, target_hwnd, restore_focus)
    }

    /// 在動用系統游標或切換焦點前，安全記錄使用者在主螢幕上的真實滑鼠位置與原焦點視窗
    pub unsafe fn snapshot_primary_state(&self, target_hwnd: HWND) {
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
        self.is_borrowing_cursor.store(true, Ordering::SeqCst);

        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).as_bool() {
            if let Ok(mut guard) = self.saved_host_cursor.lock() {
                *guard = Some(pt);
            }
        }

        let fg = GetForegroundWindow();
        if !fg.0.is_null() {
            let fg_root = GetAncestor(fg, GA_ROOT);
            let target_root = GetAncestor(target_hwnd, GA_ROOT);
            // 僅當目前前景視窗不是虛擬螢幕上的受控目標視窗時，才記為應歸還的主畫面焦點視窗
            if fg != target_hwnd && fg_root != target_root && !crate::capture::WindowCapturer::is_window_on_virtual_display(fg.0 as isize) {
                self.saved_host_foreground.store(fg.0 as isize, Ordering::SeqCst);
            }
        }
    }

    /// 僅歸還實體滑鼠游標至主螢幕原位（用於直通模式瞬時借還，不設任何邊界限制）
    pub unsafe fn restore_primary_cursor_only(&self) {
        let saved_pt = self.saved_host_cursor.lock().ok().and_then(|mut g| g.take());
        if let Some(pt) = saved_pt {
            let _ = SetCursorPos(pt.x, pt.y);
        }
        self.is_borrowing_cursor.store(false, Ordering::SeqCst);
    }

    /// 動作完成後，將實體滑鼠游標與鍵盤焦點無縫歸還回主螢幕原本的位置與視窗
    pub unsafe fn restore_primary_state(&self, target_hwnd: HWND) {
        self.restore_primary_cursor_only();

        // 2. 歸還鍵盤焦點至主螢幕原本使用的視窗 (如 Antigravity IDE)
        let mut saved_fg = self.saved_host_foreground.load(Ordering::SeqCst);
        if saved_fg == 0 || saved_fg == target_hwnd.0 as isize || crate::capture::WindowCapturer::is_window_on_virtual_display(saved_fg) {
            if let Some(ide_hwnd) = Self::find_ide_window() {
                saved_fg = ide_hwnd.0 as isize;
            }
        }

        if saved_fg != 0 && saved_fg != target_hwnd.0 as isize {
            let fg_hwnd = HWND(saved_fg as *mut _);
            let cur_fg = GetForegroundWindow();
            if cur_fg != fg_hwnd {
                let my_tid = windows::Win32::System::Threading::GetCurrentThreadId();
                let fg_tid = GetWindowThreadProcessId(cur_fg, None);
                let dst_tid = GetWindowThreadProcessId(fg_hwnd, None);

                if fg_tid != 0 && fg_tid != my_tid {
                    let _ = AttachThreadInput(my_tid, fg_tid, BOOL(1));
                }
                if dst_tid != 0 && dst_tid != my_tid {
                    let _ = AttachThreadInput(my_tid, dst_tid, BOOL(1));
                }

                let _ = SetForegroundWindow(fg_hwnd);

                if dst_tid != 0 && dst_tid != my_tid {
                    let _ = AttachThreadInput(my_tid, dst_tid, BOOL(0));
                }
                if fg_tid != 0 && fg_tid != my_tid {
                    let _ = AttachThreadInput(my_tid, fg_tid, BOOL(0));
                }
            }
        }
    }

    unsafe fn find_ide_window() -> Option<HWND> {
        let mut result: Option<HWND> = None;
        let ptr = &mut result as *mut Option<HWND> as isize;
        unsafe extern "system" fn enum_ide_cb(h: HWND, lp: LPARAM) -> BOOL {
            if windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(h).as_bool() {
                if crate::capture::is_ide_window(h) {
                    let res_ref = &mut *(lp.0 as *mut Option<HWND>);
                    *res_ref = Some(h);
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumWindows(Some(enum_ide_cb), LPARAM(ptr));
        result
    }

    fn current_timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }

    pub fn record_user_activity(&self) {
        self.last_user_input_ms
            .store(Self::current_timestamp_ms(), Ordering::SeqCst);
    }

    pub fn can_agent_act(&self, lockout_ms: u64) -> bool {
        let last = self.last_user_input_ms.load(Ordering::SeqCst);
        let now = Self::current_timestamp_ms();
        now.saturating_sub(last) >= lockout_ms
    }

    pub(super) unsafe fn find_deepest_child(&self, hwnd: HWND, screen_pt: POINT) -> HWND {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let init_area = (r.right - r.left) * (r.bottom - r.top);

        let mut data = (screen_pt, hwnd, init_area);
        let ptr = &mut data as *mut (POINT, HWND, i32) as isize;

        let _ = EnumChildWindows(hwnd, Some(Self::enum_child_cb), LPARAM(ptr));
        data.1
    }

    unsafe extern "system" fn enum_child_cb(child: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut (POINT, HWND, i32));
        if IsWindowVisible(child).as_bool() {
            let mut r = RECT::default();
            if GetWindowRect(child, &mut r).is_ok() {
                if data.0.x >= r.left
                    && data.0.x < r.right
                    && data.0.y >= r.top
                    && data.0.y < r.bottom
                {
                    let area = (r.right - r.left) * (r.bottom - r.top);
                    if area > 0 && area <= data.2 {
                        data.2 = area;
                        data.1 = child;
                    }
                }
            }
        }
        BOOL(1)
    }

    unsafe extern "system" fn collect_children_cb(child: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(child);
        BOOL(1)
    }


    /// 向目標視窗發送關閉訊息 (純背景 WM_CLOSE)
    pub fn close_window(&self, hwnd_val: isize) -> Result<String, String> {
        let hwnd = HWND(hwnd_val as *mut _);
        ensure_desktop_for_hwnd(hwnd);
        unsafe {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            Ok(format!("成功向 0x{:X} 發送 WM_CLOSE 關閉訊息", hwnd.0 as usize))
        }
    }

}
