use crate::capture::WindowEnumerator;
use crate::desktop::DesktopManager;
use crate::protocol::{ClientCommand, ServerEvent};
use crate::server::StreamServer;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

impl StreamServer {
    pub(super) async fn handle_uia_command(
        &self,
        cmd: ClientCommand,
        ctrl_tx: &mpsc::Sender<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
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
            _ => unreachable!(),
        }
        Ok(())
    }
}
