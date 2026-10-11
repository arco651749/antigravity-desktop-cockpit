use serde::Serialize;
use std::mem::size_of;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, SRCCOPY,
};
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
    fn PrintWindow(hwnd: HWND, hdc: HDC, nflags: u32) -> BOOL;
}

pub const PW_RENDERFULLCONTENT: u32 = 2;

pub fn ensure_default_desktop() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(-4); // Per-Monitor V2 DPI awareness
        let desktop_name: Vec<u16> = "Default\0".encode_utf16().collect();
        let h_desk = OpenDesktopW(desktop_name.as_ptr(), 0, BOOL(0), 0x01FF);
        if h_desk != 0 {
            let _ = SetThreadDesktop(h_desk);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub width: i32,
    pub height: i32,
}

pub struct WindowEnumerator;

impl WindowEnumerator {
    pub fn list_windows() -> Vec<WindowInfo> {
        ensure_default_desktop();
        let mut windows = Vec::new();
        unsafe {
            let ptr = &mut windows as *mut Vec<WindowInfo> as isize;
            let _ = EnumWindows(Some(Self::enum_proc), LPARAM(ptr));
        }
        windows
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return BOOL(1);
        }

        let mut title_buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut title_buf);
        let title = String::from_utf16_lossy(&title_buf[..copied.max(0) as usize]);
        let trimmed = title.trim();

        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        if !trimmed.is_empty() && width > 100 && height > 100 {
            if trimmed != "Program Manager" && trimmed != "Settings" && trimmed != "Windows Input Experience" {
                list.push(WindowInfo {
                    hwnd: hwnd.0 as isize,
                    title: trimmed.to_string(),
                    width,
                    height,
                });
            }
        }

        BOOL(1)
    }
}

pub struct WindowCapturer {
    pub hwnd: HWND,
}

impl WindowCapturer {
    pub fn new(hwnd_val: isize) -> Self {
        Self {
            hwnd: HWND(hwnd_val as *mut _),
        }
    }

    pub fn capture_jpeg(&self, quality: u8) -> Result<(Vec<u8>, i32, i32), String> {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                return Err("視窗無效或已最小化/關閉".into());
            }

            let mut rect = RECT::default();
            if GetWindowRect(self.hwnd, &mut rect).is_err() {
                return Err("無法獲取視窗外框 RECT".into());
            }

            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                return Err("視窗尺寸為 0".into());
            }

            let hdc_window = GetWindowDC(self.hwnd);
            if hdc_window.is_invalid() {
                return Err("無法獲取視窗 DC (GetWindowDC 失敗)".into());
            }

            let hdc_mem = CreateCompatibleDC(hdc_window);
            let hbm = CreateCompatibleBitmap(hdc_window, width, height);
            let old_obj = SelectObject(hdc_mem, hbm);

            // 優先調用 PrintWindow (PW_RENDERFULLCONTENT)，可穿透被遮擋視窗
            let mut rendered = PrintWindow(self.hwnd, hdc_mem, PW_RENDERFULLCONTENT).as_bool();
            if !rendered {
                rendered = PrintWindow(self.hwnd, hdc_mem, 0).as_bool();
            }
            if !rendered {
                // 降級回退方案：直接 BitBlt
                let _ = BitBlt(hdc_mem, 0, 0, width, height, hdc_window, 0, 0, SRCCOPY);
            }

            // 準備 32-bit BGRA BITMAPINFO
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down DIB
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: (width * height * 4) as u32,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default()],
            };

            let mut raw_pixels = vec![0u8; (width * height * 4) as usize];

            let lines = GetDIBits(
                hdc_mem,
                hbm,
                0,
                height as u32,
                Some(raw_pixels.as_mut_ptr() as *mut _),
                &mut bi,
                DIB_RGB_COLORS,
            );

            // 釋放所有 GDI 繪圖控制代碼
            SelectObject(hdc_mem, old_obj);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(self.hwnd, hdc_window);

            if lines == 0 {
                return Err("GetDIBits 提取 32-bit 像素陣列失敗".into());
            }

            // 32-bit BGRA -> 24-bit RGB (零額外 padding，極速轉換)
            let mut rgb_pixels = Vec::with_capacity((width * height * 3) as usize);
            for chunk in raw_pixels.chunks_exact(4) {
                rgb_pixels.push(chunk[2]); // R
                rgb_pixels.push(chunk[1]); // G
                rgb_pixels.push(chunk[0]); // B
            }

            // JPEG 壓縮編碼
            let mut jpeg_bytes = Vec::new();
            let encoder = jpeg_encoder::Encoder::new(&mut jpeg_bytes, quality.clamp(10, 95));
            encoder
                .encode(
                    &rgb_pixels,
                    width as u16,
                    height as u16,
                    jpeg_encoder::ColorType::Rgb,
                )
                .map_err(|e| format!("JPEG 編碼失敗: {:?}", e))?;

            Ok((jpeg_bytes, width, height))
        }
    }
}
