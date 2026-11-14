use crate::capture::WindowEnumerator;
use crate::desktop::DesktopManager;
use crate::monitor::MonitorManager;
use crate::protocol::{ClientCommand, ServerEvent};
use crate::server::StreamServer;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::mpsc;

impl StreamServer {
    pub async fn dispatch_command(
        &self,
        cmd: ClientCommand,
        ctrl_tx: &mpsc::Sender<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
        ClientCommand::LaunchIsolated { app, args } => {
            let desk_name = self.isolated_desktop.read().unwrap().clone();
            let app_name = app.as_deref().unwrap_or("notepad++");
            let args_vec: Vec<&str> = args.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect()).unwrap_or_default();

            let launch_res = if app_name.eq_ignore_ascii_case("notepad++") || app_name.eq_ignore_ascii_case("notepad") {
                DesktopManager::launch_notepad_plus_plus(&desk_name)
            } else {
                DesktopManager::launch_in_desktop(&desk_name, app_name, &args_vec)
            };

            match launch_res {
                Ok(pid) => {
                    self.isolated_pid.store(pid, Ordering::SeqCst);
                    *self.capture_mode.write().unwrap() = "secret_desktop".to_string();
                    self.needs_refresh.store(true, Ordering::Relaxed);
                    let resp = ServerEvent {
                        event: "app_launched".into(),
                        status: "ok".into(),
                        message: Some(format!("成功在隔離桌面 '{}' 啟動 (PID: {})", desk_name, pid)),
                        data: Some(serde_json::json!({
                            "pid": pid,
                            "desktop": desk_name,
                            "mode": "secret_desktop"
                        })),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "app_launched".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::GetIsolatedWindows => {
            let desk_name = self.isolated_desktop.read().unwrap().clone();
            match DesktopManager::list_windows_in_desktop(&desk_name) {
                Ok(wins) => {
                    let resp = ServerEvent {
                        event: "isolated_windows".into(),
                        status: "ok".into(),
                        message: None,
                        data: Some(serde_json::to_value(wins)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "isolated_windows".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::GetUIATree { hwnd, depth } => {
            let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
            let desk_name = self.isolated_desktop.read().unwrap().clone();
            let target_hwnd = self.resolve_target_hwnd(hwnd);

            if target_hwnd != 0 {
                match self.uia.fetch_tree_from_hwnd(target_hwnd, depth.unwrap_or(8)) {
                    Ok(tree) => {
                        let wins = if is_secret {
                            DesktopManager::list_windows_in_desktop(&desk_name).unwrap_or_default()
                        } else {
                            WindowEnumerator::list_windows().into_iter().map(|w| crate::desktop::DesktopWindowInfo {
                                hwnd: w.hwnd,
                                pid: 0,
                                title: w.title,
                                class_name: String::new(),
                                width: w.width,
                                height: w.height,
                                x: 0,
                                y: 0,
                            }).collect()
                        };
                        let _ = self.visual_memory.record_step(wins, Some(tree.clone()));
                        let resp = ServerEvent {
                            event: "uia_tree".into(),
                            status: "ok".into(),
                            message: None,
                            data: Some(serde_json::to_value(tree)?),
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "uia_tree".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "uia_tree".into(),
                    status: "error".into(),
                    message: Some("未找到可掃描的有效視窗 HWND".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::UIAInvoke { hwnd, name } => {
            let target_hwnd = self.resolve_target_hwnd(hwnd);

            if target_hwnd != 0 {
                match self.uia.invoke_element_by_name(target_hwnd, &name) {
                    Ok(invoked) => {
                        self.needs_refresh.store(true, Ordering::Relaxed);
                        let resp = ServerEvent {
                            event: "uia_invoked".into(),
                            status: if invoked { "ok".into() } else { "not_found".into() },
                            message: Some(format!("UIA Invoke('{}') 結果: {}", name, invoked)),
                            data: Some(serde_json::json!({ "invoked": invoked, "name": name })),
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "uia_invoked".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "uia_invoked".into(),
                    status: "error".into(),
                    message: Some("未找到目標視窗 HWND".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::UIAAction {
            hwnd,
            target,
            subaction,
            value,
        } => {
            let target_hwnd = self.resolve_target_hwnd(hwnd);
            match self.uia.execute_uia_action(
                target_hwnd,
                &target,
                &subaction,
                value.as_deref(),
            ) {
                Ok(act_res) => {
                    self.needs_refresh.store(true, Ordering::Relaxed);
                    let status_str = if act_res.success {
                        "ok"
                    } else if act_res.fallback_required {
                        "fallback_required"
                    } else {
                        "error"
                    };
                    let resp = ServerEvent {
                        event: "uia_action_done".into(),
                        status: status_str.into(),
                        message: Some(act_res.message.clone()),
                        data: Some(serde_json::to_value(act_res)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "uia_action_done".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::GetVisualDiff => {
            let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
            let desk_name = self.isolated_desktop.read().unwrap().clone();
            let wins = if is_secret {
                DesktopManager::list_windows_in_desktop(&desk_name).unwrap_or_default()
            } else {
                WindowEnumerator::list_windows().into_iter().map(|w| crate::desktop::DesktopWindowInfo {
                    hwnd: w.hwnd,
                    pid: 0,
                    title: w.title,
                    class_name: String::new(),
                    width: w.width,
                    height: w.height,
                    x: 0,
                    y: 0,
                }).collect()
            };
            let target_hwnd = self.resolve_target_hwnd(None);
            let tree = if target_hwnd != 0 {
                self.uia.fetch_tree_from_hwnd(target_hwnd, 8).ok()
            } else {
                None
            };
            let _ = self.visual_memory.record_step(wins, tree);
            let diff = self.visual_memory.compute_diff_with_previous();
            let resp = ServerEvent {
                event: "visual_diff".into(),
                status: "ok".into(),
                message: None,
                data: Some(serde_json::to_value(diff)?),
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
        ClientCommand::SetCaptureMode { mode } => {
            *self.capture_mode.write().unwrap() = mode.clone();
            self.needs_refresh.store(true, Ordering::Relaxed);
            let resp = ServerEvent {
                event: "capture_mode_set".into(),
                status: "ok".into(),
                message: Some(format!("擷取模式已設定為: {}", mode)),
                data: Some(serde_json::json!({ "mode": mode })),
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
        ClientCommand::KillIsolatedApp { pid } => {
            let target_pid = pid.unwrap_or_else(|| self.isolated_pid.load(Ordering::Relaxed));
            if target_pid > 0 {
                match DesktopManager::kill_process(target_pid) {
                    Ok(_) => {
                        self.isolated_pid.store(0, Ordering::SeqCst);
                        let resp = ServerEvent {
                            event: "app_killed".into(),
                            status: "ok".into(),
                            message: Some(format!("進程 PID {} 已結束", target_pid)),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                    Err(e) => {
                        let resp = ServerEvent {
                            event: "app_killed".into(),
                            status: "error".into(),
                            message: Some(e),
                            data: None,
                        };
                        let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                    }
                }
            } else {
                let resp = ServerEvent {
                    event: "app_killed".into(),
                    status: "error".into(),
                    message: Some("未指定目標 PID".into()),
                    data: None,
                };
                let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
            }
        }
        ClientCommand::GetWindows => {
            let windows = WindowEnumerator::list_windows();
            let resp = ServerEvent {
                event: "windows".into(),
                status: "ok".into(),
                message: None,
                data: Some(serde_json::to_value(windows)?),
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
        ClientCommand::GetMonitors => {
            let monitors = MonitorManager::list_monitors();
            let resp = ServerEvent {
                event: "monitors".into(),
                status: "ok".into(),
                message: None,
                data: Some(serde_json::to_value(monitors)?),
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
        ClientCommand::DispatchToVirtual { hwnd } => {
            *self.capture_mode.write().unwrap() = "direct".to_string();
            match MonitorManager::dispatch_to_virtual(hwnd) {
                Ok(mon) => {
                    self.target_hwnd.store(hwnd, Ordering::SeqCst);
                    self.needs_refresh.store(true, Ordering::Relaxed);
                    let mut data_val = serde_json::to_value(&mon)?;
                    if let Some(obj) = data_val.as_object_mut() {
                        obj.insert("hwnd".to_string(), serde_json::json!(hwnd));
                    }
                    let resp = ServerEvent {
                        event: "window_dispatched".into(),
                        status: "ok".into(),
                        message: Some(format!("視窗已成功派駐、最大化並重繪至虛擬螢幕 ({} @ {}x{})", mon.name, mon.width, mon.height)),
                        data: Some(data_val),
                    };
                    let json_str = serde_json::to_string(&resp)?;
                    let _ = self.event_tx.send(json_str.clone());
                    let _ = ctrl_tx.send(json_str).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "window_dispatched".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::RestoreToPrimary { hwnd } => {
            match MonitorManager::restore_to_primary(hwnd) {
                Ok(mon) => {
                    let resp = ServerEvent {
                        event: "window_restored".into(),
                        status: "ok".into(),
                        message: Some(format!("視窗已移回主螢幕 ({})", mon.name)),
                        data: Some(serde_json::to_value(mon)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "window_restored".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::PlugInVirtualDisplay => {
            match MonitorManager::plug_in_virtual_display() {
                Ok(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                    let monitors = MonitorManager::list_monitors();
                    let resp = ServerEvent {
                        event: "virtual_display_plugged".into(),
                        status: "ok".into(),
                        message: Some("已成功透過 Win32 DeviceIoControl 掛載虛擬螢幕 (0 批次檔、0 UAC)".into()),
                        data: Some(serde_json::to_value(monitors)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "virtual_display_plugged".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::PlugOutVirtualDisplay => {
            match MonitorManager::plug_out_virtual_display() {
                Ok(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                    let monitors = MonitorManager::list_monitors();
                    let resp = ServerEvent {
                        event: "virtual_display_unplugged".into(),
                        status: "ok".into(),
                        message: Some("已成功透過 Win32 DeviceIoControl 卸載虛擬螢幕 (恢復單螢幕)".into()),
                        data: Some(serde_json::to_value(monitors)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "virtual_display_unplugged".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::SetWindow { hwnd } => {
            *self.capture_mode.write().unwrap() = "direct".to_string();
            // 若目標視窗已經在虛擬螢幕上，切換時自動執行最大化、Z-Order 置頂與強制重繪，確保畫面立即切換為該視窗！
            // 若目標視窗仍在主螢幕上，不擅自搬離主螢幕，待使用者點擊「🚀 派駐虛擬艙」時才搬移。
            if crate::capture::WindowCapturer::is_window_on_virtual_display(hwnd) {
                let _ = MonitorManager::dispatch_to_virtual(hwnd);
            }

            self.target_hwnd.store(hwnd, Ordering::SeqCst);
            self.needs_refresh.store(true, Ordering::Relaxed);
            let resp = ServerEvent {
                event: "window_set".into(),
                status: "ok".into(),
                message: Some(format!("已鎖定目標視窗 0x{:X}", hwnd)),
                data: Some(serde_json::json!({ "hwnd": hwnd })),
            };
            let json_str = serde_json::to_string(&resp)?;
            let _ = self.event_tx.send(json_str.clone());
            let _ = ctrl_tx.send(json_str).await;
        }
        ClientCommand::SetFps { fps } => {
            let clamped = fps.clamp(1, 60);
            self.target_fps.store(clamped, Ordering::SeqCst);
            let resp = ServerEvent {
                event: "fps_set".into(),
                status: "ok".into(),
                message: Some(format!("幀率設定為 {} FPS", clamped)),
                data: None,
            };
            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
        }
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
        ClientCommand::Launch {
            path,
            args,
            target_screen,
            working_dir,
            wait_for_window,
            timeout_ms,
        } => {
            let args_slice: Vec<String> = args.unwrap_or_default();
            let timeout = timeout_ms.unwrap_or(4000);
            match DesktopManager::launch_app(
                &path,
                &args_slice,
                &target_screen,
                working_dir.as_deref(),
                wait_for_window,
                timeout,
            ) {
                Ok(res) => {
                    if let Some(h) = res.hwnd {
                        self.target_hwnd.store(h, Ordering::SeqCst);
                        *self.capture_mode.write().unwrap() = "direct".to_string();
                        self.needs_refresh.store(true, Ordering::Relaxed);

                        let bcast_evt = ServerEvent {
                            event: "window_set".into(),
                            status: "ok".into(),
                            message: Some(format!("已鎖定目標視窗 0x{:X}", h)),
                            data: Some(serde_json::json!({ "hwnd": h })),
                        };
                        if let Ok(json_str) = serde_json::to_string(&bcast_evt) {
                            let _ = self.event_tx.send(json_str);
                        }
                    }
                    let resp = ServerEvent {
                        event: "app_launched".into(),
                        status: "ok".into(),
                        message: Some(format!(
                            "成功啟動進程 (PID: {}){}",
                            res.pid,
                            if let Some(h) = res.hwnd {
                                format!(" 並派駐至視窗 0x{:X}", h)
                            } else {
                                "".to_string()
                            }
                        )),
                        data: Some(serde_json::to_value(&res)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "app_launched".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
            }
        }
        ClientCommand::Close {
            hwnd,
            pid,
            force,
            timeout_ms,
        } => {
            let target_h = if hwnd.is_some() && hwnd.unwrap() != 0 {
                hwnd
            } else {
                let cur = self.resolve_target_hwnd(None);
                if cur != 0 {
                    Some(cur)
                } else {
                    None
                }
            };
            let timeout = timeout_ms.unwrap_or(2500);
            match DesktopManager::close_app(target_h, pid, force, timeout) {
                Ok(res) => {
                    if let Some(h) = res.hwnd {
                        if self.target_hwnd.load(Ordering::Relaxed) == h {
                            self.target_hwnd.store(0, Ordering::SeqCst);
                        }
                    }
                    self.needs_refresh.store(true, Ordering::Relaxed);
                    let resp = ServerEvent {
                        event: "window_closed".into(),
                        status: "ok".into(),
                        message: Some(format!(
                            "視窗已關閉 (HWND: {:?}, PID: {:?}, Force: {})",
                            res.hwnd.map(|h| format!("0x{:X}", h)),
                            res.pid,
                            res.forced
                        )),
                        data: Some(serde_json::to_value(&res)?),
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
                Err(e) => {
                    let resp = ServerEvent {
                        event: "window_closed".into(),
                        status: "error".into(),
                        message: Some(e),
                        data: None,
                    };
                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                }
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
        }
        Ok(())
    }
}
