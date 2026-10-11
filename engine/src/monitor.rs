use serde::Serialize;
use std::mem::size_of;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_TOP, SWP_SHOWWINDOW, SW_RESTORE,
};

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

    pub fn dispatch_to_virtual(hwnd_val: isize) -> Result<MonitorDetail, String> {
        crate::capture::ensure_default_desktop();
        let monitors = Self::list_monitors();
        let target = monitors
            .iter()
            .find(|m| m.is_virtual_candidate)
            .cloned()
            .ok_or_else(|| "系統未檢測到虛擬第二螢幕，請先啟用虛擬顯示器驅動".to_string())?;

        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            if SetWindowPos(
                hwnd,
                HWND_TOP,
                target.x,
                target.y,
                target.width,
                target.height,
                SWP_SHOWWINDOW,
            ).is_err() {
                return Err("SetWindowPos 派駐視窗至虛擬螢幕失敗".into());
            }
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
                SWP_SHOWWINDOW,
            ).is_err() {
                return Err("SetWindowPos 將視窗移回主螢幕失敗".into());
            }
        }

        Ok(primary)
    }
}
