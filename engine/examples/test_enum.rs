use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
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
    fn SetProcessDpiAwarenessContext(value: isize) -> BOOL;
}

pub fn ensure_default_desktop() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(-4); // Per-Monitor V2
        let desktop_name: Vec<u16> = "Default\0".encode_utf16().collect();
        let h_desk = OpenDesktopW(desktop_name.as_ptr(), 0, BOOL(0), 0x01FF);
        println!("OpenDesktopW handle: 0x{:X}", h_desk);
        if h_desk != 0 {
            let ok = SetThreadDesktop(h_desk);
            println!("SetThreadDesktop success: {:?}", ok);
        }
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let visible = IsWindowVisible(hwnd).as_bool();
    let len = GetWindowTextLengthW(hwnd);
    if len > 0 {
        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..copied.max(0) as usize]);
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        if visible && rect.right - rect.left > 100 {
            println!(
                "-> HWND: 0x{:X} | Rect: {}x{} | Title: {}",
                hwnd.0 as usize,
                rect.right - rect.left,
                rect.bottom - rect.top,
                title
            );
        }
    }
    BOOL(1)
}

fn main() {
    println!("--- Testing EnumWindows with Default Desktop ---");
    ensure_default_desktop();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(0));
    }
    println!("--- Done ---");
}
