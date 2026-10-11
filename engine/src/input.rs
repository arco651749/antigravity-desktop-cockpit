use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    mouse_event, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetSystemMetrics, PostMessageW, SetForegroundWindow, SM_CXSCREEN, SM_CYSCREEN,
};

pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;

pub struct InputArbiter {
    last_user_input_ms: AtomicU64,
}

impl InputArbiter {
    pub fn new() -> Self {
        Self {
            last_user_input_ms: AtomicU64::new(0),
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
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 動作已禮讓暫停 1.5 秒".into());
        }

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            let mut client_rect = RECT::default();
            if GetClientRect(hwnd, &mut client_rect).is_err() {
                return Err("無法獲取視窗 Client 區域".into());
            }

            let mut origin = POINT {
                x: client_rect.left,
                y: client_rect.top,
            };
            let _ = ClientToScreen(hwnd, &mut origin);

            let width = (client_rect.right - client_rect.left) as f64;
            let height = (client_rect.bottom - client_rect.top) as f64;

            let client_x = (u.clamp(0.0, 1.0) * width) as i32;
            let client_y = (v.clamp(0.0, 1.0) * height) as i32;

            let screen_x = origin.x as f64 + (u.clamp(0.0, 1.0) * width);
            let screen_y = origin.y as f64 + (v.clamp(0.0, 1.0) * height);

            let screen_w = GetSystemMetrics(SM_CXSCREEN) as f64;
            let screen_h = GetSystemMetrics(SM_CYSCREEN) as f64;

            let abs_x = ((screen_x / screen_w) * 65535.0).round() as i32;
            let abs_y = ((screen_y / screen_h) * 65535.0).round() as i32;

            let mut flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE;

            match action {
                "down" => match button {
                    "right" => flags |= MOUSEEVENTF_RIGHTDOWN,
                    "middle" => flags |= MOUSEEVENTF_MIDDLEDOWN,
                    _ => flags |= MOUSEEVENTF_LEFTDOWN,
                },
                "up" => match button {
                    "right" => flags |= MOUSEEVENTF_RIGHTUP,
                    "middle" => flags |= MOUSEEVENTF_MIDDLEUP,
                    _ => flags |= MOUSEEVENTF_LEFTUP,
                },
                "wheel" => {
                    flags |= MOUSEEVENTF_WHEEL;
                }
                "move" => {}
                _ => return Err(format!("未知的滑鼠動作: {}", action)),
            }

            let mouse_input = MOUSEINPUT {
                dx: abs_x,
                dy: abs_y,
                mouseData: delta as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            };

            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 { mi: mouse_input },
            };

            if action == "down" {
                let _ = SetForegroundWindow(hwnd);
            }

            // 1. 優先嘗試 Win32 SendInput
            let sent = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            if sent == 0 {
                let _err = GetLastError();
                // 2. 若遇 UIPI 權限阻擋，自動回退調用 mouse_event
                mouse_event(flags, abs_x, abs_y, delta, 0);

                // 3. 額外輔助透過 PostMessage 直接注入目標視窗訊息佇列
                let l_param = LPARAM(((client_y as isize) << 16) | (client_x as isize & 0xFFFF));
                let wm_msg = match action {
                    "down" => match button {
                        "right" => WM_RBUTTONDOWN,
                        "middle" => WM_MBUTTONDOWN,
                        _ => WM_LBUTTONDOWN,
                    },
                    "up" => match button {
                        "right" => WM_RBUTTONUP,
                        "middle" => WM_MBUTTONUP,
                        _ => WM_LBUTTONUP,
                    },
                    _ => WM_MOUSEMOVE,
                };
                let _ = PostMessageW(hwnd, wm_msg, WPARAM(0), l_param);
            }

            Ok(format!(
                "滑鼠事件已注入: action={}, button={}, x={}, y={}",
                action, button, screen_x as i32, screen_y as i32
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
    ) -> Result<String, String> {
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

            let key_input = if let Some(ch) = unicode_char {
                let mut flags = KEYEVENTF_UNICODE;
                if is_up {
                    flags |= KEYEVENTF_KEYUP;
                }
                KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: ch as u16,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                }
            } else {
                let mut flags = windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0);
                if is_up {
                    flags |= KEYEVENTF_KEYUP;
                }
                KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk_code),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                }
            };

            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: key_input },
            };

            let sent = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            if sent == 0 {
                // 回退至 PostMessage
                if let Some(ch) = unicode_char {
                    let _ = PostMessageW(hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
                } else {
                    let msg = if is_up { WM_KEYUP } else { WM_KEYDOWN };
                    let _ = PostMessageW(hwnd, msg, WPARAM(vk_code as usize), LPARAM(0));
                }
            }

            Ok(format!("鍵盤事件已注入: action={}, vk={}", action, vk_code))
        }
    }
}
