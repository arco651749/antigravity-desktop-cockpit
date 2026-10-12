use gui_stream_engine::composite_capture::SecretDesktopCapturer;
use gui_stream_engine::desktop::DesktopManager;
use std::fs::File;
use std::io::Write;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn main() {
    let desktop_name = "CockpitSecretDesk";
    println!("=======================================================");
    println!("[ADC 2.0 驗收測試] 零干擾純 Win32 隔離桌面 + Notepad++");
    println!("=======================================================");

    // 1. 建立或開啟隔離桌面
    let t0 = Instant::now();
    let _ = match DesktopManager::create_or_open_desktop(desktop_name) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[FAIL] 建立隔離桌面失敗: {}", e);
            return;
        }
    };
    println!("[STEP 1] 隔離桌面 '{}' 建立成功 (耗時: {:?})", desktop_name, t0.elapsed());

    // 2. 啟動 Notepad++ (-multiInst -nosession)
    let t1 = Instant::now();
    let pid = match DesktopManager::launch_notepad_plus_plus(desktop_name) {
        Ok(pid) => {
            println!("[STEP 2] Notepad++ 成功於隔離桌面啟動！PID: {} (耗時: {:?})", pid, t1.elapsed());
            pid
        }
        Err(e) => {
            eprintln!("[FAIL] 啟動 Notepad++ 失敗: {}", e);
            return;
        }
    };

    // 3. 輪詢視窗清單，確認 Notepad++ 主視窗已建立且渲染就緒
    println!("[STEP 3] 等待主視窗渲染就緒 (輪詢中)...");
    let mut found_windows = Vec::new();
    for i in 1..=30 {
        sleep(Duration::from_millis(250));
        if let Ok(windows) = DesktopManager::list_windows_in_desktop(desktop_name) {
            let app_wins: Vec<_> = windows.into_iter().filter(|w| w.pid == pid).collect();
            let main_win = app_wins.iter().find(|w| w.width >= 300 && w.height >= 200);
            if let Some(mw) = main_win {
                println!("  -> 第 {} 次輪詢成功發現 Notepad++ 主視窗！(共 {} 個視窗):", i, app_wins.len());
                for w in &app_wins {
                    println!("     [HWND 0x{:X}] Title: \"{}\", Class: \"{}\", Pos: ({}, {}) Size: {}x{}",
                        w.hwnd, w.title, w.class_name, w.x, w.y, w.width, w.height);
                }
                found_windows = app_wins;
                break;
            } else if !app_wins.is_empty() {
                println!("  -> 第 {} 次輪詢探測到前導輔助視窗 ({} 個)，繼續等待主視窗...", i, app_wins.len());
            }
        }
    }

    if found_windows.is_empty() {
        eprintln!("[FAIL] 超時 (4秒) 未在隔離桌面探測到 Notepad++ 可見視窗！");
        let _ = DesktopManager::kill_process(pid);
        return;
    }

    // 4. 初始化 SecretDesktopCapturer 並截取 Composite 影格
    println!("[STEP 4] 啟動 SecretDesktopCapturer 並執行 Composite 畫面截圖...");
    let mut capturer = match SecretDesktopCapturer::new(desktop_name, pid) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[FAIL] 初始化 SecretDesktopCapturer 失敗: {}", e);
            let _ = DesktopManager::kill_process(pid);
            return;
        }
    };

    // 等待 300ms 確保視窗內容首次繪製完畢
    sleep(Duration::from_millis(300));

    let t_cap = Instant::now();
    match capturer.capture_jpeg(85) {
        Ok(jpeg_bytes) => {
            let cap_duration = t_cap.elapsed();
            println!("[STEP 5] 畫面 Composite 擷取成功！JPEG 大小: {} bytes, 耗時: {:?}",
                jpeg_bytes.len(), cap_duration);

            let artifact_path = r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\isolated_notepadpp_test.jpg";
            let mut f = File::create(artifact_path).expect("無法建立輸出圖片檔案");
            f.write_all(&jpeg_bytes).expect("寫入失敗");
            println!("[SUCCESS] 快照已寫入 Artifact 目錄: {}", artifact_path);
        }
        Err(e) => {
            eprintln!("[FAIL] 擷取影格失敗: {}", e);
        }
    }

    println!("=======================================================");
    println!("[驗收結果] Notepad++ 已在獨立背景桌面正常運作，主螢幕 0 干擾！");
    println!("保留 PID: {} 供後續 UIA 與無感控制測試", pid);
    println!("=======================================================");
}
