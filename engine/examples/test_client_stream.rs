use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[TestClient] 連線至 ws://127.0.0.1:48280 ...");
    let (ws_stream, _) = connect_async("ws://127.0.0.1:48280").await?;
    let (mut write, mut read) = ws_stream.split();

    // 1. 取得視窗列表找到 Chrome
    write.send(Message::Text(json!({ "action": "get_windows" }).to_string())).await?;
    let mut chrome_hwnd: Option<isize> = None;

    while let Some(msg) = read.next().await {
        if let Ok(Message::Text(txt)) = msg {
            let v: serde_json::Value = serde_json::from_str(&txt)?;
            if v["event"] == "windows" {
                if let Some(arr) = v["data"].as_array() {
                    for item in arr {
                        let title = item["title"].as_str().unwrap_or("");
                        let h = item["hwnd"].as_i64().unwrap_or(0);
                        println!("  [Found Window] 0x{:X}: {}", h, title);
                        if title.to_lowercase().contains("chrome") || title.contains("Google Drive") || title.contains("Drive") {
                            chrome_hwnd = Some(h as isize);
                            println!("[TestClient] 鎖定目標視窗: {} (HWND: 0x{:X})", title, h);
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    let hwnd = match chrome_hwnd {
        Some(h) => h,
        None => {
            eprintln!("[TestClient] 找不到 Chrome 視窗！");
            return Ok(());
        }
    };

    // 2. 派駐視窗至虛擬螢幕
    println!("[TestClient] 派駐視窗 0x{:X} 至 1080p 虛擬螢幕...", hwnd);
    write.send(Message::Text(json!({
        "action": "dispatch_to_virtual",
        "hwnd": hwnd
    }).to_string())).await?;

    // 等待派駐完成回應
    while let Some(msg) = read.next().await {
        if let Ok(Message::Text(txt)) = msg {
            let v: serde_json::Value = serde_json::from_str(&txt)?;
            if v["event"] == "window_dispatched" {
                println!("[TestClient] 視窗派駐成功: {:?}", v["message"]);
                break;
            }
        }
    }

    // 3. 設定串流目標視窗
    write.send(Message::Text(json!({
        "action": "set_window",
        "hwnd": hwnd
    }).to_string())).await?;

    // 4. 連續接收 5 秒的影像幀，並統計影格數與數據量
    println!("[TestClient] 開始接收影像幀 (持續 5 秒)...");
    let start_time = Instant::now();
    let mut frame_count = 0;
    let mut total_bytes = 0;
    let mut first_frame_saved = false;

    while start_time.elapsed() < Duration::from_secs(5) {
        let timeout_fut = tokio::time::timeout(Duration::from_millis(500), read.next());
        match timeout_fut.await {
            Ok(Some(Ok(Message::Binary(bin)))) => {
                frame_count += 1;
                total_bytes += bin.len();
                if !first_frame_saved {
                    std::fs::write("D:\\Users\\arco0\\.gemini\\antigravity\\brain\\a37c0d39-fc53-4582-b89e-bb8dffe1c938\\dxgi_chrome_1080p.jpg", &bin)?;
                    println!("[TestClient] 首張 DXGI 1080p 畫面已儲存至 artifact! ({} bytes)", bin.len());
                    first_frame_saved = true;
                }
            }
            Ok(Some(Ok(Message::Text(_)))) => {}
            _ => {}
        }
    }

    println!("========== 5 秒端到端串流測試結果 ==========");
    println!("接收總影格數 (Frames): {}", frame_count);
    println!("接收總流量: {} KB ({:.2} KB/s)", total_bytes / 1024, (total_bytes as f64 / 1024.0) / 5.0);
    println!("==========================================");

    Ok(())
}
