use crate::capture::{WindowCapturer, WindowEnumerator};
use crate::input::InputArbiter;
use crate::monitor::MonitorManager;
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
    pub last_frame: Arc<std::sync::RwLock<Option<Arc<Vec<u8>>>>>,
    pub needs_refresh: Arc<AtomicBool>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum ClientCommand {
    #[serde(rename = "get_windows")]
    GetWindows,
    #[serde(rename = "get_monitors")]
    GetMonitors,
    #[serde(rename = "dispatch_to_virtual")]
    DispatchToVirtual { hwnd: isize },
    #[serde(rename = "restore_to_primary")]
    RestoreToPrimary { hwnd: isize },
    #[serde(rename = "set_window")]
    SetWindow { hwnd: isize },
    #[serde(rename = "set_fps")]
    SetFps { fps: u32 },
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
        Self {
            target_hwnd: Arc::new(AtomicIsize::new(0)),
            target_fps: Arc::new(AtomicU32::new(30)),
            arbiter: Arc::new(InputArbiter::new()),
            frame_tx,
            last_frame: Arc::new(std::sync::RwLock::new(None)),
            needs_refresh: Arc::new(AtomicBool::new(true)),
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
        crate::capture::ensure_default_desktop();
        println!("[StreamEngine] 啟動專屬 OS 執行緒 DXGI 硬體擷取管線 (Affinity Pinned + DDA + Adaptive FPS)");
        let mut capturer_opt: Option<WindowCapturer> = None;
        let mut idle_count: u32 = 0;

        loop {
            // 1. 觀看者感知 (Subscriber-Aware)：若無任何前端連線觀看者，完全休眠 (0% CPU)
            if self.frame_tx.receiver_count() == 0 {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
            if hwnd_val == 0 {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }

            if capturer_opt.as_ref().map(|c| c.hwnd) != Some(hwnd_val) {
                let capturer = WindowCapturer::new(hwnd_val);
                // 優先使用初始化獲取的首張冷啟動影格立即廣播
                if !capturer.last_jpeg.is_empty() {
                    let frame = capturer.last_jpeg.clone();
                    *self.last_frame.write().unwrap() = Some(frame.clone());
                    let _ = self.frame_tx.send(frame);
                }
                capturer_opt = Some(capturer);
                idle_count = 0;
            }

            let force_refresh = self.needs_refresh.swap(false, Ordering::Relaxed);
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
                        // 畫面無變化：若處於強制重整或冷啟動尚未有任何發送，重送當前快取幀保證畫面不留白
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

            // 2. 自適應智慧調頻 (Adaptive FPS)
            let sleep_ms = if idle_count > 5 {
                100
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

        // Channel for text control messages from reader to writer
        let (ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::channel::<String>(32);

        // 連線建立時標記強制刷新，並若已有快取畫面立刻推播首幀
        self.needs_refresh.store(true, Ordering::Relaxed);
        let init_frame = self.last_frame.read().unwrap().clone();

        // Task 1: Stream binary JPEG frames + send text control messages
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
                            match MonitorManager::dispatch_to_virtual(hwnd) {
                                Ok(mon) => {
                                    self.target_hwnd.store(hwnd, Ordering::SeqCst);
                                    self.needs_refresh.store(true, Ordering::Relaxed);
                                    let resp = ServerEvent {
                                        event: "window_dispatched".into(),
                                        status: "ok".into(),
                                        message: Some(format!("視窗已成功派駐至虛擬螢幕 ({} @ {}x{})", mon.name, mon.width, mon.height)),
                                        data: Some(serde_json::to_value(mon)?),
                                    };
                                    let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
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
                        Ok(ClientCommand::SetWindow { hwnd }) => {
                            unsafe {
                                let h = windows::Win32::Foundation::HWND(hwnd as *mut _);
                                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                                    h,
                                    windows::Win32::UI::WindowsAndMessaging::SW_RESTORE,
                                );
                            }
                            if !crate::capture::WindowCapturer::is_window_on_virtual_display(hwnd) {
                                let _ = MonitorManager::dispatch_to_virtual(hwnd);
                            }

                            self.target_hwnd.store(hwnd, Ordering::SeqCst);
                            self.needs_refresh.store(true, Ordering::Relaxed);
                            let resp = ServerEvent {
                                event: "window_set".into(),
                                status: "ok".into(),
                                message: Some(format!("已自動還原並派駐視窗 0x{:X} 至虛擬螢幕", hwnd)),
                                data: None,
                            };
                            let _ = ctrl_tx.send(serde_json::to_string(&resp)?).await;
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
                        Ok(ClientCommand::Mouse {
                            source,
                            mode,
                            subaction,
                            button,
                            u,
                            v,
                            delta,
                        }) => {
                            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
                            if hwnd_val != 0 {
                                let res = if mode.eq_ignore_ascii_case("background") {
                                    self.arbiter.execute_mouse(
                                        hwnd_val, &source, &subaction, &button, u, v, delta,
                                    )
                                } else {
                                    self.arbiter.execute_mouse_direct(
                                        hwnd_val, &source, &subaction, &button, u, v, delta,
                                    )
                                };
                                match res {
                                    Ok(msg) => {
                                        self.needs_refresh.store(true, Ordering::Relaxed);
                                        if subaction != "move" {
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
                            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
                            if hwnd_val != 0 {
                                let res = if mode.eq_ignore_ascii_case("background") {
                                    self.arbiter.execute_key(
                                        hwnd_val, &source, &subaction, vk, char, ctrl, alt, shift,
                                    )
                                } else {
                                    self.arbiter.execute_key_direct(
                                        hwnd_val, &source, &subaction, vk, char, ctrl, alt, shift,
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
