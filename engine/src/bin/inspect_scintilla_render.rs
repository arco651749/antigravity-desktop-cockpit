use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetWindowDC, InvalidateRect, ReleaseDC, SelectObject, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, SendMessageW,
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

const SCI_GETTECHNOLOGY: u32 = 2631;
const SCI_SETTECHNOLOGY: u32 = 2630;
const SCI_GETTEXTLENGTH: u32 = 2183;

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
    println!("=== 測試 Scintilla Technology ===");

    let _ = DesktopManager::bind_thread_to_desktop(desktop_name).expect("綁定桌面失敗");
    let wins = DesktopManager::list_windows_in_desktop(desktop_name).expect("列出視窗失敗");
    let np_main = wins.iter().find(|w| w.class_name == "Notepad++").expect("未找到 Notepad++");
    let main_hwnd = HWND(np_main.hwnd as _);

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

    let sci = scintilla_child.expect("未找到 Scintilla 控制項");
    unsafe {
        let text_len = SendMessageW(sci.hwnd, SCI_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0;
        let current_tech = SendMessageW(sci.hwnd, SCI_GETTECHNOLOGY, WPARAM(0), LPARAM(0)).0;
        println!("Scintilla 當前字數: {}, 當前 Technology: {}", text_len, current_tech);

        println!("將 Technology 切換至 0 (GDI)...");
        let _ = SendMessageW(sci.hwnd, SCI_SETTECHNOLOGY, WPARAM(0), LPARAM(0));
        let after_tech = SendMessageW(sci.hwnd, SCI_GETTECHNOLOGY, WPARAM(0), LPARAM(0)).0;
        println!("切換後 Technology: {}", after_tech);

        let _ = InvalidateRect(sci.hwnd, None, true);
        let _ = UpdateWindow(sci.hwnd);
        let _ = InvalidateRect(main_hwnd, None, true);
        let _ = UpdateWindow(main_hwnd);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let mut r = RECT::default();
        let _ = GetWindowRect(main_hwnd, &mut r);
        let mw = (r.right - r.left).max(1);
        let mh = (r.bottom - r.top).max(1);

        let hdc_screen = GetWindowDC(HWND(0 as _));
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm_mem = CreateCompatibleBitmap(hdc_screen, mw, mh);
        let _ = SelectObject(hdc_mem, hbm_mem);

        let pw_res = PrintWindow(main_hwnd, hdc_mem, 2);
        println!("PrintWindow(PW_RENDERFULLCONTENT) res: {:?}", pw_res);
        save_hdc_to_jpeg(hdc_mem, hbm_mem, mw, mh, "probe_after_gdi_tech.jpg");

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
        let _ = ReleaseDC(HWND(0 as _), hdc_screen);
    }
}
