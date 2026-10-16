use serde::Serialize;
use std::mem::size_of;
use std::sync::Arc;
use windows::core::Interface;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, SRCCOPY,
};
use windows::Win32::System::StationsAndDesktops::*;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
};

#[link(name = "user32")]
extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> BOOL;
    fn PrintWindow(hwnd: HWND, hdc: HDC, nflags: u32) -> BOOL;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(
        hwnd: HWND,
        dwattribute: u32,
        pvattribute: *mut std::ffi::c_void,
        cbattribute: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(dwdesiredaccess: u32, binherithandle: BOOL, dwprocessid: u32) -> isize;
    fn QueryFullProcessImageNameW(
        hprocess: isize,
        dwflags: u32,
        lpexename: *mut u16,
        lpdwsize: *mut u32,
    ) -> BOOL;
    fn CloseHandle(hobject: isize) -> BOOL;
}

pub const PW_RENDERFULLCONTENT: u32 = 2;
const DWMWA_CLOAKED: u32 = 14;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

pub(crate) unsafe fn is_self_or_ide_window(hwnd: HWND) -> bool {
    let mut pid: u32 = 0;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return false;
    }
    if pid == std::process::id() {
        return true;
    }
    let h_proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid);
    if h_proc == 0 {
        return false;
    }
    let mut buf = [0u16; 512];
    let mut size = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(h_proc, 0, buf.as_mut_ptr(), &mut size);
    let _ = CloseHandle(h_proc);
    if !ok.as_bool() || size == 0 {
        return false;
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]).to_ascii_lowercase();
    path.ends_with("\\antigravity.exe") || path.ends_with("\\code.exe") || path.ends_with("\\gui-stream-engine.exe")
}

pub(crate) unsafe fn is_ide_window(hwnd: HWND) -> bool {
    let mut pid: u32 = 0;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    let h_proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid);
    if h_proc == 0 {
        return false;
    }
    let mut buf = [0u16; 512];
    let mut size = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(h_proc, 0, buf.as_mut_ptr(), &mut size);
    let _ = CloseHandle(h_proc);
    if !ok.as_bool() || size == 0 {
        return false;
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]).to_ascii_lowercase();
    path.ends_with("\\antigravity.exe") || path.ends_with("\\code.exe")
}

/// 確保當前執行緒已正確綁定至作用中互動桌面，並啟用 Per-Monitor V2 DPI 認知
pub fn ensure_default_desktop() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(-4);
        let desk = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x01FF));
        if let Ok(d) = desk {
            let _ = SetThreadDesktop(d);
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

        // 排除 Antigravity IDE 主視窗與串流引擎自身，防止誤把 IDE 自己派駐至虛擬螢幕造成無限鏡像！
        if is_self_or_ide_window(hwnd) {
            return BOOL(1);
        }

        // 過濾 Windows 11 DWM Cloaked (背景隱形/系統覆蓋層，例如 Click to Do / Recall / 懸浮輸入法)
        let mut cloaked: u32 = 0;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut std::ffi::c_void,
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
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
            if trimmed != "Program Manager"
                && trimmed != "Settings"
                && trimmed != "Windows Input Experience"
                && trimmed != "Click to Do"
                && trimmed != "Microsoft Text Input Application"
            {
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

/// DirectX 11 DXGI Desktop Duplication 硬體畫面擷取器 (RustDesk 同款架構)
/// 當畫面靜止時，AcquireNextFrame 在 GPU 驅動層逾時，實現真 0.00% CPU 佔用！
pub struct DxgiCapturer {
    pub display_name: String,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    staging_tex: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    rgb_pixels: Vec<u8>,
    pub last_jpeg: Arc<Vec<u8>>,
}

impl DxgiCapturer {
    pub fn new_for_display(target_display: &str) -> Result<Self, String> {
        ensure_default_desktop();
        unsafe {
            let factory: IDXGIFactory1 =
                CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1 失敗: {:?}", e))?;
            let adapter = factory
                .EnumAdapters1(0)
                .map_err(|e| format!("EnumAdapters1 失敗: {:?}", e))?;

            let mut target_output: Option<IDXGIOutput> = None;
            let mut out_width = 1920u32;
            let mut out_height = 1080u32;
            let mut found_name = String::new();

            let mut fallback_output: Option<IDXGIOutput> = None;
            let mut fallback_w = 1920u32;
            let mut fallback_h = 1080u32;
            let mut fallback_name = String::new();

            let mut output_idx = 0;
            while let Ok(output) = adapter.EnumOutputs(output_idx) {
                let out_desc = output.GetDesc().unwrap_or_default();
                let dev_name = String::from_utf16_lossy(&out_desc.DeviceName);
                let dev_clean = dev_name.trim_matches('\0');
                let w = (out_desc.DesktopCoordinates.right - out_desc.DesktopCoordinates.left).max(100) as u32;
                let h = (out_desc.DesktopCoordinates.bottom - out_desc.DesktopCoordinates.top).max(100) as u32;

                if !target_display.is_empty() && dev_clean == target_display {
                    target_output = Some(output);
                    found_name = dev_clean.to_string();
                    out_width = w;
                    out_height = h;
                    break;
                }

                if dev_clean != r"\\.\DISPLAY1" || out_desc.DesktopCoordinates.left > 0 {
                    fallback_output = Some(output);
                    fallback_name = dev_clean.to_string();
                    fallback_w = w;
                    fallback_h = h;
                }
                output_idx += 1;
            }

            let output = if let Some(o) = target_output {
                o
            } else if let Some(fo) = fallback_output {
                found_name = fallback_name;
                out_width = fallback_w;
                out_height = fallback_h;
                fo
            } else {
                return Err(format!("未找到可用 DXGI 顯示器 (請求: {})", target_display));
            };

            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice 失敗: {:?}", e))?;

            let device = device.unwrap();
            let context = context.unwrap();

            let output1: IDXGIOutput1 = output
                .cast()
                .map_err(|e| format!("Cast IDXGIOutput1 失敗: {:?}", e))?;
            let dupl = output1
                .DuplicateOutput(&device)
                .map_err(|e| format!("DuplicateOutput 失敗: {:?}", e))?;

            // 預先建立單一 Staging Texture 進行 GPU 記憶體映射
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: out_width,
                Height: out_height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging_tex: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_tex))
                .map_err(|e| format!("CreateTexture2D (staging) 失敗: {:?}", e))?;
            let staging_tex = staging_tex.unwrap();

            let rgb_size = (out_width * out_height * 3) as usize;
            let rgb_pixels = vec![0u8; rgb_size];

            println!(
                "[DxgiCapturer] 成功初始化 DXGI Desktop Duplication: {} ({}x{})",
                found_name, out_width, out_height
            );

            let mut capturer = Self {
                display_name: found_name,
                device,
                context,
                dupl,
                staging_tex,
                width: out_width,
                height: out_height,
                rgb_pixels,
                last_jpeg: Arc::new(Vec::new()),
            };

            // 預先取得第一幀畫面 (冷啟動防白屏)
            let _ = capturer.capture_jpeg(200, 75);
            Ok(capturer)
        }
    }

    /// 核心捕捉與編碼：
    /// 透過 AcquireNextFrame(timeout_ms) 等待 GPU 提交畫面：
    /// - 若畫面靜止 (Timeout)：直接回傳 Ok(None)，完全不複製記憶體、不進行 JPEG 編碼，CPU 0.00%！
    /// - 若有新畫面：複製 GPU Texture -> Staging -> 轉 RGB -> 單次 JPEG 編碼 -> 回傳 Ok(Some(...))。
    pub fn capture_jpeg(
        &mut self,
        timeout_ms: u32,
        quality: u8,
    ) -> Result<Option<Arc<Vec<u8>>>, String> {
        unsafe {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_resource: Option<IDXGIResource> = None;

            let hr = self.dupl.AcquireNextFrame(timeout_ms, &mut frame_info, &mut desktop_resource);
            match hr {
                Ok(()) => {
                    if frame_info.LastPresentTime == 0 && !self.last_jpeg.is_empty() {
                        let _ = self.dupl.ReleaseFrame();
                        return Ok(None);
                    }

                    let resource = match desktop_resource {
                        Some(r) => r,
                        None => {
                            let _ = self.dupl.ReleaseFrame();
                            return Ok(None);
                        }
                    };

                    let texture2d: ID3D11Texture2D = match resource.cast() {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = self.dupl.ReleaseFrame();
                            return Err(format!("資源轉換 ID3D11Texture2D 失敗: {:?}", e));
                        }
                    };

                    // GPU VRAM 直接複製到 Staging Texture
                    self.context.CopyResource(&self.staging_tex, &texture2d);

                    // 映射 Staging Texture 進行 CPU 讀取
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    if let Err(e) = self.context.Map(&self.staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) {
                        let _ = self.dupl.ReleaseFrame();
                        return Err(format!("Map staging texture 失敗: {:?}", e));
                    }

                    let row_pitch = mapped.RowPitch as usize;
                    let p_data = mapped.pData as *const u8;
                    let width = self.width as usize;
                    let height = self.height as usize;

                    // 高效向量化 BGRA -> RGB 轉換 (利用預分配緩衝區)
                    let rgb_buf = &mut self.rgb_pixels;
                    let mut dst_idx = 0;
                    for y in 0..height {
                        let row_ptr = p_data.add(y * row_pitch);
                        let row_slice = std::slice::from_raw_parts(row_ptr, width * 4);
                        for chunk in row_slice.chunks_exact(4) {
                            rgb_buf[dst_idx] = chunk[2];     // R
                            rgb_buf[dst_idx + 1] = chunk[1]; // G
                            rgb_buf[dst_idx + 2] = chunk[0]; // B
                            dst_idx += 3;
                        }
                    }

                    self.context.Unmap(&self.staging_tex, 0);
                    let _ = self.dupl.ReleaseFrame();

                    // 單次快速 JPEG 編碼
                    let mut jpeg_bytes = Vec::with_capacity(128 * 1024);
                    let encoder = jpeg_encoder::Encoder::new(&mut jpeg_bytes, quality.clamp(10, 95));
                    encoder
                        .encode(
                            rgb_buf,
                            width as u16,
                            height as u16,
                            jpeg_encoder::ColorType::Rgb,
                        )
                        .map_err(|e| format!("JPEG 編碼失敗: {:?}", e))?;

                    let arc_frame = Arc::new(jpeg_bytes);
                    self.last_jpeg = arc_frame.clone();
                    Ok(Some(arc_frame))
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // GPU 硬體判定畫面無任何變更：直接返回 None，CPU 0.00%！
                    Ok(None)
                }
                Err(e) => Err(format!("AcquireNextFrame 失敗: {:?}", e)),
            }
        }
    }
}

/// GDI 視窗層級軟體擷取器 (作為平滑降級備援方案)
pub struct GdiCapturer {
    pub hwnd: isize,
    pub width: i32,
    pub height: i32,
    raw_pixels: Vec<u8>,
    rgb_pixels: Vec<u8>,
    pub last_jpeg: Arc<Vec<u8>>,
    pub last_hash: u64,
}

impl GdiCapturer {
    pub fn new(hwnd_val: isize) -> Self {
        Self {
            hwnd: hwnd_val,
            width: 0,
            height: 0,
            raw_pixels: Vec::new(),
            rgb_pixels: Vec::new(),
            last_jpeg: Arc::new(Vec::new()),
            last_hash: 0,
        }
    }

    pub fn invalidate(&mut self) {
        self.last_hash = 0;
    }

    pub fn capture_jpeg(&mut self, quality: u8) -> Result<Option<Arc<Vec<u8>>>, String> {
        unsafe {
            let hwnd = HWND(self.hwnd as *mut _);
            if !IsWindowVisible(hwnd).as_bool() {
                return Err("視窗無效或已最小化/關閉".into());
            }

            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return Err("無法獲取視窗外框 RECT".into());
            }

            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                return Err("視窗尺寸為 0".into());
            }

            let hdc_window = GetWindowDC(hwnd);
            if hdc_window.is_invalid() {
                return Err("無法獲取視窗 DC (GetWindowDC 失敗)".into());
            }

            let hdc_mem = CreateCompatibleDC(hdc_window);
            let hbm = CreateCompatibleBitmap(hdc_window, width, height);
            let old_obj = SelectObject(hdc_mem, hbm);

            let mut rendered = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT).as_bool();
            if !rendered {
                rendered = PrintWindow(hwnd, hdc_mem, 0).as_bool();
            }
            if !rendered {
                let _ = BitBlt(hdc_mem, 0, 0, width, height, hdc_window, 0, 0, SRCCOPY);
            }

            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
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

            let raw_len = (width * height * 4) as usize;
            if self.raw_pixels.len() != raw_len {
                self.raw_pixels.resize(raw_len, 0);
            }

            let lines = GetDIBits(
                hdc_mem,
                hbm,
                0,
                height as u32,
                Some(self.raw_pixels.as_mut_ptr() as *mut _),
                &mut bi,
                DIB_RGB_COLORS,
            );

            SelectObject(hdc_mem, old_obj);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_window);

            if lines == 0 {
                return Err("GetDIBits 提取 32-bit 像素陣列失敗".into());
            }

            let hash = Self::compute_fast_sample_hash(&self.raw_pixels);
            if hash == self.last_hash && !self.last_jpeg.is_empty() {
                return Ok(None);
            }

            let rgb_len = (width * height * 3) as usize;
            if self.rgb_pixels.len() != rgb_len {
                self.rgb_pixels.resize(rgb_len, 0);
            }

            let mut dst = 0;
            for chunk in self.raw_pixels.chunks_exact(4) {
                self.rgb_pixels[dst] = chunk[2];
                self.rgb_pixels[dst + 1] = chunk[1];
                self.rgb_pixels[dst + 2] = chunk[0];
                dst += 3;
            }

            let mut jpeg_bytes = Vec::with_capacity(128 * 1024);
            let encoder = jpeg_encoder::Encoder::new(&mut jpeg_bytes, quality.clamp(10, 95));
            encoder
                .encode(
                    &self.rgb_pixels,
                    width as u16,
                    height as u16,
                    jpeg_encoder::ColorType::Rgb,
                )
                .map_err(|e| format!("JPEG 編碼失敗: {:?}", e))?;

            let arc_frame = Arc::new(jpeg_bytes);
            self.last_jpeg = arc_frame.clone();
            self.last_hash = hash;
            self.width = width;
            self.height = height;

            Ok(Some(arc_frame))
        }
    }

    #[inline]
    fn compute_fast_sample_hash(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for chunk in bytes.chunks_exact(32) {
            let val = u64::from_le_bytes(chunk[..8].try_into().unwrap());
            hash = hash.wrapping_mul(0x100000001b3) ^ val;
        }
        hash
    }
}

/// 混合式智慧擷取器 (Hybrid Unified Capturer)：
/// 若目標視窗位於虛擬顯示器 (例如 \\.\DISPLAY7)，優先啟動 DXGI 0% CPU 硬體管線；
/// 若位於主螢幕或 DXGI 初始化異常，無縫降級至 GDI 快速路徑。
pub struct WindowCapturer {
    pub hwnd: isize,
    pub width: i32,
    pub height: i32,
    dxgi: Option<DxgiCapturer>,
    gdi: GdiCapturer,
    pub last_jpeg: Arc<Vec<u8>>,
    pub needs_fallback_frame: bool,
}

impl WindowCapturer {
    pub fn new(hwnd_val: isize) -> Self {
        let mut dxgi = None;
        let mut last_jpeg = Arc::new(Vec::new());
        let mut gdi = GdiCapturer::new(hwnd_val);
        let is_on_virtual = Self::is_window_on_virtual_display(hwnd_val);

        if is_on_virtual {
            match DxgiCapturer::new_for_display("") {
                Ok(mut c) => {
                    println!("[WindowCapturer] 目標位於虛擬螢幕 ({})，啟用 DXGI 硬體零 CPU 桌面擷取！", c.display_name);
                    // 冷啟動重試最多 3 次，確保必定抓到含 Taskbar 之完整桌面首幀
                    for _ in 0..3 {
                        if !c.last_jpeg.is_empty() {
                            break;
                        }
                        let _ = c.capture_jpeg(150, 75);
                    }
                    if !c.last_jpeg.is_empty() {
                        last_jpeg = c.last_jpeg.clone();
                    }
                    dxgi = Some(c);
                }
                Err(e) => {
                    eprintln!("[WindowCapturer] DXGI 初始化失敗 ({})，降級至 GDI 模式", e);
                }
            }
        }

        // 僅在實體主螢幕 (非虛擬螢幕，即 dxgi is None) 且 last_jpeg 為空時，才呼叫 GDI 抓取單獨視窗
        if dxgi.is_none() && last_jpeg.is_empty() {
            if let Ok(Some(frame)) = gdi.capture_jpeg(75) {
                last_jpeg = frame;
            }
        }

        Self {
            hwnd: hwnd_val,
            width: 0,
            height: 0,
            dxgi,
            gdi,
            last_jpeg,
            needs_fallback_frame: false,
        }
    }

    pub fn is_window_on_virtual_display(hwnd_val: isize) -> bool {
        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            if windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() {
                let mut wp = windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT::default();
                wp.length = std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT>() as u32;
                if windows::Win32::UI::WindowsAndMessaging::GetWindowPlacement(hwnd, &mut wp).is_ok() {
                    let primary_w = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                        windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
                    ).max(1280);
                    let norm = wp.rcNormalPosition;
                    let cx = (norm.left + norm.right) / 2;
                    let cy = (norm.top + norm.bottom) / 2;
                    return cx >= primary_w || (cx < 0 && cx > -5000) || (cy < 0 && cy > -5000);
                }
            }
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let primary_w = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
                ).max(1280);
                // 只要視窗中心點超出主螢幕範圍（右側、左側延伸螢幕，或對角右上負數 Y 軸，排除最小化 -32000 異常值），即視為位於虛擬第二螢幕
                let center_x = (rect.left + rect.right) / 2;
                let center_y = (rect.top + rect.bottom) / 2;
                center_x >= primary_w || (center_x < 0 && center_x > -5000) || (center_y < 0 && center_y > -5000)
            } else {
                false
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.gdi.invalidate();
    }

    pub fn capture_jpeg(&mut self, quality: u8) -> Result<Option<Arc<Vec<u8>>>, String> {
        // 如果目前尚未開啟 DXGI，但視窗已被移至虛擬螢幕，動態晉升為 DXGI！
        if self.dxgi.is_none() && Self::is_window_on_virtual_display(self.hwnd) {
            if let Ok(mut c) = DxgiCapturer::new_for_display("") {
                println!("[WindowCapturer] 偵測到視窗進入虛擬螢幕 ({})，動態晉升為 DXGI 硬體桌面擷取", c.display_name);
                for _ in 0..3 {
                    if !c.last_jpeg.is_empty() {
                        break;
                    }
                    let _ = c.capture_jpeg(150, quality);
                }
                if !c.last_jpeg.is_empty() {
                    self.last_jpeg = c.last_jpeg.clone();
                }
                self.dxgi = Some(c);
            }
        }

        if let Some(dxgi) = self.dxgi.as_mut() {
            // 虛擬螢幕 DXGI 模式：設置 50ms 等待 GPU 提交新幀
            match dxgi.capture_jpeg(50, quality) {
                Ok(Some(frame)) => {
                    self.width = dxgi.width as i32;
                    self.height = dxgi.height as i32;
                    self.last_jpeg = frame.clone();
                    return Ok(Some(frame));
                }
                Ok(None) => {
                    // 畫面靜止 (GPU 無提交新幀)：保持現有完整桌面幀 (含 Taskbar)，
                    // 絕不降級至 GDI PrintWindow，防止 Taskbar 消失與畫面解析度畸變！
                    return Ok(None);
                }
                Err(e) => {
                    eprintln!("[WindowCapturer] DXGI 捕獲異常 ({:?})，嘗試重新連接虛擬螢幕 DXGI", e);
                    if let Ok(c) = DxgiCapturer::new_for_display("") {
                        *dxgi = c;
                    }
                    return Ok(None);
                }
            }
        }

        // 僅在實體螢幕純背景模式 (dxgi is None) 下使用 GDI 抓取單一視窗
        let res = self.gdi.capture_jpeg(quality);
        if let Ok(Some(ref frame)) = res {
            self.width = self.gdi.width;
            self.height = self.gdi.height;
            self.last_jpeg = frame.clone();
        }
        res
    }
}
