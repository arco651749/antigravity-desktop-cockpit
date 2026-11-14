use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SendMessageW,
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
    fn AttachThreadInput(idattach: u32, idattachto: u32, fattach: BOOL) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
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

        // File 選單的螢幕座標: x = r.left + 30, y = r.top + 55
        let pt = POINT { x: r.left + 30, y: r.top + 55 };

        let mut notepad_pid = 0;
        let notepad_tid = GetWindowThreadProcessId(hwnd, Some(&mut notepad_pid));
        let cur_tid = GetCurrentThreadId();
        println!("Notepad HWND: 0x{:X}, TID: {}, PID: {}", hwnd.0 as usize, notepad_tid, notepad_pid);

        // 尋找包含此點的 child
        let mut target = hwnd;
        let mut smallest = (r.right - r.left) * (r.bottom - r.top);
        unsafe extern "system" fn child_cb(c: HWND, lparam: LPARAM) -> BOOL {
            let (pt, found, area) = &mut *(lparam.0 as *mut (POINT, HWND, i32));
            if IsWindowVisible(c).as_bool() {
                let mut cr = RECT::default();
                if GetWindowRect(c, &mut cr).is_ok() {
                    if pt.x >= cr.left && pt.x < cr.right && pt.y >= cr.top && pt.y < cr.bottom {
                        let a = (cr.right - cr.left) * (cr.bottom - cr.top);
                        if a > 0 && a <= *area {
                            *area = a;
                            *found = c;
                        }
                    }
                }
            }
            BOOL(1)
        }

        let mut data = (pt, target, smallest);
        let _ = EnumChildWindows(hwnd, Some(child_cb), LPARAM(&mut data as *mut _ as isize));
        let bridge = data.1;
        println!("目標 Bridge HWND: 0x{:X}", bridge.0 as usize);

        let mut cpt_bridge = pt;
        let _ = ScreenToClient(bridge, &mut cpt_bridge);
        let lp_bridge = LPARAM(((cpt_bridge.y as u16 as isize) << 16) | (cpt_bridge.x as u16 as isize));

        let mut cpt_root = pt;
        let _ = ScreenToClient(hwnd, &mut cpt_root);
        let lp_root = LPARAM(((cpt_root.y as u16 as isize) << 16) | (cpt_root.x as u16 as isize));

        println!("Bridge Client: ({}, {}), Root Client: ({}, {})", cpt_bridge.x, cpt_bridge.y, cpt_root.x, cpt_root.y);

        // 測試 1: 附加執行緒輸入 AttachThreadInput
        let attached = AttachThreadInput(cur_tid, notepad_tid, BOOL(1)).as_bool();
        println!("AttachThreadInput: {}", attached);

        // 測試 1.1: 先發送 WM_ACTIVATE 與 WM_SETFOCUS
        let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1), LPARAM(0));
        let _ = PostMessageW(bridge, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));

        // 測試 1.2: 發送 WM_SETCURSOR
        let _ = SendMessageW(bridge, 0x0020 /* WM_SETCURSOR */, WPARAM(bridge.0 as usize), LPARAM(0x02010001));

        // 測試 1.3: 發送 WM_MOUSEMOVE 模擬游標進入
        let _ = PostMessageW(bridge, 0x0200 /* WM_MOUSEMOVE */, WPARAM(0), lp_bridge);
        sleep(Duration::from_millis(50));

        // 測試 1.4: 發送 WM_LBUTTONDOWN 與 WM_LBUTTONUP
        let _ = PostMessageW(bridge, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp_bridge);
        sleep(Duration::from_millis(100));
        let _ = PostMessageW(bridge, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp_bridge);

        if attached {
            let _ = AttachThreadInput(cur_tid, notepad_tid, BOOL(0));
        }

        println!("已完成測試 1 投遞");
    }
}
