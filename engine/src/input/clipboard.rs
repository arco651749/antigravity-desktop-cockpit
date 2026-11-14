use std::time::Duration;
use windows::Win32::Foundation::{BOOL, HWND};

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(hwnd_new_owner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn GetClipboardData(u_format: u32) -> isize;
    fn SetClipboardData(u_format: u32, h_mem: isize) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(u_flags: u32, dw_bytes: usize) -> isize;
    fn GlobalFree(h_mem: isize) -> isize;
    fn GlobalLock(h_mem: isize) -> *mut u8;
    fn GlobalUnlock(h_mem: isize) -> BOOL;
}

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

struct ClipboardSession;

impl ClipboardSession {
    fn open() -> Result<Self, String> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(HWND::default()) }.as_bool() {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        Err("無法開啟系統剪貼簿 (OpenClipboard 超時)".into())
    }
}

impl Drop for ClipboardSession {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// 讀取系統剪貼簿中的純文字 (CF_UNICODETEXT)
pub fn get_clipboard_text() -> Result<Option<String>, String> {
    let _session = ClipboardSession::open()?;
    unsafe {
        let h_mem = GetClipboardData(CF_UNICODETEXT);
        if h_mem == 0 {
            return Ok(None);
        }
        let ptr = GlobalLock(h_mem) as *const u16;
        if ptr.is_null() {
            return Ok(None);
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(h_mem);
        Ok(Some(text))
    }
}

/// 寫入純文字至系統剪貼簿 (CF_UNICODETEXT)
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    let _session = ClipboardSession::open()?;
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = utf16.len() * std::mem::size_of::<u16>();
    unsafe {
        let _ = EmptyClipboard();
        let h_mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if h_mem == 0 {
            return Err("GlobalAlloc 配置剪貼簿記憶體失敗".into());
        }
        let ptr = GlobalLock(h_mem) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(h_mem);
            return Err("GlobalLock 鎖定剪貼簿記憶體失敗".into());
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
        let _ = GlobalUnlock(h_mem);
        if SetClipboardData(CF_UNICODETEXT, h_mem) == 0 {
            let _ = GlobalFree(h_mem);
            return Err("SetClipboardData 寫入剪貼簿失敗".into());
        }
        Ok(())
    }
}
