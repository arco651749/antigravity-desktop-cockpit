use crate::desktop::DesktopManager;
use crate::protocol::{ClientCommand, ServerEvent};
use crate::server::StreamServer;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

impl StreamServer {
    pub(super) async fn handle_input_command(
        &self,
        cmd: ClientCommand,
        ctrl_tx: &mpsc::Sender<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
        ClientCommand::RestorePrimaryFocus => {
            let target_hwnd = self.resolve_target_hwnd(None);
            unsafe {
                self.arbiter.restore_primary_state(windows::Win32::Foundation::HWND(target_hwnd as *mut _));
            }
            let resp = ServerEvent {
                event: "primary_focus_restored".into(),
                status: "ok".into(),
                message: Some("實體滑鼠與鍵盤焦點已歸還主螢幕".into()),
                data: None,
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
        ClientCommand::Mouse {
            source,
            mode,
            subaction,
            button,
            u,
            v,
            delta,
        } => {
            let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
            let (target_hwnd, sub_u, sub_v) = if is_secret {
                let desk_name = self.isolated_desktop.read().unwrap().clone();
                let target_pid = self.isolated_pid.load(Ordering::Relaxed);
                if let Ok(windows) = DesktopManager::list_windows_in_desktop(&desk_name) {
                    let (screen_w, screen_h) = DesktopManager::get_desktop_metrics();
                    let pt_x = (u * screen_w as f64).round() as i32;
                    let pt_y = (v * screen_h as f64).round() as i32;

                    let mut sorted = windows;
                    if target_pid > 0 {
                        sorted.retain(|w| w.pid == target_pid);
                    }
                    // 面積由小到大排序，優先命中浮動對話框或按鈕
                    sorted.sort_by_key(|w| (w.width as i64) * (w.height as i64));

                    let hit = sorted.iter().find(|w| {
                        pt_x >= w.x && pt_x < w.x + w.width && pt_y >= w.y && pt_y < w.y + w.height
                    });

                    if let Some(w) = hit {
                        let su = if w.width > 0 { ((pt_x - w.x) as f64 / w.width as f64).clamp(0.0, 1.0) } else { 0.5 };
                        let sv = if w.height > 0 { ((pt_y - w.y) as f64 / w.height as f64).clamp(0.0, 1.0) } else { 0.5 };
                        (w.hwnd, su, sv)
                    } else {
                        (self.target_hwnd.load(Ordering::Relaxed), u, v)
                    }
                } else {
                    (self.target_hwnd.load(Ordering::Relaxed), u, v)
                }
            } else {
                (self.target_hwnd.load(Ordering::Relaxed), u, v)
            };

            // 廣播 AI 虛擬游標移動或點擊事件給所有前端面板，讓畫布即時渲染「🤖 AI 游標」動畫
            if !source.eq_ignore_ascii_case("user") {
                let bcast_evt = ServerEvent {
                    event: "ai_cursor".into(),
                    status: "ok".into(),
                    message: None,
                    data: Some(serde_json::json!({
                        "u": sub_u,
                        "v": sub_v,
                        "subaction": subaction,
                        "button": button,
                        "source": source,
                        "focus_restored": subaction == "click" || subaction == "up" || subaction == "wheel"
                    })),
                };
                if let Ok(json_str) = serde_json::to_string(&bcast_evt) {
                    let _ = self.event_tx.send(json_str);
                }
            }

            if target_hwnd != 0 {
                let effective_mode = if is_secret { "background" } else { &mode };
                let res = if effective_mode.eq_ignore_ascii_case("background") {
                    self.arbiter.execute_mouse(
                        target_hwnd, &source, &subaction, &button, sub_u, sub_v, delta,
                    )
                } else {
                    self.arbiter.execute_mouse_direct(
                        target_hwnd, &source, &subaction, &button, sub_u, sub_v, delta,
                    )
                };
                match res {
                    Ok(msg) => {
                        self.needs_refresh.store(true, Ordering::Relaxed);
                        if subaction != "move" || !source.eq_ignore_ascii_case("user") {
                            let resp = ServerEvent {
                                event: "input_ack".into(),
                                status: "ok".into(),
                                message: Some(msg),
                                data: None,
                            };
                            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                        }
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "input_rejected".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "input_rejected".into(),
                    status: "error".into(),
                    message: Some("尚未選取目標視窗，請先在上方選單選擇視窗".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::Key {
            source,
            mode,
            subaction,
            vk,
            char,
            ctrl,
            alt,
            shift,
        } => {
            let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
            let target_hwnd = self.resolve_target_hwnd(None);

            if !source.eq_ignore_ascii_case("user") && subaction == "down" {
                let (cu, cv) = self.arbiter.ai_cursor.lock().map(|g| *g).unwrap_or((0.5, 0.5));
                let bcast_evt = ServerEvent {
                    event: "ai_cursor".into(),
                    status: "ok".into(),
                    message: None,
                    data: Some(serde_json::json!({
                        "u": cu,
                        "v": cv,
                        "subaction": "key",
                        "vk": vk,
                        "char": char,
                        "ctrl": ctrl,
                        "alt": alt,
                        "shift": shift,
                        "focus_restored": true
                    })),
                };
                if let Ok(json_str) = serde_json::to_string(&bcast_evt) {
                    let _ = self.event_tx.send(json_str);
                }
            }

            if target_hwnd != 0 {
                let effective_mode = if is_secret { "background" } else { &mode };
                let res = if effective_mode.eq_ignore_ascii_case("background") {
                    self.arbiter.execute_key(
                        target_hwnd, &source, &subaction, vk, char, ctrl, alt, shift,
                    )
                } else {
                    self.arbiter.execute_key_direct(
                        target_hwnd, &source, &subaction, vk, char, ctrl, alt, shift,
                    )
                };
                match res {
                    Ok(msg) => {
                        self.needs_refresh.store(true, Ordering::Relaxed);
                        let resp = ServerEvent {
                            event: "input_ack".into(),
                            status: "ok".into(),
                            message: Some(msg),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "input_rejected".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "input_rejected".into(),
                    status: "error".into(),
                    message: Some("尚未選取目標視窗，請先在上方選單選擇視窗".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::TypeText { text, hwnd } => {
            let target_hwnd = self.resolve_target_hwnd(hwnd);

            let (cu, cv) = self.arbiter.ai_cursor.lock().map(|g| *g).unwrap_or((0.5, 0.5));
            let bcast_evt = ServerEvent {
                event: "ai_cursor".into(),
                status: "ok".into(),
                message: None,
                data: Some(serde_json::json!({
                    "u": cu,
                    "v": cv,
                    "subaction": "type",
                    "text_preview": text.chars().take(18).collect::<String>(),
                    "focus_restored": true
                })),
            };
            if let Ok(json_str) = serde_json::to_string(&bcast_evt) {
                let _ = self.event_tx.send(json_str);
            }

            if target_hwnd != 0 {
                match self.arbiter.execute_text(target_hwnd, &text) {
                    Ok(msg) => {
                        self.needs_refresh.store(true, Ordering::Relaxed);
                        let resp = ServerEvent {
                            event: "text_typed".into(),
                            status: "ok".into(),
                            message: Some(msg),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "text_typed".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "text_typed".into(),
                    status: "error".into(),
                    message: Some("未找到目標視窗 HWND".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::GetClipboard => {
            match crate::input::get_clipboard_text() {
                Ok(text) => {
                    let resp = ServerEvent::ok(
                        "clipboard",
                        None,
                        Some(serde_json::json!({ "text": text })),
                    );
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent::err("clipboard", e);
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::SetClipboard { text } => {
            match crate::input::set_clipboard_text(&text) {
                Ok(_) => {
                    let resp = ServerEvent::ok(
                        "clipboard_set",
                        Some("剪貼簿已更新".into()),
                        None,
                    );
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent::err("clipboard_set", e);
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
            _ => unreachable!(),
        }
        Ok(())
    }
}
