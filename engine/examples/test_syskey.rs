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

        // Win32 WM_SYSKEYDOWN 規範:
        // Bit 29: Context code (1 for ALT key pressed) -> 1 << 29 = 0x20000000
        // Repeat count: 1 (Bit 0-15)
        // Scan code for 'F': 0x21 (Bit 16-23) -> 0x21 << 16
        let alt_f_lparam_down = LPARAM(1 | (0x21 << 16) | (1 << 29));
        let alt_f_lparam_up = LPARAM(1 | (0x21 << 16) | (1 << 29) | (1 << 30) | (1 << 31));

        println!("發送 WM_SYSKEYDOWN (VK_F = 0x46) 包含 ALT 旗標至根視窗 0x{:X}...", hwnd.0 as usize);
        let _ = PostMessageW(hwnd, 0x0104 /* WM_SYSKEYDOWN */, WPARAM(0x46 /* VK_F */), alt_f_lparam_down);
        sleep(Duration::from_millis(100));
        let _ = PostMessageW(hwnd, 0x0105 /* WM_SYSKEYUP */, WPARAM(0x46 /* VK_F */), alt_f_lparam_up);

        println!("已發送！等待 500ms 檢查是否有選單視窗開啟...");
        sleep(Duration::from_millis(500));
    }
}
