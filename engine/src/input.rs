use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, PostMessageW};

pub const WM_SETFOCUS: u32 = 0x0007;
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
        crate::capture::ensure_default_desktop();
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
    ) -> Result<String, String> {
        crate::capture::ensure_default_desktop();
        let is_user = source.eq_ignore_ascii_case("user");
        if is_user {
            self.record_user_activity();
        } else if !self.can_agent_act(1500) {
            return Err("PAUSED_BY_USER: 人類正在操作中，AI 鍵盤動作暫停".into());
        }

        unsafe {
            // 優先發送給最後點擊的子控制項 (如 RichEditD2DPT)，若無則發給主視窗
            let last_child = self.last_clicked_child.load(Ordering::Relaxed);
            let target = if last_child != 0 {
                HWND(last_child as *mut _)
            } else {
                HWND(hwnd_val as *mut _)
            };

            let is_up = action.eq_ignore_ascii_case("up");

            if let Some(ch) = unicode_char {
                if !is_up {
                    let _ = PostMessageW(target, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
                }
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
}
