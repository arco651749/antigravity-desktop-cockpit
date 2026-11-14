use super::{
    ensure_desktop_for_hwnd, EnumChildWindows, InputArbiter, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    WM_SETFOCUS,
};
use std::mem::size_of;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SetForegroundWindow};

impl InputArbiter {
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
                // Ctrl+Shift+<字母> 語意與 Ctrl+<字母> 不同 (如 Ctrl+Shift+Z = Redo)，不可套用簡化訊息
                if !is_up && !shift {
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

                if !is_up && (65..=90).contains(&vk_code) {
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
            let mut _borrow_guard = self.borrow_cursor(hwnd, false);
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

            // 當 AI 按鍵釋放 (up) 時，等待 12ms 讓目標訊息迴圈消化快捷鍵，並標記守衛在離開作用域時歸還主畫面焦點
            if is_up && !is_user && vk_code != 16 && vk_code != 17 && vk_code != 18 {
                std::thread::sleep(Duration::from_millis(12));
                _borrow_guard.set_restore_focus(true);
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


}
