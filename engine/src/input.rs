use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::System::StationsAndDesktops::{GetThreadDesktop, SetThreadDesktop};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, VIRTUAL_KEY,
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
}

pub struct InputArbiter {
    last_user_input_ms: AtomicU64,
    last_clicked_child: AtomicIsize,
}

impl InputArbiter {
    pub fn new() -> Self {
        Self {
            last_user_input_ms: AtomicU64::new(0),
            last_clicked_child: AtomicIsize::new(0),
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

    /// 直通模式 (Direct Mode - 參考 RustDesk 實作)
    /// 透過 Windows 原生 SendInput API 進行核心級輸入直通，相容 WinUI 3 XAML、選單、分頁等所有應用
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

            // 記錄當前物理游標位置 (儲存使用者在實體主螢幕的滑鼠座標)
            let mut host_cursor = POINT::default();
            let has_host_cursor = GetCursorPos(&mut host_cursor).as_bool();

            // 點擊前激活目標視窗並同步系統物理游標至目標點 (確保 WinUI 3 與標準控制項雙重捕獲)
            let _ = SetForegroundWindow(hwnd);
            let _ = SetCursorPos(screen_x, screen_y);

            let (dw_down, dw_up) = match button {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            let mut inputs = Vec::new();

            match action {
                "down" => {
                    // 移動至座標並按下
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

            // 核心修復：游標復位時機
            // 1. 若為按下 (down) 或移動 (move)，游標應維持在目標視窗內，讓 Chromium / WinUI 3 / 3D 軟體完整捕獲點擊
            // 2. 僅在釋放 (up) 或滾輪 (wheel) 時，給予 15ms 讓目標應用訊息迴圈消化完畢後，才將物理游標無縫復位回主螢幕
            if (action == "up" || action == "wheel") && has_host_cursor {
                std::thread::sleep(Duration::from_millis(15));
                let primary_w = GetSystemMetrics(SM_CXSCREEN);
                let primary_h = GetSystemMetrics(SM_CYSCREEN);
                if host_cursor.x >= 0 && host_cursor.x < primary_w && host_cursor.y >= 0 && host_cursor.y < primary_h {
                    let _ = SetCursorPos(host_cursor.x, host_cursor.y);
                } else {
                    let _ = SetCursorPos(primary_w / 2, primary_h / 2);
                }
            }

            Ok(format!(
                "直通模式滑鼠投遞成功: HWND=0x{:X}, act={}, btn={}, x={}, y={}",
                hwnd.0 as usize, action, button, screen_x, screen_y
            ))
        }
    }

    /// 直通模式按鍵 (Direct Key Mode)
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
            let _ = SetForegroundWindow(hwnd);
            let is_up = action.eq_ignore_ascii_case("up");

            let mut inputs = Vec::new();

            // 若非單獨按下修飾鍵本體，但在按下主鍵時檢測到組合鍵狀態，先確保修飾鍵在 Windows 佇列中處於按下狀態
            if !is_up {
                if ctrl && vk_code != 17 {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x11), // VK_CONTROL
                                wScan: 0,
                                dwFlags: Default::default(),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                if alt && vk_code != 18 {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x12), // VK_MENU (Alt)
                                wScan: 0,
                                dwFlags: Default::default(),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                // 注意：若已有直接的 unicode_char (如 'A', '!'), 無需額外壓住 Shift，避免 Windows 產生二次修飾
                if shift && vk_code != 16 && unicode_char.is_none() {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x10), // VK_SHIFT
                                wScan: 0,
                                dwFlags: Default::default(),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
            }

            if let Some(ch) = unicode_char {
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
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk_code),
                            wScan: 0,
                            dwFlags: if is_up { KEYEVENTF_KEYUP } else { Default::default() },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }

            // 釋放主要按鍵時，一併釋放對應的合成修飾鍵狀態，防呆避免卡鍵
            if is_up {
                if ctrl && vk_code != 17 {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x11), // VK_CONTROL
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                if alt && vk_code != 18 {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x12), // VK_MENU
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                if shift && vk_code != 16 && unicode_char.is_none() {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x10), // VK_SHIFT
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
            }

            if !inputs.is_empty() {
                SendInput(&inputs, size_of::<INPUT>() as i32);
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
