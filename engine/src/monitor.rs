use serde::Serialize;
use std::mem::size_of;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    RedrawWindow, CDS_GLOBAL, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_POSITION,
    ENUM_CURRENT_SETTINGS, HDC, HMONITOR, HRGN, MONITORINFOEXW, RDW_ALLCHILDREN, RDW_ERASE,
    RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_MAXIMIZE, SW_RESTORE,
};
use windows::core::PCWSTR;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorDetail {
    pub name: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_virtual_candidate: bool,
}

// Amyuni / RustDesk IDD 裝置介面 GUID: {b5ffd75f-da40-4353-8ff8-b6daf6f1d8ca}
const GUID_DEVINTERFACE_USBMMIDD: windows::core::GUID =
    windows::core::GUID::from_u128(0xb5ffd75f_da40_4353_8ff8_b6daf6f1d8ca);
const IOCTL_PLUG_MONITOR: u32 = 2307084; // 0x23340C

const DIGCF_PRESENT: u32 = 0x00000002;
const DIGCF_DEVICEINTERFACE: u32 = 0x00000010;

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;

#[repr(C)]
struct SpDeviceInterfaceData {
    cb_size: u32,
    interface_class_guid: windows::core::GUID,
    flags: u32,
    reserved: usize,
}

#[link(name = "setupapi")]
extern "system" {
    fn SetupDiGetClassDevsW(
        class_guid: *const windows::core::GUID,
        enumerator: *const u16,
        hwnd_parent: isize,
        flags: u32,
    ) -> isize;

    fn SetupDiEnumDeviceInterfaces(
        device_info_set: isize,
        device_info_data: *const std::ffi::c_void,
        interface_class_guid: *const windows::core::GUID,
        member_index: u32,
        device_interface_data: *mut SpDeviceInterfaceData,
    ) -> BOOL;

    fn SetupDiGetDeviceInterfaceDetailW(
        device_info_set: isize,
        device_interface_data: *const SpDeviceInterfaceData,
        device_interface_detail_data: *mut u8,
        device_interface_detail_data_size: u32,
        required_size: *mut u32,
        device_info_data: *mut std::ffi::c_void,
    ) -> BOOL;

    fn SetupDiDestroyDeviceInfoList(device_info_set: isize) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        lp_file_name: *const u16,
        dw_desired_access: u32,
        dw_share_mode: u32,
        lp_security_attributes: *const std::ffi::c_void,
        dw_creation_disposition: u32,
        dw_flags_and_attributes: u32,
        h_template_file: isize,
    ) -> isize;

    fn DeviceIoControl(
        h_device: isize,
        dw_io_control_code: u32,
        lp_in_buffer: *const u8,
        n_in_buffer_size: u32,
        lp_out_buffer: *mut u8,
        n_out_buffer_size: u32,
        lp_bytes_returned: *mut u32,
        lp_overlapped: *mut std::ffi::c_void,
    ) -> BOOL;

    fn CloseHandle(h_object: isize) -> BOOL;
}

pub struct MonitorManager;

impl MonitorManager {
    pub fn list_monitors() -> Vec<MonitorDetail> {
        crate::capture::ensure_default_desktop();
        let mut list = Vec::new();
        unsafe {
            let ptr = &mut list as *mut Vec<MonitorDetail> as isize;
            let _ = EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(Self::enum_monitor_cb),
                LPARAM(ptr),
            );
        }
        list
    }

    unsafe extern "system" fn enum_monitor_cb(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<MonitorDetail>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

        let ok = GetMonitorInfoW(hmon, &mut info as *mut _ as *mut _);
        if ok.as_bool() {
            let is_primary = (info.monitorInfo.dwFlags & 1) != 0;
            let r = info.monitorInfo.rcMonitor;
            let width = r.right - r.left;
            let height = r.bottom - r.top;

            let name_end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            let name = String::from_utf16_lossy(&info.szDevice[..name_end]);

            // 非 Primary 螢幕即為延伸/虛擬螢幕目標
            let is_virtual_candidate = !is_primary;

            list.push(MonitorDetail {
                name,
                is_primary,
                x: r.left,
                y: r.top,
                width,
                height,
                is_virtual_candidate,
            });
        }
        BOOL(1)
    }

    /// 取得底層虛擬顯示器裝置介面路徑
    pub fn get_virtual_device_path() -> Option<Vec<u16>> {
        unsafe {
            let h_dev_info = SetupDiGetClassDevsW(
                &GUID_DEVINTERFACE_USBMMIDD,
                std::ptr::null(),
                0,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            );
            if h_dev_info == -1 || h_dev_info == 0 {
                return None;
            }

            let mut dev_data = SpDeviceInterfaceData {
                cb_size: size_of::<SpDeviceInterfaceData>() as u32,
                interface_class_guid: GUID_DEVINTERFACE_USBMMIDD,
                flags: 0,
                reserved: 0,
            };

            if SetupDiEnumDeviceInterfaces(
                h_dev_info,
                std::ptr::null(),
                &GUID_DEVINTERFACE_USBMMIDD,
                0,
                &mut dev_data,
            ).as_bool() == false
            {
                let _ = SetupDiDestroyDeviceInfoList(h_dev_info);
                return None;
            }

            let mut req_size = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                h_dev_info,
                &dev_data,
                std::ptr::null_mut(),
                0,
                &mut req_size,
                std::ptr::null_mut(),
            );

            if req_size == 0 {
                let _ = SetupDiDestroyDeviceInfoList(h_dev_info);
                return None;
            }

            let mut buf = vec![0u8; req_size as usize];
            // 在 64-bit 系統上 SP_DEVICE_INTERFACE_DETAIL_DATA_W cbSize 為 8
            let cb_size_val: u32 = 8;
            std::ptr::copy_nonoverlapping(
                &cb_size_val as *const u32 as *const u8,
                buf.as_mut_ptr(),
                4,
            );

            if SetupDiGetDeviceInterfaceDetailW(
                h_dev_info,
                &dev_data,
                buf.as_mut_ptr(),
                req_size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ).as_bool() == false
            {
                let _ = SetupDiDestroyDeviceInfoList(h_dev_info);
                return None;
            }

            let _ = SetupDiDestroyDeviceInfoList(h_dev_info);

            // 路徑位於 offset 4 開始的 UTF-16 字串
            let path_slice = std::slice::from_raw_parts(
                buf.as_ptr().add(4) as *const u16,
                ((req_size - 4) / 2) as usize,
            );

            let mut path_u16 = Vec::new();
            for &c in path_slice {
                path_u16.push(c);
                if c == 0 {
                    break;
                }
            }
            if path_u16.last() != Some(&0) {
                path_u16.push(0);
            }

            Some(path_u16)
        }
    }

    /// 發送二進位 IOCTL 控制虛擬顯示器 (0x10=插入, 0x00=拔除)
    pub fn send_virtual_display_ioctl(cmd_byte: u8) -> Result<(), String> {
        let path = Self::get_virtual_device_path()
            .ok_or_else(|| "系統未檢測到虛擬顯示器驅動介面 (usbmmidd)".to_string())?;

        unsafe {
            let h_dev = CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            );

            if h_dev == -1 || h_dev == 0 {
                return Err(format!("無法開啟虛擬顯示器裝置 Handle, LastError={}", std::io::Error::last_os_error()));
            }

            let cmd: [u8; 4] = [cmd_byte, 0, 0, 0];
            let mut bytes_ret = 0u32;
            let ok = DeviceIoControl(
                h_dev,
                IOCTL_PLUG_MONITOR,
                cmd.as_ptr(),
                4,
                std::ptr::null_mut(),
                0,
                &mut bytes_ret,
                std::ptr::null_mut(),
            );

            let _ = CloseHandle(h_dev);

            if ok.as_bool() {
                Ok(())
            } else {
                Err(format!("DeviceIoControl 發送失敗, LastError={}", std::io::Error::last_os_error()))
            }
        }
    }

    /// 透過程式碼插入虛擬螢幕 (0 批次檔、0 UAC 彈窗)
    pub fn plug_in_virtual_display() -> Result<(), String> {
        Self::send_virtual_display_ioctl(0x10)
    }

    /// 透過程式碼拔除虛擬螢幕 (恢復單螢幕)
    pub fn plug_out_virtual_display() -> Result<(), String> {
        Self::send_virtual_display_ioctl(0x00)
    }

    /// 將虛擬第二螢幕自動排列至主螢幕右上角對角處 (Diagonal Topology)
    /// 螢幕 1: (0, 0) 至 (W1, H1)
    /// 螢幕 2: (W1, -H2) 至 (W1 + W2, 0)
    /// 兩螢幕共享邊界長度為 0 (僅在頂點相切)，Windows 原生核心邊界直接封死實體滑鼠游標穿透！
    pub fn align_virtual_monitor_diagonally(virtual_device_name: &str) -> Result<(), String> {
        unsafe {
            let monitors = Self::list_monitors();
            let primary = monitors.iter().find(|m| m.is_primary).ok_or("找不到主螢幕")?;
            let primary_w = primary.width;

            let wide_dev: Vec<u16> = virtual_device_name.encode_utf16().chain(std::iter::once(0)).collect();
            let pcw_dev = PCWSTR(wide_dev.as_ptr());

            let mut dm = DEVMODEW::default();
            dm.dmSize = size_of::<DEVMODEW>() as u16;

            if !EnumDisplaySettingsW(pcw_dev, ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                return Err(format!("無法讀取顯示器 {} 的當前設定", virtual_device_name));
            }

            let target_x = primary_w;
            // 與主螢幕右上方預留 80 像素重疊通道，使游標在 Y=[0, 80] 區間可自由滑回主螢幕
            let target_y = -(dm.dmPelsHeight as i32) + 80;

            // 若已經處於 80px 重疊排列位置，無需重複變更
            if dm.Anonymous1.Anonymous2.dmPosition.x == target_x && dm.Anonymous1.Anonymous2.dmPosition.y == target_y {
                return Ok(());
            }

            dm.dmFields = DM_POSITION;
            dm.Anonymous1.Anonymous2.dmPosition.x = target_x;
            dm.Anonymous1.Anonymous2.dmPosition.y = target_y;

            let res = ChangeDisplaySettingsExW(
                pcw_dev,
                Some(&dm),
                None,
                CDS_UPDATEREGISTRY | CDS_GLOBAL,
                None,
            );

            if res == DISP_CHANGE_SUCCESSFUL {
                println!(
                    "[MonitorManager] 虛擬螢幕已自動對角化隔離：X={}, Y={} (原生 0 邊界物理隔離生效)",
                    target_x, target_y
                );
                Ok(())
            } else {
                Err(format!("ChangeDisplaySettingsExW 失敗，錯誤碼: {:?}", res))
            }
        }
    }

    /// 確保虛擬螢幕存在：若不存在則透過 DeviceIoControl 自動瞬態掛載，並等待 DWM 註冊
    pub fn ensure_virtual_monitor() -> Result<MonitorDetail, String> {
        let monitors = Self::list_monitors();
        if let Some(target) = monitors.iter().find(|m| m.is_virtual_candidate).cloned() {
            let _ = Self::align_virtual_monitor_diagonally(&target.name);
            // 重新查詢更新後的拓撲
            let updated = Self::list_monitors()
                .into_iter()
                .find(|m| m.name == target.name)
                .unwrap_or(target);
            return Ok(updated);
        }

        println!("[MonitorManager] 系統未檢測到虛擬螢幕，正在透過 DeviceIoControl 自動掛載 (cmd=0x10)...");
        Self::plug_in_virtual_display()?;

        // 等待 DWM 註冊並重劃分拓撲 (最多 3 秒)
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(3000) {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let mons = Self::list_monitors();
            if let Some(target) = mons.iter().find(|m| m.is_virtual_candidate).cloned() {
                let _ = Self::align_virtual_monitor_diagonally(&target.name);
                let updated = Self::list_monitors()
                    .into_iter()
                    .find(|m| m.name == target.name)
                    .unwrap_or(target);
                println!(
                    "[MonitorManager] 虛擬螢幕掛載成功: {} ({}x{} @ X={}, Y={})",
                    updated.name, updated.width, updated.height, updated.x, updated.y
                );
                return Ok(updated);
            }
        }

        Err("已發送 DeviceIoControl 插入指令，但等待 Windows 註冊虛擬螢幕超時 (3秒)".into())
    }

    pub fn dispatch_to_virtual(hwnd_val: isize) -> Result<MonitorDetail, String> {
        crate::capture::ensure_default_desktop();
        // 核心自動化：若虛擬螢幕不存在，自動透過 IOCTL 開啟並等待，徹底杜絕手動執行批次檔！
        let target = Self::ensure_virtual_monitor()?;

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            // 1. 先解除原主螢幕上的最大化/最小化鎖定，以便跨螢幕移動
            let _ = ShowWindow(hwnd, SW_RESTORE);

            // 2. 第一階段跨螢幕落點：將視窗移入虛擬螢幕內部，觸發 Windows WM_DPICHANGED (例如 200% -> 100% 縮放轉換)
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                target.x + 16,
                target.y + 16,
                (target.width - 32).max(800),
                (target.height - 32).max(600),
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );

            // 給予瀏覽器 (Chrome / Firefox) 30ms 消化跨螢幕 DPI 變更事件
            std::thread::sleep(std::time::Duration::from_millis(30));

            // 3. 第二階段：在虛擬螢幕上執行原生最大化 (SW_MAXIMIZE)，並透過 TOPMOST -> NOTOPMOST 確保 Z-Order 絕對置頂且不搶主螢幕焦點
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                target.x,
                target.y,
                target.width,
                target.height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            if SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                target.x,
                target.y,
                target.width,
                target.height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ).is_err() {
                return Err("SetWindowPos 派駐視窗至虛擬螢幕失敗".into());
            }

            // 4. 強制視窗與所有子控制項立即完整重繪，觸發 DWM 與 GDI 雙雙更新緩衝區
            let _ = RedrawWindow(
                hwnd,
                None,
                HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
        }

        Ok(target)
    }

    pub fn restore_to_primary(hwnd_val: isize) -> Result<MonitorDetail, String> {
        crate::capture::ensure_default_desktop();
        let monitors = Self::list_monitors();
        let primary = monitors
            .iter()
            .find(|m| m.is_primary)
            .cloned()
            .unwrap_or(MonitorDetail {
                name: "DISPLAY1".into(),
                is_primary: true,
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                is_virtual_candidate: false,
            });

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            if SetWindowPos(
                hwnd,
                HWND_TOP,
                primary.x + 80,
                primary.y + 80,
                (primary.width * 3 / 4).max(800),
                (primary.height * 3 / 4).max(600),
                SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            ).is_err() {
                return Err("SetWindowPos 將視窗移回主螢幕失敗".into());
            }
            let _ = RedrawWindow(
                hwnd,
                None,
                HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
        }

        Ok(primary)
    }
}
