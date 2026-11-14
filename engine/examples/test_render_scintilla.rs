use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, nflags: u32) -> BOOL;
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, windows::Win32::Foundation::LPARAM) -> BOOL>,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> BOOL;
}

unsafe extern "system" fn enum_child(hwnd: HWND, lparam: windows::Win32::Foundation::LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<(HWND, String, RECT)>);
    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);

    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    list.push((hwnd, class_name, rect));
    BOOL(1)
}

fn capture_hwnd_to_jpeg(hwnd: HWND, filename: &str) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let w = (rect.right - rect.left).max(1);
        let h = (rect.bottom - rect.top).max(1);

        let hdc_screen = GetWindowDC(HWND(0 as _));
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm_mem = CreateCompatibleBitmap(hdc_screen, w, h);
        let _ = SelectObject(hdc_mem, hbm_mem);
        let _ = ReleaseDC(HWND(0 as _), hdc_screen);

        let ok = PrintWindow(hwnd, hdc_mem, 2 /* PW_RENDERFULLCONTENT */);
        println!("PrintWindow for HWND 0x{:X} ({}x{}): {:?}", hwnd.0 as usize, w, h, ok);

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
        let lines = GetDIBits(
            hdc_mem,
            hbm_mem,
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

        let path = format!(r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\{}", filename);
        File::create(&path).unwrap().write_all(&jpeg).unwrap();
        println!("已儲存: {}", path);

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
    }
}

fn main() {
    let desktop_name = "CockpitSecretDesk";
    let _ = DesktopManager::bind_thread_to_desktop(desktop_name);
    let wins = DesktopManager::list_windows_in_desktop(desktop_name).unwrap();

    // 尋找 Save 對話框
    if let Some(dlg) = wins.iter().find(|w| w.class_name == "#32770") {
        println!("找到 Save 對話框: HWND 0x{:X}", dlg.hwnd);
        capture_hwnd_to_jpeg(HWND(dlg.hwnd as _), "dialog_raw.jpg");

        // 枚舉對話框子控制項
        let mut children: Vec<(HWND, String, RECT)> = Vec::new();
        unsafe {
            let ptr = &mut children as *mut _ as isize;
            let _ = EnumChildWindows(HWND(dlg.hwnd as _), Some(enum_child), windows::Win32::Foundation::LPARAM(ptr));
        }
        for (ch_hwnd, cls, r) in children {
            println!("  Child: HWND 0x{:X}, Class: {}, Rect: ({}, {}) {}x{}",
                ch_hwnd.0 as usize, cls, r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
    }

    // 尋找 Notepad++ 主視窗中的 Scintilla
    if let Some(np) = wins.iter().find(|w| w.class_name == "Notepad++") {
        let mut children: Vec<(HWND, String, RECT)> = Vec::new();
        unsafe {
            let ptr = &mut children as *mut _ as isize;
            let _ = EnumChildWindows(HWND(np.hwnd as _), Some(enum_child), windows::Win32::Foundation::LPARAM(ptr));
        }
        for (ch_hwnd, cls, r) in children {
            if cls.starts_with("Scintilla") && (r.right - r.left > 200) {
                println!("測試單獨擷取 Scintilla: HWND 0x{:X}", ch_hwnd.0 as usize);
                capture_hwnd_to_jpeg(ch_hwnd, "scintilla_direct.jpg");
                break;
            }
        }
    }
}
