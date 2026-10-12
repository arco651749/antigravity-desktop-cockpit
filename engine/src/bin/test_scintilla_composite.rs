use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, PostMessageW, SendMessageW,
    WM_CHAR, WM_SETFOCUS,
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

pub const PRF_CHECKVISIBLE: usize = 0x00000001;
pub const PRF_NONCLIENT: usize    = 0x00000002;
pub const PRF_CLIENT: usize       = 0x00000004;
pub const PRF_ERASEBKGND: usize   = 0x00000008;
pub const PRF_CHILDREN: usize     = 0x00000010;
pub const PRF_OWNED: usize        = 0x00000020;
pub const WM_PRINT: u32           = 0x0317;
pub const WM_PRINTCLIENT: u32     = 0x0318;

#[derive(Debug, Clone)]
struct ChildItem {
    hwnd: HWND,
    class: String,
    rect: RECT,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<ChildItem>);
    let mut class_buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_buf);
    let class = String::from_utf16_lossy(&class_buf[..len.max(0) as usize]);
    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    list.push(ChildItem { hwnd, class, rect });
    BOOL(1)
}

fn save_hdc_to_jpeg(hdc: HDC, hbm: windows::Win32::Graphics::Gdi::HBITMAP, w: i32, h: i32, path: &str) {
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
        println!("  -> 成功儲存: {}", path);
    }
}

fn main() {
    let desktop_name = "CockpitSecretDesk";
    println!("=== 啟動 Notepad++ 並測試 Scintilla 遞迴渲染 ===");

    let _ = DesktopManager::bind_thread_to_desktop(desktop_name);
    let pid = DesktopManager::launch_notepad_plus_plus(desktop_name).expect("啟動 Notepad++ 失敗");
    sleep(Duration::from_millis(1000));

    let wins = DesktopManager::list_windows_in_desktop(desktop_name).unwrap();
    let np_main = wins.iter().find(|w| w.class_name == "Notepad++").expect("未找到主視窗");
    let main_hwnd = HWND(np_main.hwnd as _);

    // 尋找 Scintilla
    let mut children: Vec<ChildItem> = Vec::new();
    unsafe {
        let ptr = &mut children as *mut _ as isize;
        let _ = EnumChildWindows(main_hwnd, Some(enum_cb), LPARAM(ptr));
    }

    let mut scintilla_child = None;
    for c in &children {
        if c.class.starts_with("Scintilla") && (c.rect.right - c.rect.left > 300) {
            scintilla_child = Some(c.clone());
            break;
        }
    }

    let sci = scintilla_child.expect("未找到主 Scintilla");
    println!("找到 Scintilla: HWND 0x{:X}, Rect: ({}, {}) {}x{}",
        sci.hwnd.0 as usize, sci.rect.left, sci.rect.top, sci.rect.right - sci.rect.left, sci.rect.bottom - sci.rect.top);

    // 鍵入大字號文字
    unsafe {
        PostMessageW(sci.hwnd, WM_SETFOCUS, WPARAM(0), LPARAM(0));
        sleep(Duration::from_millis(50));
        let text = "========================================\r\nANTIGRAVITY DESKTOP COCKPIT 2.0 (ADC)\r\n100% Native Rust Zero-Interference Control\r\n========================================\r\n";
        for ch in text.chars() {
            let _ = PostMessageW(sci.hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(1));
        }
    }
    sleep(Duration::from_millis(400));

    unsafe {
        let (screen_w, screen_h) = DesktopManager::get_desktop_metrics();
        let hdc_screen = GetWindowDC(HWND(0 as _));

        // 測試 1: 傳統 PrintWindow(main_hwnd)
        let hdc_mem1 = CreateCompatibleDC(hdc_screen);
        let hbm_mem1 = CreateCompatibleBitmap(hdc_screen, screen_w, screen_h);
        let _ = SelectObject(hdc_mem1, hbm_mem1);
        let _ = PrintWindow(main_hwnd, hdc_mem1, 2);
        save_hdc_to_jpeg(hdc_mem1, hbm_mem1, screen_w, screen_h,
            r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\test_render_method1_main_pw.jpg");
        let _ = DeleteDC(hdc_mem1);
        let _ = DeleteObject(hbm_mem1);

        // 測試 2: 對 Scintilla 呼叫 PrintWindow
        let sci_w = (sci.rect.right - sci.rect.left).max(1);
        let sci_h = (sci.rect.bottom - sci.rect.top).max(1);
        let hdc_mem2 = CreateCompatibleDC(hdc_screen);
        let hbm_mem2 = CreateCompatibleBitmap(hdc_screen, sci_w, sci_h);
        let _ = SelectObject(hdc_mem2, hbm_mem2);
        let res2 = PrintWindow(sci.hwnd, hdc_mem2, 2);
        println!("PrintWindow(scintilla) result: {:?}", res2);
        save_hdc_to_jpeg(hdc_mem2, hbm_mem2, sci_w, sci_h,
            r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\test_render_method2_sci_pw.jpg");
        let _ = DeleteDC(hdc_mem2);
        let _ = DeleteObject(hbm_mem2);

        // 測試 3: 對 Scintilla 發送 SendMessageW(WM_PRINT, ...)
        let hdc_mem3 = CreateCompatibleDC(hdc_screen);
        let hbm_mem3 = CreateCompatibleBitmap(hdc_screen, sci_w, sci_h);
        let _ = SelectObject(hdc_mem3, hbm_mem3);
        let flags = PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND | PRF_CHILDREN;
        let res3 = SendMessageW(sci.hwnd, WM_PRINT, WPARAM(hdc_mem3.0 as usize), LPARAM(flags as isize));
        println!("SendMessageW(WM_PRINT, scintilla) result: {:?}", res3);
        save_hdc_to_jpeg(hdc_mem3, hbm_mem3, sci_w, sci_h,
            r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\test_render_method3_sci_wmprint.jpg");
        let _ = DeleteDC(hdc_mem3);
        let _ = DeleteObject(hbm_mem3);

        let _ = ReleaseDC(HWND(0 as _), hdc_screen);
    }

    let _ = DesktopManager::kill_process(pid);
    println!("=== 測試結束 ===");
}
