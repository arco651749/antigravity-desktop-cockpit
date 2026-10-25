use crate::capture::{WindowCapturer, WindowEnumerator};
use crate::composite_capture::SecretDesktopCapturer;
use crate::desktop::{DesktopManager, DEFAULT_SECRET_DESKTOP};
use crate::input::InputArbiter;
use crate::monitor::MonitorManager;
use crate::uia::{UIAEngine, VisualMemory};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

pub struct StreamServer {
    pub target_hwnd: Arc<AtomicIsize>,
    pub target_fps: Arc<AtomicU32>,
    pub arbiter: Arc<InputArbiter>,
    pub frame_tx: broadcast::Sender<Arc<Vec<u8>>>,
    pub event_tx: broadcast::Sender<String>,
    pub last_frame: Arc<std::sync::RwLock<Option<Arc<Vec<u8>>>>>,
    pub needs_refresh: Arc<AtomicBool>,
    pub capture_mode: Arc<std::sync::RwLock<String>>,
    pub isolated_pid: Arc<AtomicU32>,
    pub isolated_desktop: Arc<std::sync::RwLock<String>>,
    pub uia: Arc<UIAEngine>,
    pub visual_memory: Arc<VisualMemory>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum ClientCommand {
    #[serde(rename = "launch_isolated")]
    LaunchIsolated {
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        args: Option<Vec<String>>,
    },
    #[serde(rename = "get_isolated_windows")]
    GetIsolatedWindows,
    #[serde(rename = "get_uia_tree")]
    GetUIATree {
        #[serde(default)]
        hwnd: Option<isize>,
        #[serde(default)]
        depth: Option<usize>,
    },
    #[serde(rename = "uia_invoke")]
    UIAInvoke {
        #[serde(default)]
        hwnd: Option<isize>,
        name: String,
    },
    #[serde(rename = "get_visual_diff")]
    GetVisualDiff,
    #[serde(rename = "set_capture_mode")]
    SetCaptureMode { mode: String },
    #[serde(rename = "kill_isolated_app")]
    KillIsolatedApp {
        #[serde(default)]
        pid: Option<u32>,
    },
    #[serde(rename = "get_windows")]
    GetWindows,
    #[serde(rename = "get_monitors")]
    GetMonitors,
    #[serde(rename = "dispatch_to_virtual")]
    DispatchToVirtual { hwnd: isize },
    #[serde(rename = "restore_to_primary")]
    RestoreToPrimary { hwnd: isize },
    #[serde(rename = "plug_in_virtual_display")]
    PlugInVirtualDisplay,
    #[serde(rename = "plug_out_virtual_display")]
    PlugOutVirtualDisplay,
    #[serde(rename = "set_window")]
    SetWindow { hwnd: isize },
    #[serde(rename = "set_fps")]
    SetFps { fps: u32 },
    #[serde(rename = "restore_primary_focus")]
    RestorePrimaryFocus,
    #[serde(rename = "mouse")]
    Mouse {
        #[serde(default = "default_source")]
        source: String,
        #[serde(default = "default_mode")]
        mode: String,
        subaction: String,
        #[serde(default = "default_button")]
        button: String,
        #[serde(default)]
        u: f64,
        #[serde(default)]
        v: f64,
        #[serde(default)]
        delta: i32,
    },
    #[serde(rename = "key")]
    Key {
        #[serde(default = "default_source")]
        source: String,
        #[serde(default = "default_mode")]
        mode: String,
        subaction: String,
        #[serde(default)]
        vk: u16,
        #[serde(default)]
        char: Option<char>,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
    #[serde(rename = "type_text")]
    TypeText {
        text: String,
        #[serde(default)]
        hwnd: Option<isize>,
    },
    #[serde(rename = "launch")]
    Launch {
        path: String,
        #[serde(default)]
        args: Option<Vec<String>>,
        #[serde(default = "default_target_screen")]
        target_screen: String,
        #[serde(default)]
        working_dir: Option<String>,
        #[serde(default = "default_true")]
        wait_for_window: bool,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename = "close", alias = "close_window", alias = "terminate")]
    Close {
        #[serde(default)]
        hwnd: Option<isize>,
        #[serde(default)]
        pid: Option<u32>,
        #[serde(default)]
        force: bool,
        #[serde(default = "default_close_timeout_ms")]
        timeout_ms: Option<u64>,
    },
}

fn default_target_screen() -> String {
    "virtual".to_string()
}
fn default_true() -> bool {
    true
}
fn default_timeout_ms() -> Option<u64> {
    Some(4000)
}
fn default_close_timeout_ms() -> Option<u64> {
    Some(2500)
}

fn default_source() -> String {
    "human".to_string()
}
fn default_button() -> String {
    "left".to_string()
}
fn default_mode() -> String {
    "direct".to_string()
}

#[derive(Debug, Serialize)]
pub struct ServerEvent {
    pub event: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl StreamServer {
    pub fn new() -> Self {
        let (frame_tx, _) = broadcast::channel(16);
        let (event_tx, _) = broadcast::channel(64);
        Self {
            target_hwnd: Arc::new(AtomicIsize::new(0)),
            target_fps: Arc::new(AtomicU32::new(30)),
            arbiter: Arc::new(InputArbiter::new()),
            frame_tx,
            event_tx,
            last_frame: Arc::new(std::sync::RwLock::new(None)),
            needs_refresh: Arc::new(AtomicBool::new(true)),
            capture_mode: Arc::new(std::sync::RwLock::new("direct".to_string())),
            isolated_pid: Arc::new(AtomicU32::new(0)),
            isolated_desktop: Arc::new(std::sync::RwLock::new(DEFAULT_SECRET_DESKTOP.to_string())),
            uia: Arc::new(UIAEngine::new().expect("UIAEngine 初始化失敗")),
            visual_memory: Arc::new(VisualMemory::new()),
        }
    }

    /// 智慧解析目標視窗 HWND：在隔離桌面中優先命中 #32770 模態彈窗，次之排除 IME/覆蓋層並鎖定最大面積之主視窗
    pub fn resolve_target_hwnd(&self, explicit_hwnd: Option<isize>) -> isize {
        if let Some(h) = explicit_hwnd {
            if h != 0 {
                return h;
            }
        }
        let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
        if is_secret {
            let desk_name = self.isolated_desktop.read().unwrap().clone();
            let isolated_pid = self.isolated_pid.load(Ordering::Relaxed);
            if let Ok(windows) = DesktopManager::list_windows_in_desktop(&desk_name) {
                let mut filtered = windows;
                if isolated_pid > 0 {
                    let by_pid: Vec<_> = filtered.iter().filter(|w| w.pid == isolated_pid).cloned().collect();
                    if !by_pid.is_empty() {
                        filtered = by_pid;
                    }
                }

                // 1. 優先回傳 #32770 模態彈窗 (如 Save As / Confirm 等)
                if let Some(dlg) = filtered.iter().find(|w| w.class_name == "#32770") {
                    return dlg.hwnd;
                }

                // 2. 過濾掉微軟輸入法/覆蓋層/無名透明小視窗 (如 UAC_InputIndicatorOverlayWnd, Default IME)
                let mut usable: Vec<_> = filtered
                    .into_iter()
                    .filter(|w| {
                        !w.class_name.starts_with("IME")
                            && !w.class_name.starts_with("MSCTFIME")
                            && !w.class_name.contains("Indicator")
                            && w.width > 50
                            && w.height > 50
                    })
                    .collect();

                // 3. 面積由大到小排序，主視窗面積最大
                usable.sort_by_key(|w| -(w.width as i64 * w.height as i64));
                if let Some(main_win) = usable.first() {
                    return main_win.hwnd;
                }
            }
            0
        } else {
            self.target_hwnd.load(Ordering::Relaxed)
        }
    }

    pub async fn run(self: Arc<Self>, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        crate::capture::ensure_default_desktop();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        let listener = TcpListener::bind(addr).await?;
        println!("[StreamEngine] WebSocket 伺服器已在 ws://{} 啟動", addr);

        // 在獨立專屬 OS 執行緒啟動背景擷取管線 (防止 Tokio 執行緒池切換破壞 Windows Desktop 綁定)
        let server_capture = self.clone();
        std::thread::Builder::new()
            .name("capture-worker".into())
            .spawn(move || {
                server_capture.capture_loop_thread();
            })
            .expect("Failed to spawn capture worker thread");


        // Accept WebSocket client connections
        while let Ok((stream, peer_addr)) = listener.accept().await {
            let server_conn = self.clone();
            tokio::spawn(async move {
                if let Err(e) = server_conn.handle_connection(stream, peer_addr).await {
                    eprintln!("[StreamEngine] 連線異常 ({}): {:?}", peer_addr, e);
                }
            });
        }

        Ok(())
    }

    fn capture_loop_thread(&self) {
        println!("[StreamEngine] 啟動專屬 OS 執行緒畫面擷取管線 (支援 Win32 隔離桌面與 DXGI 雙模式)");
        let mut capturer_opt: Option<WindowCapturer> = None;
        let mut secret_capturer_opt: Option<SecretDesktopCapturer> = None;
        let mut idle_count: u32 = 0;

        loop {
            // 1. 觀看者感知 (Subscriber-Aware)：若無任何前端連線觀看者，完全休眠 (0% CPU)
            if self.frame_tx.receiver_count() == 0 {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            let mode = self.capture_mode.read().unwrap().clone();
            let force_refresh = self.needs_refresh.swap(false, Ordering::Relaxed);

            if mode == "secret_desktop" {
                let desk_name = self.isolated_desktop.read().unwrap().clone();
                let pid = self.isolated_pid.load(Ordering::Relaxed);

                let need_recreate = match secret_capturer_opt.as_ref() {
                    Some(c) => c.desktop_name != desk_name || c.target_pid != pid,
                    None => true,
                };

                if need_recreate {
                    match SecretDesktopCapturer::new(&desk_name, pid) {
                        Ok(capturer) => {
                            secret_capturer_opt = Some(capturer);
                            idle_count = 0;
                        }
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(150));
                            continue;
                        }
                    }
                }

                if let Some(capturer) = secret_capturer_opt.as_mut() {
                    match capturer.capture_jpeg(75) {
                        Ok(frame_arc) => {
                            *self.last_frame.write().unwrap() = Some(frame_arc.clone());
                            let _ = self.frame_tx.send(frame_arc);
                            idle_count = 0;
                        }
                        Err(_) => {
                            idle_count = idle_count.saturating_add(1);
                        }
                    }
                }
            } else {
                crate::capture::ensure_default_desktop();
                let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
                if hwnd_val == 0 {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }

                if capturer_opt.as_ref().map(|c| c.hwnd) != Some(hwnd_val) {
                    // 關鍵修正：必須先將舊的 capturer_opt 設為 None 釋放 IDXGIOutputDuplication，
                    // 否則新建 WindowCapturer 時呼叫 DuplicateOutput 會因 DXGI 單一實例鎖定而失敗！
                    capturer_opt = None;
                    let capturer = WindowCapturer::new(hwnd_val);
                    if !capturer.last_jpeg.is_empty() {
                        let frame = capturer.last_jpeg.clone();
                        *self.last_frame.write().unwrap() = Some(frame.clone());
                        let _ = self.frame_tx.send(frame);
                    }
                    capturer_opt = Some(capturer);
                    idle_count = 0;
                }

                if force_refresh {
                    if let Some(c) = capturer_opt.as_mut() {
                        c.invalidate();
                    }
                    idle_count = 0;
                }

                if let Some(capturer) = capturer_opt.as_mut() {
                    match capturer.capture_jpeg(75) {
                        Ok(Some(frame_arc)) => {
                            *self.last_frame.write().unwrap() = Some(frame_arc.clone());
                            let _ = self.frame_tx.send(frame_arc);
                            idle_count = 0;
                        }
                        Ok(None) => {
                            if force_refresh || self.last_frame.read().unwrap().is_none() {
                                if !capturer.last_jpeg.is_empty() {
                                    let frame = capturer.last_jpeg.clone();
                                    *self.last_frame.write().unwrap() = Some(frame.clone());
                                    let _ = self.frame_tx.send(frame);
                                }
                            }
                            idle_count = idle_count.saturating_add(1);
                        }
                        Err(e) => {
                            eprintln!("[StreamEngine] 擷取異常: {}", e);
                            std::thread::sleep(Duration::from_millis(150));
                            continue;
                        }
                    }
                }
            }

            // 自適應智慧調頻
            let sleep_ms = if idle_count > 5 {
                80
            } else {
                let fps = self.target_fps.load(Ordering::Relaxed).clamp(1, 60);
                1000 / fps as u64
            };

            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    async fn handle_connection(
        &self,
        stream: TcpStream,
        _peer: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ws_stream = accept_async(stream).await?;
        let (mut ws_writer, mut ws_reader) = ws_stream.split();

        let mut frame_rx = self.frame_tx.subscribe();
        let mut event_rx = self.event_tx.subscribe();

        // Channel for text control messages from reader to writer
        let (ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::channel::<String>(32);

        // 連線建立時標記強制刷新，並若已有快取畫面立刻推播首幀
        self.needs_refresh.store(true, Ordering::Relaxed);
        let init_frame = self.last_frame.read().unwrap().clone();

        // Task 1: Stream binary JPEG frames + send text control messages + broadcast events
        let send_task = tokio::spawn(async move {
            if let Some(cached) = init_frame {
                let _ = ws_writer.send(Message::Binary((*cached).clone())).await;
            }
            loop {
                tokio::select! {
                    msg = ctrl_rx.recv() => {
                        if let Some(text) = msg {
                            if ws_writer.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    bcast = event_rx.recv() => {
                        if let Ok(text) = bcast {
                            if ws_writer.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                    }
                    frame = frame_rx.recv() => {
                        if let Ok(bytes) = frame {
                            if ws_writer.send(Message::Binary((*bytes).clone())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Task 2: Process incoming client commands
        while let Some(msg_result) = ws_reader.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    let cmd_res: Result<ClientCommand, _> = serde_json::from_str(&text);
                    match cmd_res {
                        Ok(ClientCommand::LaunchIsolated { app, args }) => {
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
                        Ok(ClientCommand::GetIsolatedWindows) => {
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
                        Ok(ClientCommand::GetUIATree { hwnd, depth }) => {
                            let is_secret = *self.capture_mode.read().unwrap() == "secret_desktop";
                            let desk_name = self.isolated_desktop.read().unwrap().clone();
                            let target_hwnd = self.resolve_target_hwnd(hwnd);

                            if target_hwnd != 0 {
                                match self.uia.fetch_tree_from_hwnd(target_hwnd, depth.unwrap_or(4)) {
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
                        Ok(ClientCommand::UIAInvoke { hwnd, name }) => {
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
                        Ok(ClientCommand::GetVisualDiff) => {
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
                                self.uia.fetch_tree_from_hwnd(target_hwnd, 3).ok()
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
                        Ok(ClientCommand::SetCaptureMode { mode }) => {
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
                        Ok(ClientCommand::KillIsolatedApp { pid }) => {
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
                        Ok(ClientCommand::GetWindows) => {
                            let windows = WindowEnumerator::list_windows();
                            let resp = ServerEvent {
                                event: "windows".into(),
                                status: "ok".into(),
                                message: None,
                                data: Some(serde_json::to_value(windows)?),
                            };
                            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                        }
                        Ok(ClientCommand::GetMonitors) => {
                            let monitors = MonitorManager::list_monitors();
                            let resp = ServerEvent {
                                event: "monitors".into(),
                                status: "ok".into(),
                                message: None,
                                data: Some(serde_json::to_value(monitors)?),
                            };
                            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
                        }
                        Ok(ClientCommand::DispatchToVirtual { hwnd }) => {
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
                        Ok(ClientCommand::RestoreToPrimary { hwnd }) => {
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
                        Ok(ClientCommand::PlugInVirtualDisplay) => {
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
                        Ok(ClientCommand::PlugOutVirtualDisplay) => {
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
                        Ok(ClientCommand::SetWindow { hwnd }) => {
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
                        Ok(ClientCommand::SetFps { fps }) => {
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
                        Ok(ClientCommand::RestorePrimaryFocus) => {
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
                        Ok(ClientCommand::Mouse {
                            source,
                            mode,
                            subaction,
                            button,
                            u,
                            v,
                            delta,
                        }) => {
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
                        Ok(ClientCommand::Key {
                            source,
                            mode,
                            subaction,
                            vk,
                            char,
                            ctrl,
                            alt,
                            shift,
                        }) => {
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
                        Ok(ClientCommand::TypeText { text, hwnd }) => {
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
                        Ok(ClientCommand::Launch {
                            path,
                            args,
                            target_screen,
                            working_dir,
                            wait_for_window,
                            timeout_ms,
                        }) => {
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
                        Ok(ClientCommand::Close {
                            hwnd,
                            pid,
                            force,
                            timeout_ms,
                        }) => {
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
                        Err(e) => {
                            eprintln!("[StreamEngine] JSON 解析失敗: {:?}, 原文: {}", e, text);
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }

        send_task.abort();
        Ok(())
    }
}
