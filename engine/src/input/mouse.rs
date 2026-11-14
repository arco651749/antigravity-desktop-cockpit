use super::{
    ensure_desktop_for_hwnd, InputArbiter, ScreenToClient, MK_LBUTTON, MK_MBUTTON, MK_RBUTTON,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS,
};
use std::mem::size_of;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, GetWindowRect, PostMessageW, SetForegroundWindow, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

impl InputArbiter {
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

            let is_on_virtual = crate::capture::WindowCapturer::is_window_on_virtual_display(hwnd_val);
            let virtual_mon = crate::monitor::MonitorManager::list_monitors()
                .into_iter()
                .find(|m| m.is_virtual_candidate);

            let (screen_x, screen_y) = if let Some(vm) = virtual_mon {
                if is_on_virtual || vm.x > 0 {
                    let sx = vm.x as f64 + (u.clamp(0.0, 1.0) * vm.width as f64);
                    let sy = vm.y as f64 + (v.clamp(0.0, 1.0) * vm.height as f64);
                    (sx.round() as i32, sy.round() as i32)
                } else {
                    let width = (win_rect.right - win_rect.left).max(1) as f64;
                    let height = (win_rect.bottom - win_rect.top).max(1) as f64;
                    let sx = win_rect.left as f64 + (u.clamp(0.0, 1.0) * width);
                    let sy = win_rect.top as f64 + (v.clamp(0.0, 1.0) * height);
                    (sx.round() as i32, sy.round() as i32)
                }
            } else {
                let width = (win_rect.right - win_rect.left).max(1) as f64;
                let height = (win_rect.bottom - win_rect.top).max(1) as f64;
                let sx = win_rect.left as f64 + (u.clamp(0.0, 1.0) * width);
                let sy = win_rect.top as f64 + (v.clamp(0.0, 1.0) * height);
                (sx.round() as i32, sy.round() as i32)
            };

            let screen_pt = POINT {
                x: screen_x,
                y: screen_y,
            };

            // 尋找包含此物理座標點的最深層葉子控制項 (例如 Notepad 的 RichEditD2DPT)
            // 拖曳 (Drag) 狀態保護：若在拖曳中 (move / up)，鎖定 down 時命中的同一個 target_child，
            // 確保所有後續 move 與釋放 up 均投遞給原控制項，防止滑鼠滑出控制項時丟失 up 或座標錯位！
            let target_child = if action == "move" || action == "up" {
                let saved = self.last_clicked_child.load(Ordering::Relaxed);
                if saved != 0 {
                    let ch = HWND(saved as *mut _);
                    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, GA_ROOT};
                    if IsWindow(ch).as_bool() && (ch == hwnd || GetAncestor(ch, GA_ROOT) == hwnd) {
                        ch
                    } else {
                        self.find_deepest_child(hwnd, screen_pt)
                    }
                } else {
                    self.find_deepest_child(hwnd, screen_pt)
                }
            } else {
                let ch = self.find_deepest_child(hwnd, screen_pt);
                if action == "down" {
                    self.last_clicked_child.store(ch.0 as isize, Ordering::Relaxed);
                }
                ch
            };

            if action == "up" {
                self.last_clicked_child.store(0, Ordering::Relaxed);
            }

            // 將螢幕物理座標轉換為該目標子控制項的局部 Client 座標
            let mut client_pt = screen_pt;
            let _ = ScreenToClient(target_child, &mut client_pt);

            let sub_x = client_pt.x;
            let sub_y = client_pt.y;
            let l_param = if action == "wheel" {
                // WM_MOUSEWHEEL 的 lParam 依 Win32 標準必須是螢幕絕對座標 (screen coordinates)！
                LPARAM(((screen_y as u16 as isize) << 16) | (screen_x as u16 as isize))
            } else {
                LPARAM(((sub_y as u16 as isize) << 16) | (sub_x as u16 as isize))
            };

            if let Ok(mut guard) = self.ai_cursor.lock() {
                *guard = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            }

            if action == "wheel" {
                let wp = ((delta as isize) << 16) as usize;
                let _ = PostMessageW(target_child, WM_MOUSEWHEEL, WPARAM(wp), l_param);
                if target_child != hwnd {
                    let _ = PostMessageW(hwnd, WM_MOUSEWHEEL, WPARAM(wp), l_param);
                }
                return Ok(format!(
                    "背景滾輪投遞成功: HWND=0x{:X}, delta={}, x={}, y={}",
                    target_child.0 as usize, delta, screen_x, screen_y
                ));
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
            if windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() {
                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_RESTORE);
                std::thread::sleep(Duration::from_millis(50));
            }
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() {
                return Err("無法獲取視窗外框 RECT".into());
            }

            let is_on_virtual = crate::capture::WindowCapturer::is_window_on_virtual_display(hwnd_val);
            let virtual_mon = crate::monitor::MonitorManager::list_monitors()
                .into_iter()
                .find(|m| m.is_virtual_candidate);

            let (screen_x, screen_y) = if let Some(vm) = virtual_mon {
                if is_on_virtual || vm.x > 0 {
                    let sx = vm.x as f64 + (u.clamp(0.0, 1.0) * vm.width as f64);
                    let sy = vm.y as f64 + (v.clamp(0.0, 1.0) * vm.height as f64);
                    (sx.round() as i32, sy.round() as i32)
                } else {
                    let width = (r.right - r.left).max(1) as f64;
                    let height = (r.bottom - r.top).max(1) as f64;
                    let sx = r.left as f64 + (u.clamp(0.0, 1.0) * width);
                    let sy = r.top as f64 + (v.clamp(0.0, 1.0) * height);
                    (sx.round() as i32, sy.round() as i32)
                }
            } else {
                let width = (r.right - r.left).max(1) as f64;
                let height = (r.bottom - r.top).max(1) as f64;
                let sx = r.left as f64 + (u.clamp(0.0, 1.0) * width);
                let sy = r.top as f64 + (v.clamp(0.0, 1.0) * height);
                (sx.round() as i32, sy.round() as i32)
            };

            // 換算虛擬多螢幕絕對座標 (RustDesk 規範)
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let norm_x = (((screen_x - vx) * 65535) / vw) as i32;
            let norm_y = (((screen_y - vy) * 65535) / vh) as i32;

            // 直通模式：直接激活目標視窗，將實體滑鼠送至第二螢幕，取消自動歸還焦點
            let _ = SetForegroundWindow(hwnd);

            let (dw_down, dw_up) = match button {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            let mut inputs = Vec::new();

            match action {
                "click" => {
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
                                dwFlags: dw_down | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
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
                                dwFlags: dw_up | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
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
                                dwFlags: dw_down | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
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
                "wheel" => {
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
                                dx: 0,
                                dy: 0,
                                mouseData: delta as u32,
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

            Ok(format!(
                "直通模式滑鼠投遞成功: HWND=0x{:X}, act={}, btn={}, x={}, y={}",
                hwnd.0 as usize, action, button, screen_x, screen_y
            ))
        }
    }
}
