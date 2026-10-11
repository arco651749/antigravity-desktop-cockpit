use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, SendMessageW,
};

#[link(name = "user32")]
extern "system" {
    fn OpenDesktopW(
        lpszdesktop: *const u16,
        dwflags: u32,
        finherit: BOOL,
        dwdesiredaccess: u32,
    ) -> isize;
    fn SetThreadDesktop(hdesktop: isize) -> BOOL;
}

unsafe fn find_notepad() -> HWND {
    let desktop_name: Vec<u16> = "Default\0".encode_utf16().collect();
    let h_desk = OpenDesktopW(desktop_name.as_ptr(), 0, BOOL(0), 0x01FF);
    if h_desk != 0 {
        let _ = SetThreadDesktop(h_desk);
    }

    let mut found = HWND::default();
    unsafe extern "system" fn enum_cb(h: HWND, lparam: LPARAM) -> BOOL {
        let len = GetWindowTextLengthW(h);
        if len > 0 {
            let mut buf = vec![0u16; (len + 1) as usize];
            let copied = GetWindowTextW(h, &mut buf);
            let title = String::from_utf16_lossy(&buf[..copied.max(0) as usize]);
            if title.contains("Notepad") || title.contains("記事本") {
                let p = lparam.0 as *mut HWND;
                *p = h;
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    let _ = EnumWindows(Some(enum_cb), LPARAM(&mut found as *mut _ as isize));
    found
}

fn hit_test_name(val: isize) -> &'static str {
    match val {
        0 => "HTERROR",
        1 => "HTCLIENT (客戶區)",
        2 => "HTCAPTION (標題列)",
        3 => "HTSYSMENU (系統選單)",
        4 => "HTGROWBOX (縮放邊角)",
        5 => "HTMENU (選單)",
        6 => "HTHSCROLL (水平滾動條)",
        7 => "HTVSCROLL (垂直滾動條)",
        8 => "HTMINBUTTON (最小化按鈕)",
        9 => "HTMAXBUTTON (最大化按鈕)",
        10 => "HTLEFT (左邊框)",
        11 => "HTRIGHT (右邊框)",
        12 => "HTTOP (頂部邊框)",
        13 => "HTTOPLEFT",
        14 => "HTTOPRIGHT",
        15 => "HTBOTTOM",
        16 => "HTBOTTOMLEFT",
        17 => "HTBOTTOMRIGHT",
        18 => "HTBORDER",
        20 => "HTCLOSE (關閉按鈕)",
        21 => "HTHELP",
        _ => "OTHER",
    }
}

fn main() {
    unsafe {
        let hwnd = find_notepad();
        if hwnd.0.is_null() {
            println!("未找到 Notepad");
            return;
        }

        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);

        // 測試 1: 標籤頁區域 (y = top + 20)
        let pt_tab = POINT { x: r.left + 80, y: r.top + 20 };
        let lp_tab = LPARAM(((pt_tab.y as u16 as isize) << 16) | (pt_tab.x as u16 as isize));
        let res_tab = SendMessageW(hwnd, 0x0084 /* WM_NCHITTEST */, WPARAM(0), lp_tab);
        println!("分頁標籤點 ({}, {}): WM_NCHITTEST -> {} ({})",
            pt_tab.x, pt_tab.y, res_tab.0, hit_test_name(res_tab.0));

        // 測試 2: File 選單區域 (y = top + 55)
        let pt_menu = POINT { x: r.left + 30, y: r.top + 55 };
        let lp_menu = LPARAM(((pt_menu.y as u16 as isize) << 16) | (pt_menu.x as u16 as isize));
        let res_menu = SendMessageW(hwnd, 0x0084 /* WM_NCHITTEST */, WPARAM(0), lp_menu);
        println!("File 選單點 ({}, {}): WM_NCHITTEST -> {} ({})",
            pt_menu.x, pt_menu.y, res_menu.0, hit_test_name(res_menu.0));

        // 測試 3: 格式工具列 (y = top + 55, x = middle)
        let pt_tool = POINT { x: r.left + 500, y: r.top + 55 };
        let lp_tool = LPARAM(((pt_tool.y as u16 as isize) << 16) | (pt_tool.x as u16 as isize));
        let res_tool = SendMessageW(hwnd, 0x0084 /* WM_NCHITTEST */, WPARAM(0), lp_tool);
        println!("工具列點 ({}, {}): WM_NCHITTEST -> {} ({})",
            pt_tool.x, pt_tool.y, res_tool.0, hit_test_name(res_tool.0));

        // 測試 4: 文字編輯區 (y = top + 150)
        let pt_edit = POINT { x: r.left + 100, y: r.top + 150 };
        let lp_edit = LPARAM(((pt_edit.y as u16 as isize) << 16) | (pt_edit.x as u16 as isize));
        let res_edit = SendMessageW(hwnd, 0x0084 /* WM_NCHITTEST */, WPARAM(0), lp_edit);
        println!("文字編輯區 ({}, {}): WM_NCHITTEST -> {} ({})",
            pt_edit.x, pt_edit.y, res_edit.0, hit_test_name(res_edit.0));
    }
}
