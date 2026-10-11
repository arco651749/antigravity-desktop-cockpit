use std::thread::sleep;
use std::time::Duration;
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
        if hwnd.0.is_null() {
            println!("未找到 Notepad");
            return;
        }

        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let w = r.right - r.left;
        let h = r.bottom - r.top;

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

        // 測試 1: 點擊中央偏上 (u=0.3, v=0.25) 並輸入 'A'
        let sx1 = r.left + (w as f64 * 0.3) as i32;
        let sy1 = r.top + (h as f64 * 0.25) as i32;
        let pt1 = POINT { x: sx1, y: sy1 };

        let mut data1 = (pt1, target, smallest);
        let _ = EnumChildWindows(hwnd, Some(child_cb), LPARAM(&mut data1 as *mut _ as isize));
        let leaf1 = data1.1;

        let mut cpt1 = pt1;
        let _ = ScreenToClient(leaf1, &mut cpt1);
        let lp1 = LPARAM(((cpt1.y as u16 as isize) << 16) | (cpt1.x as u16 as isize));

        println!("點擊位置 1: leaf=0x{:X}, client=({}, {})", leaf1.0 as usize, cpt1.x, cpt1.y);
        let _ = PostMessageW(leaf1, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));
        let _ = PostMessageW(leaf1, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp1);
        sleep(Duration::from_millis(50));
        let _ = PostMessageW(leaf1, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp1);
        sleep(Duration::from_millis(50));
        let _ = PostMessageW(leaf1, 0x0102 /* WM_CHAR */, WPARAM('A' as usize), LPARAM(0));

        sleep(Duration::from_millis(500));

        // 測試 2: 點擊中央偏下 (u=0.5, v=0.5) 並輸入 'B'
        let sx2 = r.left + (w as f64 * 0.5) as i32;
        let sy2 = r.top + (h as f64 * 0.5) as i32;
        let pt2 = POINT { x: sx2, y: sy2 };

        let mut data2 = (pt2, target, smallest);
        let _ = EnumChildWindows(hwnd, Some(child_cb), LPARAM(&mut data2 as *mut _ as isize));
        let leaf2 = data2.1;

        let mut cpt2 = pt2;
        let _ = ScreenToClient(leaf2, &mut cpt2);
        let lp2 = LPARAM(((cpt2.y as u16 as isize) << 16) | (cpt2.x as u16 as isize));

        println!("點擊位置 2: leaf=0x{:X}, client=({}, {})", leaf2.0 as usize, cpt2.x, cpt2.y);
        let _ = PostMessageW(leaf2, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));
        let _ = PostMessageW(leaf2, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp2);
        sleep(Duration::from_millis(50));
        let _ = PostMessageW(leaf2, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp2);
        sleep(Duration::from_millis(50));
        let _ = PostMessageW(leaf2, 0x0102 /* WM_CHAR */, WPARAM('B' as usize), LPARAM(0));

        println!("點擊測試完成！");
    }
}
