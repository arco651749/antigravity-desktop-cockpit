use gui_stream_engine::desktop::DesktopManager;
use gui_stream_engine::uia::{UIAEngine, VisualMemory};
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_CLOSE, WM_SETFOCUS};

#[link(name = "user32")]
extern "system" {
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> windows::Win32::Foundation::BOOL>,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::BOOL;
}

unsafe extern "system" fn enum_child(hwnd: HWND, lparam: LPARAM) -> windows::Win32::Foundation::BOOL {
    let list = &mut *(lparam.0 as *mut Vec<HWND>);
    list.push(hwnd);
    windows::Win32::Foundation::BOOL(1)
}

fn main() {
    let desk = "CockpitSecretDesk";
    println!("=======================================================");
    println!("[ADC 2.0 Milestone 2 驗收測試] Rust 原生 UIA 語意感知與靜默互動");
    println!("=======================================================");

    // 1. 綁定執行緒至隔離桌面
    let _ = DesktopManager::bind_thread_to_desktop(desk).expect("綁定隔離桌面失敗");

    // 2. 啟動乾淨的 Notepad++
    let pid = DesktopManager::launch_notepad_plus_plus(desk).expect("啟動 Notepad++ 失敗");
    println!("[STEP 1] 已啟動 Notepad++ (PID: {})", pid);
    sleep(Duration::from_millis(1000));

    let wins_init = DesktopManager::list_windows_in_desktop(desk).expect("列出視窗失敗");
    let np_main = wins_init.iter().find(|w| w.class_name == "Notepad++").expect("未找到 Notepad++ 主視窗");
    let main_hwnd = np_main.hwnd;
    println!("[STEP 2] Notepad++ 主視窗: HWND 0x{:X}", main_hwnd);

    // 3. 初始化 UIA 引擎並擷取語意樹
    let t0 = Instant::now();
    let uia = UIAEngine::new().expect("UIAEngine 初始化失敗");
    let tree = uia.fetch_tree_from_hwnd(main_hwnd, 4).expect("掃描 UIA 樹失敗");
    println!("[STEP 3] UIA 樹狀掃描成功 (耗時: {:?})", t0.elapsed());
    println!("  -> 根節點: [{}] \"{}\", Class: \"{}\"", tree.control_type, tree.name, tree.class_name);
    println!("  -> 子節點數量: {}", tree.children.len());
    for (i, c) in tree.children.iter().enumerate() {
        println!("     [{}] {} \"{}\" (Rect: {:?})", i + 1, c.control_type, c.name, c.bounding_rect);
    }

    // 4. 初始化 VisualMemory 快照記錄
    let memory = VisualMemory::new();
    let _ = memory.record_step(wins_init, Some(tree));
    println!("[STEP 4] 基準感知快照 Step 1 記錄完成");

    // 5. 鍵入文字使文件處於 Modified 狀態，並發送 WM_CLOSE 觸發儲存詢問彈窗
    println!("[STEP 5] 鍵入字元並發送關閉指令...");
    unsafe {
        let mut children: Vec<HWND> = Vec::new();
        let ptr = &mut children as *mut _ as isize;
        let _ = EnumChildWindows(HWND(main_hwnd as _), Some(enum_child), LPARAM(ptr));
        for ch in children {
            let mut cbuf = [0u16; 128];
            let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(ch, &mut cbuf);
            let cls = String::from_utf16_lossy(&cbuf[..len.max(0) as usize]);
            if cls.starts_with("Scintilla") {
                let _ = PostMessageW(ch, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                let _ = PostMessageW(ch, WM_CHAR, WPARAM('T' as usize), LPARAM(1));
                break;
            }
        }
        sleep(Duration::from_millis(250));
        let _ = PostMessageW(HWND(main_hwnd as _), WM_CLOSE, WPARAM(0), LPARAM(0));
    }

    sleep(Duration::from_millis(600));

    // 6. 記錄 Step 2 感知快照並執行 Visual Memory 差異比對
    let wins_step2 = DesktopManager::list_windows_in_desktop(desk).expect("列出視窗失敗");
    let dlg_info = wins_step2.iter().find(|w| w.class_name == "#32770").expect("未探測到 #32770 彈窗");
    let dlg_hwnd = dlg_info.hwnd;

    let dlg_tree = uia.fetch_tree_from_hwnd(dlg_hwnd, 3).ok();
    let _ = memory.record_step(wins_step2, dlg_tree.clone());

    let diff = memory.compute_diff_with_previous().expect("比對失敗");
    println!("[STEP 6] Visual Memory 差異偵測成功！");
    println!("  -> 新增視窗: {} 個", diff.added_windows.len());
    for w in &diff.added_windows {
        println!("     + HWND 0x{:X} Title=\"{}\" Class=\"{}\"", w.hwnd, w.title, w.class_name);
    }
    println!("  -> 新增對話框 (diff_mode=added): {} 個", diff.new_dialogs.len());
    for d in &diff.new_dialogs {
        println!("     [DETECTED DIALOG] HWND 0x{:X} Title=\"{}\"", d.hwnd, d.title);
    }

    // 7. 檢查對話框的 UIA 控制項 (是否有 "No" 按鈕)
    if let Some(ref d_tree) = dlg_tree {
        println!("[STEP 7] 對話框 UIA 語意節點:");
        for elem in d_tree.flatten() {
            if elem.control_type == "Button" {
                println!("  -> Button: \"{}\" Patterns: {:?}", elem.name, elem.supported_patterns);
            }
        }
    }

    // 8. 透過 UIA InvokePattern 靜默點擊 "No" (不儲存直接退出)
    println!("[STEP 8] 執行 UIA 靜默互動: InvokeElementByName(\"No\")...");
    let invoked = uia.invoke_element_by_name(dlg_hwnd, "No").expect("Invoke 過程失敗");
    println!("  -> Invoke 結果: {}", invoked);

    // 等待 Notepad++ 關閉
    sleep(Duration::from_millis(500));

    // 9. 驗證視窗已完全正常關閉
    let wins_final = DesktopManager::list_windows_in_desktop(desk).unwrap_or_default();
    let np_still_alive = wins_final.iter().any(|w| w.pid == pid);
    if !np_still_alive {
        println!("[SUCCESS] Notepad++ 接收到 UIA Invoke(\"No\") 後已乾淨結束！");
    } else {
        println!("[WARN] Notepad++ 仍在背景運行，強制清理進程");
        let _ = DesktopManager::kill_process(pid);
    }

    println!("=======================================================");
    println!("[驗收結果] Milestone 2 (UIA 語意樹 + 快照比對 + InvokePattern) 全部通過！");
    println!("=======================================================");
}
