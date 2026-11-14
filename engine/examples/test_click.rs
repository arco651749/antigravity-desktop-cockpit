use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible, PostMessageW,
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
        println!("找到 Notepad HWND: 0x{:X}", hwnd.0 as usize);
        if hwnd.0.is_null() {
            return;
        }

        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        println!("Notepad 尺寸: {}x{}", w, h);

        // 點擊文字區域 (u=0.5, v=0.3)
        let sx = r.left + (w as f64 * 0.5) as i32;
        let sy = r.top + (h as f64 * 0.3) as i32;
        let screen_pt = POINT { x: sx, y: sy };
        println!("測試螢幕點: ({}, {})", sx, sy);

        // 尋找 leaf
        let mut target = hwnd;
        let mut smallest = w * h;
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

        let mut data = (screen_pt, target, smallest);
        let _ = EnumChildWindows(hwnd, Some(child_cb), LPARAM(&mut data as *mut _ as isize));
        let leaf = data.1;
        println!("葉子 HWND: 0x{:X}", leaf.0 as usize);

        let mut cpt = screen_pt;
        let _ = ScreenToClient(leaf, &mut cpt);
        println!("葉子 Client 座標: ({}, {})", cpt.x, cpt.y);

        let lp = LPARAM(((cpt.y as u16 as isize) << 16) | (cpt.x as u16 as isize));
        let _ = PostMessageW(leaf, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));
        let _ = PostMessageW(leaf, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = PostMessageW(leaf, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp);

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = PostMessageW(leaf, 0x0102 /* WM_CHAR */, WPARAM('Q' as usize), LPARAM(0));
        println!("已發送點擊與字元 'Q'");
    }
}
