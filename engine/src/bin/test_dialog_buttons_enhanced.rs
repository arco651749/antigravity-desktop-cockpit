use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    DrawFrameControl, DrawTextW, GetDC, GetDIBits, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DFC_BUTTON, DFCS_BUTTONPUSH, DIB_RGB_COLORS, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, HDC, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, PostMessageW,
    WM_CLOSE, WM_SETFOCUS,
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

fn main() {
    let desk = "CockpitSecretDesk";
    let _ = DesktopManager::bind_thread_to_desktop(desk);
    let pid = DesktopManager::launch_notepad_plus_plus(desk).unwrap();
    sleep(Duration::from_millis(800));

    let wins = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let np = wins.iter().find(|w| w.class_name == "Notepad++").unwrap();
    let main_hwnd = HWND(np.hwnd as _);

    // 取得 Scintilla 並鍵入 'A'
    unsafe extern "system" fn enum_all(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<(HWND, String, String, RECT)>);
        let mut cbuf = [0u16; 128];
        let clen = GetClassNameW(hwnd, &mut cbuf);
        let cls = String::from_utf16_lossy(&cbuf[..clen.max(0) as usize]);
        let mut tbuf = [0u16; 128];
        let tlen = GetWindowTextW(hwnd, &mut tbuf);
        let title = String::from_utf16_lossy(&tbuf[..tlen.max(0) as usize]);
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        list.push((hwnd, cls, title, r));
        BOOL(1)
    }

    let mut main_children: Vec<(HWND, String, String, RECT)> = Vec::new();
    unsafe {
        let ptr = &mut main_children as *mut _ as isize;
        let _ = EnumChildWindows(main_hwnd, Some(enum_all), LPARAM(ptr));
        for (ch, cls, _, r) in &main_children {
            if cls.starts_with("Scintilla") && (r.right - r.left > 200) {
                let _ = PostMessageW(*ch, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                let _ = PostMessageW(*ch, 0x0102 /* WM_CHAR */, WPARAM('Z' as usize), LPARAM(1));
                break;
            }
        }
        sleep(Duration::from_millis(200));
        let _ = PostMessageW(main_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    sleep(Duration::from_millis(600));

    let wins_after = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let dlg_info = wins_after.iter().find(|w| w.class_name == "#32770").expect("未找到對話框");
    let dlg_hwnd = HWND(dlg_info.hwnd as _);

    let mut dlg_children: Vec<(HWND, String, String, RECT)> = Vec::new();
    unsafe {
        let ptr = &mut dlg_children as *mut _ as isize;
        let _ = EnumChildWindows(dlg_hwnd, Some(enum_all), LPARAM(ptr));
    }

    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(dlg_hwnd, &mut r);
        let dw = (r.right - r.left).max(1);
        let dh = (r.bottom - r.top).max(1);

        let hdc_dlg = GetDC(dlg_hwnd);
        let hdc_mem = CreateCompatibleDC(hdc_dlg);
        let hbm_mem = CreateCompatibleBitmap(hdc_dlg, dw, dh);
        let _ = SelectObject(hdc_mem, hbm_mem);

        // 1. 基底對話框
        let _ = PrintWindow(dlg_hwnd, hdc_mem, 2);

        // 2. 對所有 Button 繪製標準按鈕外觀 + 文字
        for (_ch, cls, title, cr) in &dlg_children {
            if cls == "Button" {
                let mut rel_rect = RECT {
                    left: cr.left - r.left,
                    top: cr.top - r.top,
                    right: cr.right - r.left,
                    bottom: cr.bottom - r.top,
                };

                // 畫按鈕底框
                let _ = DrawFrameControl(hdc_mem, &mut rel_rect, DFC_BUTTON, DFCS_BUTTONPUSH);

                // 畫按鈕文字
                let clean_title = title.replace('&', "");
                let mut title_wide: Vec<u16> = clean_title.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = SetBkMode(hdc_mem, TRANSPARENT);
                let _ = SetTextColor(hdc_mem, COLORREF(0x00000000));
                let mut text_rect = rel_rect;
                let _ = DrawTextW(
                    hdc_mem,
                    &mut title_wide,
                    &mut text_rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                println!("成功渲染按鈕: '{}' at {:?}", title, rel_rect);
            }
        }

        // Save
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: dw,
            biHeight: -dh,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (dw * dh * 4) as u32,
            ..Default::default()
        };
        let mut bgra = vec![0u8; (dw * dh * 4) as usize];
        let _ = GetDIBits(hdc_mem, hbm_mem, 0, dh as u32, Some(bgra.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        let mut rgb = vec![0u8; (dw * dh * 3) as usize];
        let mut s = 0;
        let mut d = 0;
        for _ in 0..(dw * dh) {
            rgb[d] = bgra[s + 2];
            rgb[d + 1] = bgra[s + 1];
            rgb[d + 2] = bgra[s];
            s += 4;
            d += 3;
        }
        let mut jpeg = Vec::new();
        let enc = Encoder::new(&mut jpeg, 85);
        enc.encode(&rgb, dw as u16, dh as u16, ColorType::Rgb).unwrap();
        let path = r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\dialog_buttons_perfect_rendered.jpg";
        File::create(path).unwrap().write_all(&jpeg).unwrap();
        println!("成功儲存完美對話框快照: {}", path);

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
        let _ = ReleaseDC(dlg_hwnd, hdc_dlg);
    }

    let _ = DesktopManager::kill_process(pid);
}
