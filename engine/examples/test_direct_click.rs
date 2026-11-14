use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
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
        println!("Notepad HWND: 0x{:X}, 外框: [{}, {}, {}, {}]", hwnd.0 as usize, r.left, r.top, r.right, r.bottom);

        // File 選單座標
        let target_x = r.left + 35;
        let target_y = r.top + 55;
        println!("目標螢幕點 (File 選單): ({}, {})", target_x, target_y);

        // 1. 記錄當前游標位置
        let mut orig_cursor = POINT::default();
        let _ = GetCursorPos(&mut orig_cursor);
        println!("原游標位置: ({}, {})", orig_cursor.x, orig_cursor.y);

        // 2. 換算 RustDesk 絕對座標
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let norm_x = ((target_x - vx) * 65536 / vw) as i32;
        let norm_y = ((target_y - vy) * 65536 / vh) as i32;

        let inputs = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: norm_x,
                        dy: norm_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: norm_x,
                        dy: norm_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: norm_x,
                        dy: norm_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];

        println!("發送 SendInput 點擊...");
        let sent = SendInput(&inputs, size_of::<INPUT>() as i32);
        println!("SendInput 傳回值: {} 個事件", sent);

        sleep(Duration::from_millis(50));

        // 3. 還原游標位置（瞬間還原，人類感覺不到游標被吸走！）
        let _ = SetCursorPos(orig_cursor.x, orig_cursor.y);
        println!("游標已成功還原至 ({}, {})", orig_cursor.x, orig_cursor.y);
    }
}
