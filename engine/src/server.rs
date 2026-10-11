use crate::capture::{WindowCapturer, WindowEnumerator};
use crate::input::InputArbiter;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
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
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum ClientCommand {
    #[serde(rename = "get_windows")]
    GetWindows,
    #[serde(rename = "set_window")]
    SetWindow { hwnd: isize },
    #[serde(rename = "set_fps")]
    SetFps { fps: u32 },
    #[serde(rename = "mouse")]
    Mouse {
        #[serde(default = "default_source")]
        source: String,
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
        subaction: String,
        #[serde(default)]
        vk: u16,
        #[serde(default)]
        char: Option<char>,
    },
}

fn default_source() -> String {
    "user".to_string()
}
fn default_button() -> String {
    "left".to_string()
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
        }
    }

    pub async fn run(self: Arc<Self>, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        crate::capture::ensure_default_desktop();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        let listener = TcpListener::bind(addr).await?;
        println!("[StreamEngine] WebSocket 伺服器已在 ws://{} 啟動", addr);

        // Spawn background capture loop
        let server_capture = self.clone();
        tokio::spawn(async move {
            server_capture.capture_loop().await;
        });

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

    async fn capture_loop(&self) {
        crate::capture::ensure_default_desktop();
        println!("[StreamEngine] 啟動背景擷取管線");
        loop {
            let fps = self.target_fps.load(Ordering::Relaxed).clamp(1, 60);
            let frame_dur = Duration::from_millis(1000 / fps as u64);

            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
            if hwnd_val != 0 {
                let capturer = WindowCapturer::new(hwnd_val);
                match capturer.capture_jpeg(75) {
                    Ok((jpeg_bytes, _, _)) => {
                        let _ = self.frame_tx.send(Arc::new(jpeg_bytes));
                    }
                    Err(e) => {
                        eprintln!("[StreamEngine] 擷取警告: {}", e);
                    }
                }
            }

            tokio::time::sleep(frame_dur).await;
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

        // Task 1: Stream binary JPEG frames + send text control messages
        let send_task = tokio::spawn(async move {
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
                        Ok(ClientCommand::SetWindow { hwnd }) => {
                            self.target_hwnd.store(hwnd, Ordering::SeqCst);
                            let resp = ServerEvent {
                                event: "window_set".into(),
                                status: "ok".into(),
                                message: Some(format!("已切換至視窗 0x{:X}", hwnd)),
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
                            subaction,
                            button,
                            u,
                            v,
                            delta,
                        }) => {
                            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
                            if hwnd_val != 0 {
                                let res = self.arbiter.execute_mouse(
                                    hwnd_val, &source, &subaction, &button, u, v, delta,
                                );
                                match res {
                                    Ok(msg) => {
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
                            subaction,
                            vk,
                            char,
                        }) => {
                            let hwnd_val = self.target_hwnd.load(Ordering::Relaxed);
                            if hwnd_val != 0 {
                                let res = self.arbiter.execute_key(
                                    hwnd_val, &source, &subaction, vk, char,
                                );
                                match res {
                                    Ok(msg) => {
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
