use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, PostMessageW, SendMessageW,
    WM_CLOSE, WM_PAINT, WM_SETFOCUS,
};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, nflags: u32) -> BOOL;
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
}

#[derive(Debug, Clone)]
struct Child {
    hwnd: HWND,
    class: String,
    title: String,
    rect: RECT,
}

unsafe extern "system" fn enum_child(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<Child>);
    let mut cbuf = [0u16; 128];
    let clen = GetClassNameW(hwnd, &mut cbuf);
    let class = String::from_utf16_lossy(&cbuf[..clen.max(0) as usize]);

    let mut tbuf = [0u16; 128];
    let tlen = GetWindowTextW(hwnd, &mut tbuf);
    let title = String::from_utf16_lossy(&tbuf[..tlen.max(0) as usize]);

    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    list.push(Child { hwnd, class, title, rect });
    BOOL(1)
}

fn save_dc_jpeg(hdc: HDC, hbm: windows::Win32::Graphics::Gdi::HBITMAP, w: i32, h: i32, path: &str) {
    unsafe {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (w * h * 4) as u32,
            ..Default::default()
        };

        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let _ = GetDIBits(
            hdc,
            hbm,
            0,
            h as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        let mut rgb = vec![0u8; (w * h * 3) as usize];
        let mut s = 0;
        let mut d = 0;
        for _ in 0..(w * h) {
            rgb[d] = bgra[s + 2];
            rgb[d + 1] = bgra[s + 1];
            rgb[d + 2] = bgra[s];
            s += 4;
            d += 3;
        }

        let mut jpeg = Vec::new();
        let enc = Encoder::new(&mut jpeg, 85);
        enc.encode(&rgb, w as u16, h as u16, ColorType::Rgb).unwrap();
        File::create(path).unwrap().write_all(&jpeg).unwrap();
        println!("  -> 已寫入快照: {}", path);
    }
}

fn main() {
    let desk = "CockpitSecretDesk";
    let _ = DesktopManager::bind_thread_to_desktop(desk);
    let pid = DesktopManager::launch_notepad_plus_plus(desk).unwrap();
    sleep(Duration::from_millis(800));

    let wins = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let np = wins.iter().find(|w| w.class_name == "Notepad++").unwrap();
    let main_hwnd = HWND(np.hwnd as _);

    // 觸發對話框
    unsafe {
        // 先輸入個字元讓它 dirty
        let mut children: Vec<Child> = Vec::new();
        let ptr = &mut children as *mut _ as isize;
        let _ = EnumChildWindows(main_hwnd, Some(enum_child), LPARAM(ptr));
        for c in &children {
            if c.class.starts_with("Scintilla") && (c.rect.right - c.rect.left > 200) {
                let _ = PostMessageW(c.hwnd, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                let _ = PostMessageW(c.hwnd, 0x0102 /* WM_CHAR */, WPARAM('A' as usize), LPARAM(1));
                break;
            }
        }
        sleep(Duration::from_millis(200));
        let _ = PostMessageW(main_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
    }

    sleep(Duration::from_millis(600));

    // 尋找對話框
    let wins_after = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let dlg_info = wins_after.iter().find(|w| w.class_name == "#32770").expect("未找到 #32770 對話框");
    let dlg_hwnd = HWND(dlg_info.hwnd as _);
    println!("找到對話框: HWND 0x{:X}", dlg_hwnd.0 as usize);

    // 枚舉對話框子控制項
    let mut dlg_children: Vec<Child> = Vec::new();
    unsafe {
        let ptr = &mut dlg_children as *mut _ as isize;
        let _ = EnumChildWindows(dlg_hwnd, Some(enum_child), LPARAM(ptr));
    }
    println!("對話框子控制項清單 (共 {} 個):", dlg_children.len());
    for c in &dlg_children {
        println!("  Child 0x{:X} Class='{}' Title='{}' Rect=({}, {}) {}x{}",
            c.hwnd.0 as usize, c.class, c.title, c.rect.left, c.rect.top,
            c.rect.right - c.rect.left, c.rect.bottom - c.rect.top);
    }

    // 測試對話框複合繪製：先 PrintWindow(dlg)，再對每個 child 呼叫 PrintWindow 或繪製按鈕
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(dlg_hwnd, &mut r);
        let dw = (r.right - r.left).max(1);
        let dh = (r.bottom - r.top).max(1);

        let hdc_dlg_screen = GetDC(dlg_hwnd);
        let hdc_mem = CreateCompatibleDC(hdc_dlg_screen);
        let hbm_mem = CreateCompatibleBitmap(hdc_dlg_screen, dw, dh);
        let _ = SelectObject(hdc_mem, hbm_mem);

        // 1. 基底對話框
        let _ = PrintWindow(dlg_hwnd, hdc_mem, 2);

        // 2. 子控制項逐一渲染
        for c in &dlg_children {
            let cw = (c.rect.right - c.rect.left).max(1);
            let ch = (c.rect.bottom - c.rect.top).max(1);
            let hdc_ch = CreateCompatibleDC(hdc_mem);
            let hbm_ch = CreateCompatibleBitmap(hdc_mem, cw, ch);
            let _ = SelectObject(hdc_ch, hbm_ch);

            // 嘗試 PrintWindow 0 或 2
            let pw_ok = PrintWindow(c.hwnd, hdc_ch, 0);
            if pw_ok.as_bool() {
                // 將子控制項 blt 至對話框局部座標
                let rel_x = c.rect.left - r.left;
                let rel_y = c.rect.top - r.top;
                let _ = BitBlt(hdc_mem, rel_x, rel_y, cw, ch, hdc_ch, 0, 0, SRCCOPY);
                println!("  -> 子控制項 '{}' PrintWindow 成功, Blt 到 ({}, {})", c.title, rel_x, rel_y);
            } else {
                println!("  -> 子控制項 '{}' PrintWindow 失敗", c.title);
            }

            let _ = DeleteDC(hdc_ch);
            let _ = DeleteObject(hbm_ch);
        }

        save_dc_jpeg(hdc_mem, hbm_mem, dw, dh,
            r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\dialog_composite_buttons_tested.jpg");

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
        let _ = ReleaseDC(dlg_hwnd, hdc_dlg_screen);
    }

    let _ = DesktopManager::kill_process(pid);
}
