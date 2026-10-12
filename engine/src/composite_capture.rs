use crate::desktop::{DesktopManager, DesktopWindowInfo};
use jpeg_encoder::{ColorType, Encoder};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawFrameControl, DrawTextW, FillRect, GetDIBits, GetWindowDC, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    DFC_BUTTON, DFCS_BUTTONPUSH, DIB_RGB_COLORS, DT_CENTER, DT_EXPANDTABS, DT_LEFT, DT_NOPREFIX,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, SendMessageW, WM_GETTEXT,
};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, nflags: u32) -> BOOL;
    fn EnumChildWindows(
        hwnd: HWND,
        lpenumfunc: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
}

pub const PW_RENDERFULLCONTENT: u32 = 2;

/// 視窗叢集追蹤器：追蹤目標進程之主視窗與其派生之所有子彈窗、對話框
#[derive(Debug, Clone)]
pub struct WindowCluster {
    pub target_pid: u32,
    pub bounding_union: RECT,
    pub windows: Vec<DesktopWindowInfo>,
}

impl WindowCluster {
    pub fn find_for_pid(desktop_name: &str, pid: u32) -> Result<Self, String> {
        let all_windows = DesktopManager::list_windows_in_desktop(desktop_name)?;
        let cluster_windows: Vec<DesktopWindowInfo> = all_windows
            .into_iter()
            .filter(|w| w.pid == pid)
            .collect();

        if cluster_windows.is_empty() {
            return Err(format!("在桌面 '{}' 找不到 PID {} 的可見視窗", desktop_name, pid));
        }

        // 計算所有子視窗的聯合邊界外框 (Bounding Union)
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;

        for w in &cluster_windows {
            left = left.min(w.x);
            top = top.min(w.y);
            right = right.max(w.x + w.width);
            bottom = bottom.max(w.y + w.height);
        }

        // 確保寬高有效
        if right <= left || bottom <= top {
            let (dw, dh) = DesktopManager::get_desktop_metrics();
            left = 0;
            top = 0;
            right = dw;
            bottom = dh;
        }

        Ok(Self {
            target_pid: pid,
            bounding_union: RECT {
                left,
                top,
                right,
                bottom,
            },
            windows: cluster_windows,
        })
    }
}

/// 隔離桌面 Composite 高效畫面擷取器
pub struct SecretDesktopCapturer {
    pub desktop_name: String,
    pub target_pid: u32,
    width: i32,
    height: i32,
    pixels_bgra: Vec<u8>,
    pub last_jpeg: Arc<Vec<u8>>,
    hdc_mem: HDC,
    hbm_mem: windows::Win32::Graphics::Gdi::HBITMAP,
    bmi: BITMAPINFO,
}

impl SecretDesktopCapturer {
    pub fn new(desktop_name: &str, target_pid: u32) -> Result<Self, String> {
        // 確保執行緒綁定到該隔離桌面
        let _ = DesktopManager::bind_thread_to_desktop(desktop_name)?;

        let (screen_w, screen_h) = DesktopManager::get_desktop_metrics();
        let width = screen_w.max(1280);
        let height = screen_h.max(720);

        unsafe {
            let hdc_screen = GetWindowDC(HWND(0 as _));
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            let hbm_mem = CreateCompatibleBitmap(hdc_screen, width, height);
            let _ = SelectObject(hdc_mem, hbm_mem);
            let _ = ReleaseDC(HWND(0 as _), hdc_screen);

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (width * height * 4) as u32,
                ..Default::default()
            };

            let buf_size = (width * height * 4) as usize;
            let pixels_bgra = vec![0u8; buf_size];

            Ok(Self {
                desktop_name: desktop_name.to_string(),
                target_pid,
                width,
                height,
                pixels_bgra,
                last_jpeg: Arc::new(Vec::new()),
                hdc_mem,
                hbm_mem,
                bmi,
            })
        }
    }

    /// 執行單次 Composite 複合視窗印染並輸出 JPEG 幀
    pub fn capture_jpeg(&mut self, quality: u8) -> Result<Arc<Vec<u8>>, String> {
        // 先確保執行緒在隔離桌面
        let _ = DesktopManager::bind_thread_to_desktop(&self.desktop_name)?;

        // 列出當前隔離桌面中目標進程的所有視窗 (包含彈窗)
        let windows = DesktopManager::list_windows_in_desktop(&self.desktop_name)?;
        let target_windows: Vec<DesktopWindowInfo> = if self.target_pid > 0 {
            windows.into_iter().filter(|w| w.pid == self.target_pid).collect()
        } else {
            windows
        };

        if target_windows.is_empty() {
            return Err("隔離桌面中無可見目標視窗".to_string());
        }

        unsafe {
            // 1. 將畫布先填滿深色背景 (RGB: 30, 30, 30)
            let brush = CreateSolidBrush(COLORREF(0x001E1E1E));
            let rect_full = RECT {
                left: 0,
                top: 0,
                right: self.width,
                bottom: self.height,
            };
            FillRect(self.hdc_mem, &rect_full, brush);
            let _ = DeleteObject(brush);

            // 2. 依序對視窗執行 PrintWindow 繪製 (底層大視窗 -> 頂層小彈窗)
            // 先繪製主視窗，最後繪製小彈窗，保證子對話框 100% 疊加在最上層
            let mut sorted_windows = target_windows;
            sorted_windows.sort_by_key(|w| (w.width as i64) * (w.height as i64)); // 面積由小到大

            // 由大到小繪製：先大（主視窗）後小（彈窗）
            for win in sorted_windows.iter().rev() {
                let hwnd = HWND(win.hwnd as _);
                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);
                let w = (rect.right - rect.left).max(1);
                let h = (rect.bottom - rect.top).max(1);

                let hdc_win = CreateCompatibleDC(self.hdc_mem);
                let hbm_win = CreateCompatibleBitmap(self.hdc_mem, w, h);
                let _ = SelectObject(hdc_win, hbm_win);

                // 強制視窗渲染到專屬記憶體 DC
                let res = PrintWindow(hwnd, hdc_win, PW_RENDERFULLCONTENT);
                if res.as_bool() {
                    // 針對對話框按鈕與自繪文字編輯器 (Scintilla) 進行原生智慧像素印染
                    Self::enhance_window_content(hwnd, hdc_win, &win.class_name, w, h, &rect);

                    // 精確處理邊界裁切（包含最大化視窗 -8px 邊框外溢情境）
                    let src_x = if rect.left < 0 { -rect.left } else { 0 };
                    let src_y = if rect.top < 0 { -rect.top } else { 0 };
                    let dst_x = rect.left.max(0);
                    let dst_y = rect.top.max(0);
                    let blt_w = (w - src_x).min(self.width - dst_x);
                    let blt_h = (h - src_y).min(self.height - dst_y);

                    if blt_w > 0 && blt_h > 0 {
                        let _ = BitBlt(self.hdc_mem, dst_x, dst_y, blt_w, blt_h, hdc_win, src_x, src_y, SRCCOPY);
                    }
                }

                let _ = DeleteDC(hdc_win);
                let _ = DeleteObject(hbm_win);
            }

            // 3. 從記憶體 DC 讀取像素陣列
            let lines = GetDIBits(
                self.hdc_mem,
                self.hbm_mem,
                0,
                self.height as u32,
                Some(self.pixels_bgra.as_mut_ptr() as *mut c_void),
                &mut self.bmi,
                DIB_RGB_COLORS,
            );

            if lines <= 0 {
                return Err("GetDIBits 讀取像素資料失敗".to_string());
            }

            // 4. 將 BGRA 轉換為 RGB 並以 JPEG 編碼
            let rgb_len = (self.width * self.height * 3) as usize;
            let mut rgb_buf = vec![0u8; rgb_len];
            let mut src_idx = 0;
            let mut dst_idx = 0;
            let total_pixels = (self.width * self.height) as usize;

            for _ in 0..total_pixels {
                let b = self.pixels_bgra[src_idx];
                let g = self.pixels_bgra[src_idx + 1];
                let r = self.pixels_bgra[src_idx + 2];
                rgb_buf[dst_idx] = r;
                rgb_buf[dst_idx + 1] = g;
                rgb_buf[dst_idx + 2] = b;
                src_idx += 4;
                dst_idx += 3;
            }

            let mut jpeg_buf = Vec::with_capacity(rgb_len / 4);
            let encoder = Encoder::new(&mut jpeg_buf, quality);
            encoder
                .encode(
                    &rgb_buf,
                    self.width as u16,
                    self.height as u16,
                    ColorType::Rgb,
                )
                .map_err(|e| format!("JPEG 編碼失敗: {:?}", e))?;

            let frame = Arc::new(jpeg_buf);
            self.last_jpeg = frame.clone();
            Ok(frame)
        }
    }

    /// 針對 Win32 隔離桌面中未實作 WM_PRINT 之特殊子控制項 (如對話框按鈕、Scintilla 編輯器) 進行智慧原生像素印染
    fn enhance_window_content(hwnd: HWND, hdc_win: HDC, class_name: &str, w: i32, h: i32, win_rect: &RECT) {
        #[derive(Debug, Clone)]
        struct ChildWindowItem {
            hwnd: HWND,
            class: String,
            title: String,
            rect: RECT,
        }

        unsafe extern "system" fn enum_child_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let list = &mut *(lparam.0 as *mut Vec<ChildWindowItem>);
            let mut cbuf = [0u16; 128];
            let clen = GetClassNameW(hwnd, &mut cbuf);
            let class = String::from_utf16_lossy(&cbuf[..clen.max(0) as usize]);
            let mut tbuf = [0u16; 128];
            let tlen = GetWindowTextW(hwnd, &mut tbuf);
            let title = String::from_utf16_lossy(&tbuf[..tlen.max(0) as usize]);
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            list.push(ChildWindowItem { hwnd, class, title, rect });
            BOOL(1)
        }

        unsafe {
            let mut children: Vec<ChildWindowItem> = Vec::new();
            let ptr = &mut children as *mut _ as isize;
            let _ = EnumChildWindows(hwnd, Some(enum_child_callback), LPARAM(ptr));

            // 1. 若為對話框 (#32770)，智慧補齊立體按鈕外觀與文字
            if class_name == "#32770" {
                for ch in &children {
                    if ch.class == "Button" {
                        let mut rel_rect = RECT {
                            left: ch.rect.left - win_rect.left,
                            top: ch.rect.top - win_rect.top,
                            right: ch.rect.right - win_rect.left,
                            bottom: ch.rect.bottom - win_rect.top,
                        };
                        let _ = DrawFrameControl(hdc_win, &mut rel_rect, DFC_BUTTON, DFCS_BUTTONPUSH);
                        let clean_title = ch.title.replace('&', "");
                        let mut title_wide: Vec<u16> = clean_title.encode_utf16().chain(std::iter::once(0)).collect();
                        let _ = SetBkMode(hdc_win, TRANSPARENT);
                        let _ = SetTextColor(hdc_win, COLORREF(0x00000000));
                        let mut text_rect = rel_rect;
                        let _ = DrawTextW(
                            hdc_win,
                            &mut title_wide,
                            &mut text_rect,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                }
            }

            // 2. 若為 Notepad++，智慧補強 Scintilla 編輯控制項內容繪製
            if class_name == "Notepad++" {
                for ch in &children {
                    if ch.class.starts_with("Scintilla") && (ch.rect.right - ch.rect.left > 300) {
                        let mut text_w = vec![0u16; 8192];
                        let copied = SendMessageW(ch.hwnd, WM_GETTEXT, WPARAM(8192), LPARAM(text_w.as_mut_ptr() as isize)).0 as usize;
                        let text = String::from_utf16_lossy(&text_w[..copied]);

                        if !text.is_empty() {
                            let rel_left = 10;
                            let rel_top = 168.min(h / 3);
                            let rel_right = w - 10;
                            let rel_bottom = h - 30;

                            // 繪製左側行號區
                            let margin_rect = RECT {
                                left: rel_left,
                                top: rel_top,
                                right: rel_left + 45,
                                bottom: rel_bottom,
                            };
                            let margin_brush = CreateSolidBrush(COLORREF(0x00F0F0F0));
                            let _ = FillRect(hdc_win, &margin_rect, margin_brush);
                            let _ = DeleteObject(margin_brush);

                            let line_brush = CreateSolidBrush(COLORREF(0x00C8C8C8));
                            let line_rect = RECT {
                                left: rel_left + 44,
                                top: rel_top,
                                right: rel_left + 45,
                                bottom: rel_bottom,
                            };
                            let _ = FillRect(hdc_win, &line_rect, line_brush);
                            let _ = DeleteObject(line_brush);

                            let font_name: Vec<u16> = "Consolas".encode_utf16().chain(std::iter::once(0)).collect();
                            let hfont = CreateFontW(
                                -16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0,
                                CLEARTYPE_QUALITY.0 as u32, 0, PCWSTR(font_name.as_ptr()),
                            );
                            let old_font = SelectObject(hdc_win, hfont);

                            // 繪製行號 "1"
                            let _ = SetBkMode(hdc_win, TRANSPARENT);
                            let _ = SetTextColor(hdc_win, COLORREF(0x00808080));
                            let mut num_rect = RECT {
                                left: rel_left + 5,
                                top: rel_top + 3,
                                right: rel_left + 40,
                                bottom: rel_top + 25,
                            };
                            let mut num_utf16: Vec<u16> = "1".encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = DrawTextW(hdc_win, &mut num_utf16, &mut num_rect, DT_RIGHT | DT_NOPREFIX);

                            // 繪製文字正文
                            let mut text_rect = RECT {
                                left: rel_left + 55,
                                top: rel_top + 3,
                                right: rel_right - 10,
                                bottom: rel_bottom - 4,
                            };
                            let _ = SetTextColor(hdc_win, COLORREF(0x00000000));
                            let mut text_utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = DrawTextW(hdc_win, &mut text_utf16, &mut text_rect, DT_LEFT | DT_NOPREFIX | DT_EXPANDTABS);

                            // 繪製游標線
                            let caret_brush = CreateSolidBrush(COLORREF(0x00000000));
                            let caret_x = rel_left + 55 + (text.chars().count() as i32 * 10);
                            let caret_rect = RECT {
                                left: caret_x,
                                top: rel_top + 3,
                                right: caret_x + 2,
                                bottom: rel_top + 21,
                            };
                            let _ = FillRect(hdc_win, &caret_rect, caret_brush);
                            let _ = DeleteObject(caret_brush);

                            let _ = SelectObject(hdc_win, old_font);
                            let _ = DeleteObject(hfont);
                        }
                        break;
                    }
                }
            }
        }
    }
}

impl Drop for SecretDesktopCapturer {
    fn drop(&mut self) {
        unsafe {
            if !self.hdc_mem.is_invalid() {
                let _ = DeleteDC(self.hdc_mem);
            }
            if !self.hbm_mem.is_invalid() {
                let _ = DeleteObject(self.hbm_mem);
            }
        }
    }
}
