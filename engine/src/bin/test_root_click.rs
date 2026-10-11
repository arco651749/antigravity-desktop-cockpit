use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, PostMessageW,
};

#[link(name = "user32")]
extern "system" {
    fn ScreenToClient(hwnd: HWND, lppoint: *mut POINT) -> BOOL;
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
        let pt = POINT { x: r.left + 30, y: r.top + 55 };

        let mut cpt = pt;
        let _ = ScreenToClient(hwnd, &mut cpt);
        let lp = LPARAM(((cpt.y as u16 as isize) << 16) | (cpt.x as u16 as isize));

        println!("發送點擊至 ROOT 視窗: HWND 0x{:X}, Client: ({}, {})", hwnd.0 as usize, cpt.x, cpt.y);
        let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1), LPARAM(0));
        let _ = PostMessageW(hwnd, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));
        let _ = PostMessageW(hwnd, 0x0200 /* WM_MOUSEMOVE */, WPARAM(0), lp);
        sleep(Duration::from_millis(50));
        let _ = PostMessageW(hwnd, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp);
        sleep(Duration::from_millis(100));
        let _ = PostMessageW(hwnd, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp);
        println!("發送完成！");
    }
}
