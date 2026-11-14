use crate::capture::WindowCapturer;
use crate::composite_capture::SecretDesktopCapturer;
use crate::desktop::{DesktopManager, DEFAULT_SECRET_DESKTOP};
use crate::input::InputArbiter;
use crate::uia::{UIAEngine, VisualMemory};
use crate::protocol::ClientCommand;
use futures_util::{SinkExt, StreamExt};
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
                        Ok(cmd) => {
                            if let Err(e) = self.dispatch_command(cmd, &ctrl_tx).await {
                                eprintln!("[StreamEngine] 指令執行失敗: {:?}", e);
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
