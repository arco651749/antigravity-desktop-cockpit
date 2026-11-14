use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsWindowVisible, PostMessageW,
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
        println!("Notepad HWND: 0x{:X}, 外框: [{}, {}, {}, {}]", hwnd.0 as usize, r.left, r.top, r.right, r.bottom);

        // 測試點擊 "File" 選單: x = r.left + 25 (大約 177), y = r.top + 55 (大約 199)
        let pt = POINT { x: r.left + 25, y: r.top + 55 };
        println!("目標螢幕點 (File 選單): ({}, {})", pt.x, pt.y);

        // 列出所有包含此點的視窗 (從頂層到所有子視窗)
        let mut candidates: Vec<HWND> = Vec::new();
        unsafe extern "system" fn enum_cb(c: HWND, lparam: LPARAM) -> BOOL {
            let (target_pt, list) = &mut *(lparam.0 as *mut (POINT, Vec<HWND>));
            let mut cr = RECT::default();
            if GetWindowRect(c, &mut cr).is_ok() {
                if target_pt.x >= cr.left && target_pt.x < cr.right && target_pt.y >= cr.top && target_pt.y < cr.bottom {
                    list.push(c);
                }
            }
            BOOL(1)
        }

        let mut data = (pt, candidates);
        let _ = EnumChildWindows(hwnd, Some(enum_cb), LPARAM(&mut data as *mut _ as isize));
        let hits = data.1;

        println!("包含此點的子視窗數量: {}", hits.len());
        for h in &hits {
            let mut cls_buf = vec![0u16; 256];
            let cls_len = GetClassNameW(*h, &mut cls_buf);
            let cls = String::from_utf16_lossy(&cls_buf[..cls_len.max(0) as usize]);
            let vis = IsWindowVisible(*h).as_bool();
            let mut cr = RECT::default();
            let _ = GetWindowRect(*h, &mut cr);
            println!("  -> HWND: 0x{:X} | 類別: {} | 可見: {} | 矩形: [{}, {}, {}, {}]",
                h.0 as usize, cls, vis, cr.left, cr.top, cr.right, cr.bottom);
        }

        // 測試發送點擊給每個候選者
        if let Some(target) = hits.last() {
            println!("\n嘗試點擊最小候選 HWND: 0x{:X}", target.0 as usize);
            let mut cpt = pt;
            let _ = ScreenToClient(*target, &mut cpt);
            println!("Client 座標: ({}, {})", cpt.x, cpt.y);

            let lp = LPARAM(((cpt.y as u16 as isize) << 16) | (cpt.x as u16 as isize));
            let _ = PostMessageW(hwnd, 0x0006 /* WM_ACTIVATE */, WPARAM(1), LPARAM(0));
            let _ = PostMessageW(*target, 0x0007 /* WM_SETFOCUS */, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(*target, 0x0201 /* WM_LBUTTONDOWN */, WPARAM(1), lp);
            sleep(Duration::from_millis(50));
            let _ = PostMessageW(*target, 0x0202 /* WM_LBUTTONUP */, WPARAM(0), lp);
            println!("已發送點擊至目標！");
        }
    }
}
