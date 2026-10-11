use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsWindowVisible,
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

fn main() {
    unsafe {
        let hwnd = find_notepad();
        if hwnd.0.is_null() {
            println!("未找到 Notepad");
            return;
        }

        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        println!("Notepad HWND: 0x{:X}, 外框: [{}, {}, {}, {}] (寬: {}, 高: {})",
            hwnd.0 as usize, r.left, r.top, r.right, r.bottom, r.right - r.left, r.bottom - r.top);

        println!("\n--- 子控制項層級結構 ---");
        unsafe extern "system" fn child_cb(c: HWND, _lparam: LPARAM) -> BOOL {
            let mut cr = RECT::default();
            let _ = GetWindowRect(c, &mut cr);
            let mut cls_buf = vec![0u16; 256];
            let cls_len = GetClassNameW(c, &mut cls_buf);
            let cls_name = String::from_utf16_lossy(&cls_buf[..cls_len.max(0) as usize]);

            let mut title_buf = vec![0u16; 256];
            let title_len = GetWindowTextW(c, &mut title_buf);
            let title = String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]);

            let vis = IsWindowVisible(c).as_bool();
            println!(
                "HWND: 0x{:X} | 可見: {} | 類別: {:<35} | 標題: '{}' | 矩形: [{}, {}, {}, {}] (寬: {}, 高: {})",
                c.0 as usize, vis, cls_name, title, cr.left, cr.top, cr.right, cr.bottom, cr.right - cr.left, cr.bottom - cr.top
            );
            BOOL(1)
        }

        let _ = EnumChildWindows(hwnd, Some(child_cb), LPARAM(0));
    }
}
