use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, PostMessageW,
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

        println!("找到 Notepad HWND: 0x{:X}", hwnd.0 as usize);

        // 1. 激活視窗
        let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1), LPARAM(0));
        sleep(Duration::from_millis(50));

        // 2. Alt Down (VK_MENU = 0x12)
        // Scan code for Alt is 0x38. Bit 29 (context code) = 1.
        let lp_alt_down = LPARAM(1 | (0x38 << 16) | (1 << 29));
        println!("發送 Alt Down 至 0x{:X}", hwnd.0 as usize);
        let _ = PostMessageW(hwnd, 0x0104 /* WM_SYSKEYDOWN */, WPARAM(0x12), lp_alt_down);
        sleep(Duration::from_millis(50));

        // 3. F Down (VK_F = 0x46)
        // Scan code for F is 0x21. Bit 29 = 1.
        let lp_f_down = LPARAM(1 | (0x21 << 16) | (1 << 29));
        println!("發送 F Down 至 0x{:X}", hwnd.0 as usize);
        let _ = PostMessageW(hwnd, 0x0104 /* WM_SYSKEYDOWN */, WPARAM(0x46), lp_f_down);
        sleep(Duration::from_millis(50));

        // 4. F Up
        let lp_f_up = LPARAM(1 | (0x21 << 16) | (1 << 29) | (1 << 30) | (1 << 31));
        let _ = PostMessageW(hwnd, 0x0105 /* WM_SYSKEYUP */, WPARAM(0x46), lp_f_up);
        sleep(Duration::from_millis(50));

        // 5. Alt Up
        let lp_alt_up = LPARAM(1 | (0x38 << 16) | (1 << 30) | (1 << 31));
        let _ = PostMessageW(hwnd, 0x0101 /* WM_KEYUP */, WPARAM(0x12), lp_alt_up);

        println!("序列發送完畢！");
    }
}
