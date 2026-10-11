#[path = "../capture.rs"]
mod capture;
use capture::{ensure_default_desktop, DxgiCapturer, WindowCapturer};
use std::time::{Duration, Instant};

fn main() {
    let handle = std::thread::spawn(|| {
        ensure_default_desktop();
        println!("[Dedicated Thread] 成功綁定專屬 OS 執行緒桌面！");

        // Try DxgiCapturer on DISPLAY7
        match DxgiCapturer::new_for_display(r"\\.\DISPLAY7") {
            Ok(mut dxgi) => {
                println!("[Dedicated Thread] DXGI 初始化成功: {}x{}", dxgi.width, dxgi.height);
                for i in 1..=5 {
                    let t0 = Instant::now();
                    let res = dxgi.capture_jpeg(50, 75);
                    let elapsed = t0.elapsed();
                    match res {
                        Ok(Some(frame)) => {
                            println!("Frame {}: 獲得新影格 {} bytes (耗時 {:?})", i, frame.len(), elapsed);
                        }
                        Ok(None) => {
                            println!("Frame {}: 靜止逾時 (耗時 {:?})", i, elapsed);
                        }
                        Err(e) => {
                            println!("Frame {}: 擷取錯誤: {:?}", i, e);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Err(e) => {
                println!("[Dedicated Thread] DXGI 初始化失敗: {:?}", e);
            }
        }
    });

    handle.join().unwrap();
}
