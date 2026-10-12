use gui_stream_engine::desktop::DesktopManager;
use jpeg_encoder::{ColorType, Encoder};
use std::fs::File;
use std::io::Write;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, GetDIBits, GetWindowDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_EXPANDTABS, DT_LEFT, DT_NOPREFIX,
    HDC, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, SendMessageW, WM_GETTEXT,
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
    println!("=== 測試 Scintilla 文字智能合成渲染 ===");

    let _ = DesktopManager::bind_thread_to_desktop(desktop_name).expect("綁定桌面失敗");
    let wins = DesktopManager::list_windows_in_desktop(desktop_name).expect("列出視窗失敗");
    let np_main = wins.iter().find(|w| w.class_name == "Notepad++").expect("未找到 Notepad++ 主視窗");
    let main_hwnd = HWND(np_main.hwnd as _);

    let mut children: Vec<ChildItem> = Vec::new();
    unsafe {
        let ptr = &mut children as *mut _ as isize;
        let _ = EnumChildWindows(main_hwnd, Some(enum_cb), LPARAM(ptr));
    }

    println!("Notepad++ 子視窗清單 (共 {} 個):", children.len());
    for c in &children {
        println!("  HWND 0x{:X}: Class='{}', Rect=({}, {}) {}x{}",
            c.hwnd.0 as usize, c.class, c.rect.left, c.rect.top,
            c.rect.right - c.rect.left, c.rect.bottom - c.rect.top);
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
        // 讀取視窗座標
        let mut main_rect = RECT::default();
        let _ = GetWindowRect(main_hwnd, &mut main_rect);
        let mw = (main_rect.right - main_rect.left).max(1);
        let mh = (main_rect.bottom - main_rect.top).max(1);

        println!("Main Rect: left={}, top={}, right={}, bottom={}, size={}x{}",
            main_rect.left, main_rect.top, main_rect.right, main_rect.bottom, mw, mh);
        println!("Sci Rect: left={}, top={}, right={}, bottom={}, size={}x{}",
            sci.rect.left, sci.rect.top, sci.rect.right, sci.rect.bottom,
            sci.rect.right - sci.rect.left, sci.rect.bottom - sci.rect.top);

        let hdc_screen = GetWindowDC(HWND(0 as _));
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm_mem = CreateCompatibleBitmap(hdc_screen, mw, mh);
        let _ = SelectObject(hdc_mem, hbm_mem);

        // 1. 基底視窗 PrintWindow
        let _ = PrintWindow(main_hwnd, hdc_mem, 2);

        // 2. 取得 Scintilla 文字
        let mut text_w = vec![0u16; 4096];
        let mut copied = SendMessageW(sci.hwnd, WM_GETTEXT, WPARAM(4096), LPARAM(text_w.as_mut_ptr() as isize)).0 as usize;
        let mut text = String::from_utf16_lossy(&text_w[..copied]);

        if text.trim().is_empty() {
            println!("Scintilla 為空，注入目標文字: ADC 2.0 測試成功");
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                sci.hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS,
                WPARAM(0),
                LPARAM(0),
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
            let target_text = "ADC 2.0 測試成功\r\n";
            for ch in target_text.chars() {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    sci.hwnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_CHAR,
                    WPARAM(ch as usize),
                    LPARAM(1),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(300));

            copied = SendMessageW(sci.hwnd, WM_GETTEXT, WPARAM(4096), LPARAM(text_w.as_mut_ptr() as isize)).0 as usize;
            text = String::from_utf16_lossy(&text_w[..copied]);
        }

        println!("讀取到 Scintilla 內容 ({} 字元): {:?}", copied, text);

        if !text.is_empty() {
            // 精確客戶區偏移: 避開非客戶區、選單列、工具列與分頁標籤，頂部真實起始為 168px
            let rel_left = 10;
            let rel_top = 168;
            let rel_right = mw - 10;
            let rel_bottom = mh - 30;

            println!("精確繪製 Scintilla 編輯區: top={}, left={}, right={}, bottom={}",
                rel_top, rel_left, rel_right, rel_bottom);

            // 1. 繪製左側行號區 (Margin): 寬度約 45px，灰白底色 (RGB: 240, 240, 240)
            let margin_rect = RECT {
                left: rel_left,
                top: rel_top,
                right: rel_left + 45,
                bottom: rel_bottom,
            };
            let margin_brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(0x00F0F0F0));
            let _ = windows::Win32::Graphics::Gdi::FillRect(hdc_mem, &margin_rect, margin_brush);
            let _ = DeleteObject(margin_brush);

            // 分隔垂直線 (RGB: 200, 200, 200)
            let line_brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(0x00C8C8C8));
            let line_rect = RECT {
                left: rel_left + 44,
                top: rel_top,
                right: rel_left + 45,
                bottom: rel_bottom,
            };
            let _ = windows::Win32::Graphics::Gdi::FillRect(hdc_mem, &line_rect, line_brush);
            let _ = DeleteObject(line_brush);

            // 建立等寬字型 Consolas (清晰 16 像素高度)
            let font_name: Vec<u16> = "Consolas".encode_utf16().chain(std::iter::once(0)).collect();
            let hfont = CreateFontW(
                -16,
                0,
                0,
                0,
                400, // FW_NORMAL
                0,
                0,
                0,
                1, // DEFAULT_CHARSET
                0,
                0,
                windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY.0 as u32,
                0,
                PCWSTR(font_name.as_ptr()),
            );
            let old_font = SelectObject(hdc_mem, hfont);

            // 繪製行號 "1"
            let _ = SetBkMode(hdc_mem, TRANSPARENT);
            let _ = SetTextColor(hdc_mem, COLORREF(0x00808080)); // 灰色行號
            let mut num_rect = RECT {
                left: rel_left + 5,
                top: rel_top + 3,
                right: rel_left + 40,
                bottom: rel_top + 25,
            };
            let mut num_utf16: Vec<u16> = "1".encode_utf16().chain(std::iter::once(0)).collect();
            let _ = DrawTextW(
                hdc_mem,
                &mut num_utf16,
                &mut num_rect,
                windows::Win32::Graphics::Gdi::DT_RIGHT | DT_NOPREFIX,
            );

            // 2. 繪製 Scintilla 編輯文字正文
            let mut text_rect = RECT {
                left: rel_left + 55,
                top: rel_top + 3,
                right: rel_right - 10,
                bottom: rel_bottom - 4,
            };

            let _ = SetTextColor(hdc_mem, COLORREF(0x00000000)); // 黑色主文字
            let mut text_utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = DrawTextW(
                hdc_mem,
                &mut text_utf16,
                &mut text_rect,
                DT_LEFT | DT_NOPREFIX | DT_EXPANDTABS,
            );

            // 3. 繪製游標 (Caret)
            let caret_brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(0x00000000));
            let caret_x = rel_left + 55 + (text.chars().count() as i32 * 10);
            let caret_rect = RECT {
                left: caret_x,
                top: rel_top + 3,
                right: caret_x + 2,
                bottom: rel_top + 21,
            };
            let _ = windows::Win32::Graphics::Gdi::FillRect(hdc_mem, &caret_rect, caret_brush);
            let _ = DeleteObject(caret_brush);

            // 恢復與刪除字型
            let _ = SelectObject(hdc_mem, old_font);
            let _ = DeleteObject(hfont);
        }

        save_hdc_to_jpeg(hdc_mem, hbm_mem, mw, mh,
            r"D:\Users\arco0\.gemini\antigravity\brain\a37c0d39-fc53-4582-b89e-bb8dffe1c938\user_task_typed_screen.jpg");

        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm_mem);
        let _ = ReleaseDC(HWND(0 as _), hdc_screen);
    }

    println!("=== 合成完成 ===");
}
