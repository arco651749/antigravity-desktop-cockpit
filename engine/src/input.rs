use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::System::StationsAndDesktops::{GetThreadDesktop, SetThreadDesktop};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, KEYEVENTF_EXTENDEDKEY,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, VIRTUAL_KEY, MapVirtualKeyW, MAPVK_VK_TO_VSC, KEYBD_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, PostMessageW, SetForegroundWindow,
    SM_CXSCREEN, SM_CYSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
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

pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;

pub const MK_LBUTTON: usize = 0x0001;
pub const MK_RBUTTON: usize = 0x0002;
pub const MK_MBUTTON: usize = 0x0010;

#[link(name = "user32")]
extern "system" {
    fn ScreenToClient(hwnd: HWND, lppoint: *mut POINT) -> BOOL;
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
    fn IsWindowVisible(hwnd: HWND) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn GetCursorPos(lppoint: *mut POINT) -> BOOL;
    fn GetForegroundWindow() -> HWND;
    fn AttachThreadInput(idattach: u32, idattachto: u32, fattach: BOOL) -> BOOL;
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

    /// 主螢幕邊界空氣牆 (Primary Screen Cursor Fence)：
    /// 當虛擬第二螢幕開啟時，使用者實體桌面並沒有那台螢幕；若實體滑鼠游標意外滑過主螢幕右緣 (x >= primary_w)
    /// 且目前並非處於 12ms 原子點擊借用瞬間，立即將實體滑鼠游標攔截回主螢幕邊緣內，徹底杜絕「滑鼠卡進虛擬螢幕出不來」！
    pub fn enforce_primary_cursor_fence(&self) {
        if self.is_borrowing_cursor.load(Ordering::SeqCst) {
            return;
        }
        unsafe {
            let primary_w = GetSystemMetrics(SM_CXSCREEN).max(1280);
            let primary_h = GetSystemMetrics(SM_CYSCREEN).max(720);
            let mut cur = POINT::default();
            if GetCursorPos(&mut cur).as_bool() {
                if cur.x >= primary_w || cur.x < 0 || cur.y >= primary_h || cur.y < 0 {
                    let saved_pt = self.saved_host_cursor.lock().ok().and_then(|g| *g);
                    if let Some(pt) = saved_pt {
                        if pt.x >= 0 && pt.x < primary_w && pt.y >= 0 && pt.y < primary_h {
                            let _ = SetCursorPos(pt.x, pt.y);
                            return;
                        }
                    }
                    let clamped_x = cur.x.clamp(0, primary_w - 2);
                    let clamped_y = cur.y.clamp(0, primary_h - 2);
                    let _ = SetCursorPos(clamped_x, clamped_y);
                } else {
                    // 隨時記憶使用者在主螢幕上的最新合法游標座標
                    if let Ok(mut guard) = self.saved_host_cursor.lock() {
                        *guard = Some(cur);
                    }
                }
            }
        }
    }

    /// 在動用系統游標或切換焦點前，安全記錄使用者在「主螢幕」上的真實滑鼠位置與原焦點視窗
    pub unsafe fn snapshot_primary_state(&self, target_hwnd: HWND) {
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
        self.is_borrowing_cursor.store(true, Ordering::SeqCst);
        let primary_w = GetSystemMetrics(SM_CXSCREEN).max(1280);
        let primary_h = GetSystemMetrics(SM_CYSCREEN).max(720);

        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).as_bool() {
            // 僅當目前游標確實位於主螢幕內時才記錄，絕不被虛擬螢幕座標覆蓋！
            if pt.x >= 0 && pt.x < primary_w && pt.y >= 0 && pt.y < primary_h {
                if let Ok(mut guard) = self.saved_host_cursor.lock() {
                    *guard = Some(pt);
                }
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

    /// 僅歸還實體滑鼠游標至主螢幕原位（用於 down / move 瞬時投遞後，避免實體游標滯留於虛擬螢幕）
    pub unsafe fn restore_primary_cursor_only(&self) {
        let primary_w = GetSystemMetrics(SM_CXSCREEN).max(1280);
        let primary_h = GetSystemMetrics(SM_CYSCREEN).max(720);
        let saved_pt = self.saved_host_cursor.lock().ok().and_then(|g| *g);
        if let Some(pt) = saved_pt {
            let _ = SetCursorPos(pt.x, pt.y);
        } else {
            let mut cur = POINT::default();
            if GetCursorPos(&mut cur).as_bool() {
                if cur.x < 0 || cur.x >= primary_w || cur.y < 0 || cur.y >= primary_h {
                    let _ = SetCursorPos(primary_w / 2, primary_h / 2);
                }
            }
        }
        self.is_borrowing_cursor.store(false, Ordering::SeqCst);
    }

    /// 動作完成後，將實體滑鼠游標與鍵盤焦點無縫歸還回主螢幕原本的位置與視窗
    pub unsafe fn restore_primary_state(&self, target_hwnd: HWND) {
        self.restore_primary_cursor_only();

        // 2. 歸還鍵盤焦點至主螢幕原本使用的視窗 (如 Antigravity IDE)
        let saved_fg = self.saved_host_foreground.load(Ordering::SeqCst);
        if saved_fg != 0 && saved_fg != target_hwnd.0 as isize {
            let fg_hwnd = HWND(saved_fg as *mut _);
            let cur_fg = GetForegroundWindow();
            if cur_fg != fg_hwnd {
                let cur_tid = GetWindowThreadProcessId(cur_fg, None);
                let dst_tid = GetWindowThreadProcessId(fg_hwnd, None);
                if cur_tid != 0 && dst_tid != 0 && cur_tid != dst_tid {
                    let _ = AttachThreadInput(cur_tid, dst_tid, BOOL(1));
                    let _ = SetForegroundWindow(fg_hwnd);
                    let _ = AttachThreadInput(cur_tid, dst_tid, BOOL(0));
                } else {
                    let _ = SetForegroundWindow(fg_hwnd);
                }
            }
        }
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

    unsafe fn find_deepest_child(&self, hwnd: HWND, screen_pt: POINT) -> HWND {
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

    pub fn execute_mouse(
        &self,
        hwnd_val: isize,
        source: &str,
        action: &str,
        button: &str,
        u: f64,
        v: f64,
        delta: i32,
    ) -> Result<String, String> {
        let hwnd = HWND(hwnd_val as *mut _);
        ensure_desktop_for_hwnd(hwnd);
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 動作已禮讓暫停 1.5 秒".into());
        }

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            let mut win_rect = RECT::default();
            if GetWindowRect(hwnd, &mut win_rect).is_err() {
                return Err("無法獲取視窗外框 RECT".into());
            }

            let width = (win_rect.right - win_rect.left) as f64;
            let height = (win_rect.bottom - win_rect.top) as f64;

            // 將 normalized (u, v) 映射至物理螢幕絕對座標
            let screen_x = (win_rect.left as f64 + (u.clamp(0.0, 1.0) * width)).round() as i32;
            let screen_y = (win_rect.top as f64 + (v.clamp(0.0, 1.0) * height)).round() as i32;
            let screen_pt = POINT {
                x: screen_x,
                y: screen_y,
            };

            // 尋找包含此物理座標點的最深層葉子控制項 (例如 Notepad 的 RichEditD2DPT)
            let target_child = self.find_deepest_child(hwnd, screen_pt);
            self.last_clicked_child
                .store(target_child.0 as isize, Ordering::Relaxed);

            // 將螢幕物理座標轉換為該目標子控制項的局部 Client 座標
            let mut client_pt = screen_pt;
            let _ = ScreenToClient(target_child, &mut client_pt);

            let sub_x = client_pt.x;
            let sub_y = client_pt.y;
            let l_param = LPARAM(((sub_y as u16 as isize) << 16) | (sub_x as u16 as isize));

            if let Ok(mut guard) = self.ai_cursor.lock() {
                *guard = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            }

            if action == "click" {
                let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1 /* WA_ACTIVE */), LPARAM(0));
                let _ = PostMessageW(target_child, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                let (wm_down, wm_up, mk) = match button {
                    "right" => (WM_RBUTTONDOWN, WM_RBUTTONUP, WPARAM(MK_RBUTTON)),
                    "middle" => (WM_MBUTTONDOWN, WM_MBUTTONUP, WPARAM(MK_MBUTTON)),
                    _ => (WM_LBUTTONDOWN, WM_LBUTTONUP, WPARAM(MK_LBUTTON)),
                };
                let _ = PostMessageW(target_child, wm_down, mk, l_param);
                std::thread::sleep(Duration::from_millis(10));
                let _ = PostMessageW(target_child, wm_up, WPARAM(0), l_param);
                return Ok(format!(
                    "背景原子點擊成功: HWND=0x{:X}, btn={}, x={}, y={}",
                    target_child.0 as usize, button, sub_x, sub_y
                ));
            }

            let (wm_msg, w_param) = match action {
                "down" => {
                    // 發送 WM_ACTIVATE 告知頂層視窗內部處於激活狀態，同時發送 WM_SETFOCUS 聚焦目標子控制項
                    let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1 /* WA_ACTIVE */), LPARAM(0));
                    let _ = PostMessageW(target_child, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                    match button {
                        "right" => (WM_RBUTTONDOWN, WPARAM(MK_RBUTTON)),
                        "middle" => (WM_MBUTTONDOWN, WPARAM(MK_MBUTTON)),
                        _ => (WM_LBUTTONDOWN, WPARAM(MK_LBUTTON)),
                    }
                }
                "up" => match button {
                    "right" => (WM_RBUTTONUP, WPARAM(0)),
                    "middle" => (WM_MBUTTONUP, WPARAM(0)),
                    _ => (WM_LBUTTONUP, WPARAM(0)),
                },
                "wheel" => {
                    let wp = ((delta as isize) << 16) as usize;
                    (WM_MOUSEWHEEL, WPARAM(wp))
                }
                _ => {
                    // 拖曳移動時保持按鈕按壓旗標 (如 Blender 中鍵 3D 旋轉)
                    let mk = match button {
                        "left" => MK_LBUTTON,
                        "right" => MK_RBUTTON,
                        "middle" => MK_MBUTTON,
                        _ => 0,
                    };
                    (WM_MOUSEMOVE, WPARAM(mk))
                }
            };

            // 純背景靜默投遞：游標不跳躍、視窗不彈出！
            let _ = PostMessageW(target_child, wm_msg, w_param, l_param);

            Ok(format!(
                "背景滑鼠投遞成功: HWND=0x{:X}, act={}, btn={}, x={}, y={}",
                target_child.0 as usize, action, button, sub_x, sub_y
            ))
        }
    }

    pub fn execute_key(
        &self,
        hwnd_val: isize,
        source: &str,
        action: &str,
        vk_code: u16,
        unicode_char: Option<char>,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Result<String, String> {
        let hwnd = HWND(hwnd_val as *mut _);
        ensure_desktop_for_hwnd(hwnd);
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 鍵盤動作暫停".into());
        }

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, GA_ROOT};
            let last_child = self.last_clicked_child.load(Ordering::Relaxed);
            let root = HWND(hwnd_val as *mut _);
            let target = if last_child != 0 {
                let ch = HWND(last_child as *mut _);
                if IsWindow(ch).as_bool() && GetAncestor(ch, GA_ROOT) == root {
                    ch
                } else {
                    root
                }
            } else {
                root
            };

            let is_up = action.eq_ignore_ascii_case("up");

            // 1. Alt 系統組合鍵處理 (例如 Alt+F 展開選單)
            if alt && vk_code != 18 /* VK_MENU */ {
                let lp = LPARAM(1 | (1 << 29) | if is_up { (1 << 30) | (1 << 31) } else { 0 });
                let msg = if is_up { 0x0105 /* WM_SYSKEYUP */ } else { 0x0104 /* WM_SYSKEYDOWN */ };
                let _ = PostMessageW(root, msg, WPARAM(vk_code as usize), lp);
                if target != root {
                    let _ = PostMessageW(target, msg, WPARAM(vk_code as usize), lp);
                }
                return Ok(format!("Alt 快捷鍵投遞: VK={}, act={}", vk_code, action));
            }

            // 2. Ctrl 快捷鍵處理 (Ctrl+A, Ctrl+C, Ctrl+V, Ctrl+Z, Ctrl+X, Ctrl+F 等)
            if ctrl && vk_code != 17 /* VK_CONTROL */ {
                if !is_up {
                    match vk_code {
                        65 => { // Ctrl+A (全選)
                            let _ = PostMessageW(target, 0x00B1 /* EM_SETSEL */, WPARAM(0), LPARAM(-1));
                            return Ok("Ctrl+A (全選) 投遞成功".into());
                        }
                        67 => { // Ctrl+C (複製)
                            let _ = PostMessageW(target, 0x0301 /* WM_COPY */, WPARAM(0), LPARAM(0));
                            return Ok("Ctrl+C (複製) 投遞成功".into());
                        }
                        86 => { // Ctrl+V (貼上)
                            let _ = PostMessageW(target, 0x0302 /* WM_PASTE */, WPARAM(0), LPARAM(0));
                            return Ok("Ctrl+V (貼上) 投遞成功".into());
                        }
                        88 => { // Ctrl+X (剪下)
                            let _ = PostMessageW(target, 0x0300 /* WM_CUT */, WPARAM(0), LPARAM(0));
                            return Ok("Ctrl+X (剪下) 投遞成功".into());
                        }
                        90 => { // Ctrl+Z (復原)
                            let _ = PostMessageW(target, 0x0304 /* WM_UNDO */, WPARAM(0), LPARAM(0));
                            return Ok("Ctrl+Z (復原) 投遞成功".into());
                        }
                        _ => {}
                    }
                }
                // 發送底層 KEYDOWN 訊息
                let msg = if is_up { WM_KEYUP } else { WM_KEYDOWN };
                let _ = PostMessageW(target, msg, WPARAM(vk_code as usize), LPARAM(0));
                let _ = PostMessageW(root, msg, WPARAM(vk_code as usize), LPARAM(0));

                if !is_up && vk_code >= 65 && vk_code <= 90 {
                    // Win32 標準控制碼 (Ctrl+A = 1, Ctrl+F = 6)
                    let ctrl_ascii = (vk_code - 64) as usize;
                    let _ = PostMessageW(target, WM_CHAR, WPARAM(ctrl_ascii), LPARAM(0));
                }
                return Ok(format!("Ctrl 快捷鍵投遞: VK={}, act={}", vk_code, action));
            }

            // 3. 一般文字輸入或功能鍵 (Enter, Backspace, Tab, 方向鍵, 純字母)
            if let Some(ch) = unicode_char {
                if !is_up {
                    // 純文字輸入只發送單一 WM_CHAR，防止與視窗訊息迴圈 TranslateMessage 重複疊加產生多重字元
                    let _ = PostMessageW(target, WM_CHAR, WPARAM(ch as usize), LPARAM(1));
                }
                // is_up 釋放時無需對純文字再次發送多餘訊息
            } else {
                let msg = if is_up { WM_KEYUP } else { WM_KEYDOWN };
                let _ = PostMessageW(target, msg, WPARAM(vk_code as usize), LPARAM(0));
            }

            Ok(format!(
                "背景按鍵投遞成功: Target=0x{:X}, act={}, vk={}",
                target.0 as usize, action, vk_code
            ))
        }
    }

    /// 直通模式 (Direct Mode - 支援虛擬 AI 游標與原子級游標/焦點借還)
    /// 透過 Windows 原生 SendInput API 進行核心級輸入直通，操作完畢即刻將實體游標與鍵盤焦點歸還主螢幕
    pub fn execute_mouse_direct(
        &self,
        hwnd_val: isize,
        source: &str,
        action: &str,
        button: &str,
        u: f64,
        v: f64,
        delta: i32,
    ) -> Result<String, String> {
        crate::capture::ensure_default_desktop();
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 動作已禮讓暫停 1.5 秒".into());
        }

        // 更新內部 AI 虛擬游標座標
        if let Ok(mut guard) = self.ai_cursor.lock() {
            *guard = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        }

        // 若為 AI 發出的純移動 (move)，僅移動畫布上的「🤖 虛擬 AI 游標」，完全不動實體滑鼠與焦點！
        if action == "move" && !is_user {
            return Ok(format!(
                "AI 虛擬游標移動至 ({:.4}, {:.4}) [主畫面實體滑鼠 0 位移]",
                u, v
            ));
        }

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() {
                return Err("無法獲取視窗外框 RECT".into());
            }

            let width = (r.right - r.left) as f64;
            let height = (r.bottom - r.top) as f64;
            let screen_x = (r.left as f64 + (u.clamp(0.0, 1.0) * width)).round() as i32;
            let screen_y = (r.top as f64 + (v.clamp(0.0, 1.0) * height)).round() as i32;

            // 換算虛擬多螢幕絕對座標 (RustDesk 規範)
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let norm_x = (((screen_x - vx) * 65535) / vw) as i32;
            let norm_y = (((screen_y - vy) * 65535) / vh) as i32;

            // 動作開始前，安全快照主螢幕上的真實游標座標與原前景視窗 (絕不被虛擬螢幕座標覆蓋)
            self.snapshot_primary_state(hwnd);

            // 激活目標視窗並同步系統游標至目標點
            let _ = SetForegroundWindow(hwnd);
            let _ = SetCursorPos(screen_x, screen_y);

            let (dw_down, dw_up) = match button {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            if action == "click" {
                // 原子級瞬移點擊：單一流程內完成 Move+Down -> 12ms -> Up -> 12ms -> 無縫歸還主螢幕滑鼠與鍵盤焦點
                let down_inputs = [
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: dw_down,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                ];
                SendInput(&down_inputs, size_of::<INPUT>() as i32);
                std::thread::sleep(Duration::from_millis(12));

                let up_inputs = [INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: norm_x,
                            dy: norm_y,
                            mouseData: 0,
                            dwFlags: dw_up | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                }];
                SendInput(&up_inputs, size_of::<INPUT>() as i32);
                std::thread::sleep(Duration::from_millis(12));

                self.restore_primary_state(hwnd);

                return Ok(format!(
                    "原子直通點擊完成並已歸還主畫面滑鼠/焦點: HWND=0x{:X}, btn={}, x={}, y={}",
                    hwnd.0 as usize, button, screen_x, screen_y
                ));
            }

            let mut inputs = Vec::new();

            match action {
                "down" => {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: dw_down,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                "up" => {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: dw_up | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                "move" => {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                "wheel" => {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: (delta * 120) as u32,
                                dwFlags: MOUSEEVENTF_WHEEL,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                _ => {}
            }

            if !inputs.is_empty() {
                SendInput(&inputs, size_of::<INPUT>() as i32);
            }

            // 無論是 down / move / up / wheel，等待 10ms 讓目標應用訊息迴圈接收後，
            // 100% 將實體滑鼠游標歸還至主螢幕原位置，徹底杜絕實體游標滯留或卡死在虛擬螢幕！
            std::thread::sleep(Duration::from_millis(10));
            if action == "up" || action == "wheel" || !is_user {
                self.restore_primary_state(hwnd);
            } else {
                self.restore_primary_cursor_only();
            }

            Ok(format!(
                "直通模式滑鼠投遞成功: HWND=0x{:X}, act={}, btn={}, x={}, y={}",
                hwnd.0 as usize, action, button, screen_x, screen_y
            ))
        }
    }

    /// 直通模式按鍵 (Direct Key Mode - 支援主畫面焦點快照與歸還)
    pub fn execute_key_direct(
        &self,
        hwnd_val: isize,
        source: &str,
        action: &str,
        vk_code: u16,
        unicode_char: Option<char>,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Result<String, String> {
        crate::capture::ensure_default_desktop();
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 鍵盤動作暫停".into());
        }

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            self.snapshot_primary_state(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let is_up = action.eq_ignore_ascii_case("up");

            // 仿 RustDesk 物理硬體掃描碼生成機制 (同時提供 wVk 與 OEM wScan，並自動判定擴充鍵標記)
            let make_hardware_key = |vk: u16, up: bool| -> INPUT {
                let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
                let is_extended = match vk {
                    33..=46 | 91..=93 | 111 | 144 | 163 | 165 => true, // PageUp/Down, Home, End, Arrows, Delete, Win 等
                    _ => scan >> 8 == 0xE0 || scan >> 8 == 0xE1,
                };
                let mut flags = KEYBD_EVENT_FLAGS::default();
                if is_extended {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                if up {
                    flags |= KEYEVENTF_KEYUP;
                }
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            wScan: scan,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                }
            };

            let mut inputs = Vec::new();

            if !is_up {
                if ctrl && vk_code != 17 {
                    inputs.push(make_hardware_key(0x11, false)); // VK_CONTROL
                }
                if alt && vk_code != 18 {
                    inputs.push(make_hardware_key(0x12, false)); // VK_MENU (Alt)
                }
                if shift && vk_code != 16 && unicode_char.is_none() {
                    inputs.push(make_hardware_key(0x10, false)); // VK_SHIFT
                }
            }

            if let Some(ch) = unicode_char.filter(|_| !ctrl && !alt) {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch as u16,
                            dwFlags: KEYEVENTF_UNICODE | if is_up { KEYEVENTF_KEYUP } else { Default::default() },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            } else {
                inputs.push(make_hardware_key(vk_code, is_up));
            }

            if is_up {
                if ctrl && vk_code != 17 {
                    inputs.push(make_hardware_key(0x11, true)); // VK_CONTROL
                }
                if alt && vk_code != 18 {
                    inputs.push(make_hardware_key(0x12, true)); // VK_MENU (Alt)
                }
                if shift && vk_code != 16 && unicode_char.is_none() {
                    inputs.push(make_hardware_key(0x10, true)); // VK_SHIFT
                }
            }

            if !inputs.is_empty() {
                SendInput(&inputs, size_of::<INPUT>() as i32);
            }

            // 當 AI 按鍵釋放 (up) 時，等待 12ms 讓目標訊息迴圈消化快捷鍵，隨即將滑鼠與鍵盤焦點歸還回主畫面視窗
            if is_up && !is_user && vk_code != 16 && vk_code != 17 && vk_code != 18 {
                std::thread::sleep(Duration::from_millis(12));
                self.restore_primary_state(hwnd);
            } else {
                self.restore_primary_cursor_only();
            }

            Ok(format!(
                "直通模式按鍵投遞成功: HWND=0x{:X}, act={}, vk={}",
                hwnd.0 as usize, action, vk_code
            ))
        }
    }

    /// 向目標視窗或其編輯子控制項逐字注入文字 (純背景 WM_CHAR，支援多語系，零焦點干擾)
    pub fn execute_text(&self, hwnd_val: isize, text: &str) -> Result<String, String> {
        let hwnd = HWND(hwnd_val as *mut _);
        ensure_desktop_for_hwnd(hwnd);
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, GA_ROOT};
            let last_child = self.last_clicked_child.load(Ordering::Relaxed);
            let valid_child = if last_child != 0 {
                let ch = HWND(last_child as *mut _);
                if IsWindow(ch).as_bool() && GetAncestor(ch, GA_ROOT) == hwnd {
                    Some(ch)
                } else {
                    None
                }
            } else {
                None
            };

            let target = if let Some(ch) = valid_child {
                ch
            } else {
                let mut children: Vec<HWND> = Vec::new();
                let ptr = &mut children as *mut _ as isize;
                let _ = EnumChildWindows(hwnd, Some(Self::collect_children_cb), LPARAM(ptr));
                let edit_child = children.iter().find(|&&ch| {
                    let mut cbuf = [0u16; 128];
                    let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(ch, &mut cbuf);
                    let cls = String::from_utf16_lossy(&cbuf[..len.max(0) as usize]);
                    cls.starts_with("Scintilla") || cls.eq_ignore_ascii_case("Edit")
                });
                edit_child.copied().unwrap_or(hwnd)
            };

            let _ = PostMessageW(target, WM_SETFOCUS, WPARAM(0), LPARAM(0));
            for ch in text.chars() {
                let _ = PostMessageW(target, WM_CHAR, WPARAM(ch as usize), LPARAM(1));
            }
            Ok(format!("成功向 0x{:X} 注入文字 (共 {} 字元)", target.0 as usize, text.chars().count()))
        }
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
