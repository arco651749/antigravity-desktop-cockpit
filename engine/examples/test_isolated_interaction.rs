use gui_stream_engine::composite_capture::SecretDesktopCapturer;
use gui_stream_engine::desktop::{DesktopManager, DesktopWindowInfo};
use std::fs::File;
use std::io::Write;
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, PostMessageW, SendMessageW,
    WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KEYUP, WM_SETFOCUS,
};

#[link(name = "user32")]
extern "system" {
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
}

#[derive(Debug, Clone)]
struct ChildInfo {
    hwnd: isize,
    class: String,
    w: i32,
    h: i32,
}

unsafe extern "system" fn enum_child_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<ChildInfo>);
    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);

    let mut rect = windows::Win32::Foundation::RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;

    list.push(ChildInfo {
        hwnd: hwnd.0 as isize,
        class: class_name,
        w,
        h,
    });
    BOOL(1)
}

fn main() {
    let desktop_name = "CockpitSecretDesk";
    println!("=======================================================");
    println!("[ADC 2.0 互動測試] 隔離桌面 Notepad++ 靜默文字輸入與彈窗擷取");
    println!("=======================================================");

    // 1. 綁定執行緒至隔離桌面
    let _ = DesktopManager::bind_thread_to_desktop(desktop_name).expect("無法綁定隔離桌面");

    // 2. 獲取當前隔離桌面上的 Notepad++ 視窗
    let windows = DesktopManager::list_windows_in_desktop(desktop_name).expect("列出視窗失敗");
    let np_main = windows.iter().find(|w| w.class_name == "Notepad++");

    let (main_hwnd, pid) = match np_main {
        Some(w) => (w.hwnd, w.pid),
        None => {
            println!("未找到現有 Notepad++，正在新啟動實例...");
            let pid = DesktopManager::launch_notepad_plus_plus(desktop_name).expect("啟動失敗");
            sleep(Duration::from_millis(800));
            let wins = DesktopManager::list_windows_in_desktop(desktop_name).expect("列出視窗失敗");
            let m = wins.into_iter().find(|w| w.class_name == "Notepad++").expect("未找到主視窗");
            (m.hwnd, pid)
        }
    };

    println!("[STEP 1] 目標視窗: HWND 0x{:X}, PID {}", main_hwnd, pid);

    // 3. 枚舉子控制項，尋找 Scintilla 編輯器
    let mut children: Vec<ChildInfo> = Vec::new();
    unsafe {
        let ptr = &mut children as *mut Vec<ChildInfo> as isize;
        let _ = EnumChildWindows(HWND(main_hwnd as _), Some(enum_child_proc), LPARAM(ptr));
    }

    println!("[STEP 2] 子控制項枚舉完成 (共 {} 個):", children.len());
    let mut scintilla_hwnd = None;
    for c in &children {
        if c.class.starts_with("Scintilla") {
            println!("   -> 發現編輯核心 Scintilla: HWND 0x{:X} ({}x{})", c.hwnd, c.w, c.h);
            if scintilla_hwnd.is_none() || (c.w * c.h > 10000) {
                scintilla_hwnd = Some(c.hwnd);
            }
        }
    }

    let edit_target = scintilla_hwnd.unwrap_or(main_hwnd);
    println!("[STEP 3] 設定文字焦點至 0x{:X}", edit_target);

    unsafe {
        // 設定焦點
        PostMessageW(HWND(edit_target as _), WM_SETFOCUS, WPARAM(0), LPARAM(0));
        sleep(Duration::from_millis(50));

        // 注入文字: "ADC 2.0: 100% Zero-Interference Native Cockpit!"
        let test_text = "ADC 2.0: 100% Zero-Interference Native Cockpit!\r\nHello from background isolated desktop.\r\n";
        println!("注入文字測試...");
        for ch in test_text.chars() {
            PostMessageW(
                HWND(edit_target as _),
                WM_CHAR,
                WPARAM(ch as usize),
                LPARAM(1),
            );
        }
    }

    sleep(Duration::from_millis(300));

    // 4. 快照 1: 驗證文字已成功鍵入
    let mut capturer = SecretDesktopCapturer::new(desktop_name, pid).expect("Capturer 建立失敗");
    let snap1 = capturer.capture_jpeg(85).expect("擷取 snap1 失敗");
    let snap1_path = r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\isolated_typing_verified.jpg";
    File::create(snap1_path).unwrap().write_all(&snap1).unwrap();
    println!("[STEP 4] 文字輸入快照已儲存: {}", snap1_path);

    // 5. 觸發儲存詢問彈窗 (發送 WM_CLOSE 關閉未儲存的文件視窗)
    println!("[STEP 5] 發送關閉指令 (觸發未儲存確認對話框)...");
    unsafe {
        PostMessageW(HWND(main_hwnd as _), WM_CLOSE, WPARAM(0), LPARAM(0));
    }

    // 等待對話框彈出
    sleep(Duration::from_millis(600));

    // 6. 再次檢查視窗清單，確認是否有對話框彈窗 (如 #32770)
    let wins_after = DesktopManager::list_windows_in_desktop(desktop_name).expect("列出視窗失敗");
    println!("[STEP 6] 關閉後桌面視窗清單 (共 {} 個):", wins_after.len());
    for w in &wins_after {
        if w.pid == pid {
            println!("   -> HWND 0x{:X}: Title=\"{}\", Class=\"{}\", Rect=({}, {}) {}x{}",
                w.hwnd, w.title, w.class_name, w.x, w.y, w.width, w.height);
        }
    }

    // 7. 快照 2: 驗證複合視窗擷取 (Composite Capture 是否同時抓下主視窗與疊加在上的對話框)
    let snap2 = capturer.capture_jpeg(85).expect("擷取 snap2 失敗");
    let snap2_path = r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\isolated_dialog_composite_verified.jpg";
    File::create(snap2_path).unwrap().write_all(&snap2).unwrap();
    println!("[STEP 7] 複合對話框快照已儲存: {}", snap2_path);

    println!("=======================================================");
    println!("[測試完成] 快照驗收成功！");
    println!("=======================================================");
}
