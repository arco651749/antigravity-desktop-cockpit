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

pub const PRF_NONCLIENT: usize    = 0x00000002;
pub const PRF_CLIENT: usize       = 0x00000004;
pub const PRF_ERASEBKGND: usize   = 0x00000008;
pub const PRF_CHILDREN: usize     = 0x00000010;
pub const WM_PRINT: u32           = 0x0317;

fn main() {
    let desk = "CockpitSecretDesk";
    let _ = DesktopManager::bind_thread_to_desktop(desk);
    let pid = DesktopManager::launch_notepad_plus_plus(desk).unwrap();
    sleep(Duration::from_millis(800));

    let wins = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let np = wins.iter().find(|w| w.class_name == "Notepad++").unwrap();
    let main_hwnd = HWND(np.hwnd as _);

    // 鍵入字元並觸發關閉
    unsafe {
        let _ = PostMessageW(main_hwnd, 0x0102 /* WM_CHAR */, WPARAM('X' as usize), LPARAM(1));
        sleep(Duration::from_millis(150));
        let _ = PostMessageW(main_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    sleep(Duration::from_millis(600));

    let wins_after = DesktopManager::list_windows_in_desktop(desk).unwrap();
    let dlg_info = wins_after.iter().find(|w| w.class_name == "#32770").expect("未找到對話框");
    let dlg_hwnd = HWND(dlg_info.hwnd as _);

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
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

    let mut children: Vec<(HWND, String, String, RECT)> = Vec::new();
    unsafe {
        let ptr = &mut children as *mut _ as isize;
        let _ = EnumChildWindows(dlg_hwnd, Some(enum_cb), LPARAM(ptr));
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

        // 2. 子控制項使用 SendMessage(WM_PRINT)
        let flags = PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND;
        for (ch, cls, title, cr) in &children {
            let cw = (cr.right - cr.left).max(1);
            let ch_h = (cr.bottom - cr.top).max(1);
            let hdc_c = CreateCompatibleDC(hdc_mem);
            let hbm_c = CreateCompatibleBitmap(hdc_mem, cw, ch_h);
            let _ = SelectObject(hdc_c, hbm_c);

            let res = SendMessageW(*ch, WM_PRINT, WPARAM(hdc_c.0 as usize), LPARAM(flags as isize));
            let rel_x = cr.left - r.left;
            let rel_y = cr.top - r.top;
            let _ = BitBlt(hdc_mem, rel_x, rel_y, cw, ch_h, hdc_c, 0, 0, SRCCOPY);
            println!("Child '{}' ({}) WM_PRINT res: {:?}, Blt at ({}, {})", title, cls, res, rel_x, rel_y);

            let _ = DeleteDC(hdc_c);
            let _ = DeleteObject(hbm_c);
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
        let path = r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\dialog_wmprint_children.jpg";
        File::create(path).unwrap().write_all(&jpeg).unwrap();
        println!("成功儲存: {}", path);

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
        let _ = ReleaseDC(dlg_hwnd, hdc_dlg);
    }

    let _ = DesktopManager::kill_process(pid);
}
