use serde::{Deserialize, Serialize};
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::StationsAndDesktops::{
    CreateDesktopW, EnumDesktopWindows, OpenDesktopW, SetThreadDesktop,
    DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK,
};
use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentThreadId, TerminateProcess,
    CREATE_NEW_CONSOLE, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

pub const DEFAULT_SECRET_DESKTOP: &str = "CockpitSecretDesk";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopWindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub class_name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
}

pub struct DesktopManager;

impl DesktopManager {
    /// 建立或開啟指定的隔離桌面
    pub fn create_or_open_desktop(name: &str) -> Result<HDESK, String> {
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let pcwstr = PCWSTR(name_wide.as_ptr());

        unsafe {
            // DESKTOP_ALL_ACCESS = 0x01FF
            let access = DESKTOP_ACCESS_FLAGS(0x01FF);
            let created = CreateDesktopW(
                pcwstr,
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                access.0,
                None,
            );

            if let Ok(hdesk) = created {
                println!("[DesktopManager] 成功建立隔離桌面: {}", name);
                return Ok(hdesk);
            }

            // 若已存在，則開啟
            let opened = OpenDesktopW(pcwstr, DESKTOP_CONTROL_FLAGS(0), false, access.0);
            match opened {
                Ok(hdesk) => {
                    println!("[DesktopManager] 成功開啟現有隔離桌面: {}", name);
                    Ok(hdesk)
                }
                Err(e) => Err(format!("無法建立或開啟隔離桌面 '{}': {:?}", name, e)),
            }
        }
    }

    /// 將當前執行緒綁定至指定隔離桌面
    pub fn bind_thread_to_desktop(name: &str) -> Result<HDESK, String> {
        let hdesk = Self::create_or_open_desktop(name)?;
        unsafe {
            SetThreadDesktop(hdesk).map_err(|e| {
                format!("SetThreadDesktop 綁定到 '{}' 失敗: {:?}", name, e)
            })?;
        }
        println!("[DesktopManager] 執行緒 {} 已綁定至隔離桌面: {}", unsafe { GetCurrentThreadId() }, name);
        Ok(hdesk)
    }

    /// 在指定的隔離桌面中啟動應用程式 (將進程 100% 約束於 lpDesktop)
    pub fn launch_in_desktop(
        desktop_name: &str,
        exe_path: &str,
        args: &[&str],
    ) -> Result<u32, String> {
        // 先確保該桌面存在
        let _ = Self::create_or_open_desktop(desktop_name)?;

        let mut desk_wide: Vec<u16> = desktop_name.encode_utf16().chain(std::iter::once(0)).collect();

        // 構造完整命令列: "exe_path" arg1 arg2 ...
        let full_cmd = if args.is_empty() {
            format!("\"{}\"", exe_path)
        } else {
            format!("\"{}\" {}", exe_path, args.join(" "))
        };
        let mut cmd_wide: Vec<u16> = full_cmd.encode_utf16().chain(std::iter::once(0)).collect();

        let mut si = STARTUPINFOW::default();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = PWSTR(desk_wide.as_mut_ptr());

        let mut pi = PROCESS_INFORMATION::default();

        unsafe {
            let res = CreateProcessW(
                PCWSTR::null(),
                PWSTR(cmd_wide.as_mut_ptr()),
                None,
                None,
                false,
                CREATE_NEW_CONSOLE,
                None,
                PCWSTR::null(),
                &si,
                &mut pi,
            );

            if res.is_ok() {
                let pid = pi.dwProcessId;
                let _ = CloseHandle(pi.hProcess);
                let _ = CloseHandle(pi.hThread);
                println!(
                    "[DesktopManager] 成功在隔離桌面 '{}' 啟動進程 (PID: {}): {}",
                    desktop_name, pid, full_cmd
                );
                Ok(pid)
            } else {
                Err(format!(
                    "CreateProcessW 啟動失敗: {:?}",
                    windows::core::Error::from_win32()
                ))
            }
        }
    }

    /// 專屬啟動 Notepad++: 自動探測安裝路徑並使用 -multiInst -nosession 參數
    pub fn launch_notepad_plus_plus(desktop_name: &str) -> Result<u32, String> {
        let default_paths = [
            r"C:\Program Files\Notepad++\notepad++.exe",
            r"C:\Program Files (x86)\Notepad++\notepad++.exe",
        ];

        let mut target_exe = None;
        for p in &default_paths {
            if Path::new(p).exists() {
                target_exe = Some(p.to_string());
                break;
            }
        }

        let exe = target_exe.ok_or_else(|| {
            "未在預設路徑 (C:\\Program Files\\Notepad++\\) 找到 notepad++.exe".to_string()
        })?;

        Self::launch_in_desktop(desktop_name, &exe, &["-multiInst", "-nosession"])
    }

    /// 列出指定隔離桌面上的所有可見視窗
    pub fn list_windows_in_desktop(desktop_name: &str) -> Result<Vec<DesktopWindowInfo>, String> {
        let hdesk = Self::create_or_open_desktop(desktop_name)?;
        let mut windows = Vec::new();

        unsafe {
            let ptr = &mut windows as *mut Vec<DesktopWindowInfo> as isize;
            let _ = EnumDesktopWindows(hdesk, Some(Self::enum_desktop_proc), LPARAM(ptr));
        }

        Ok(windows)
    }

    unsafe extern "system" fn enum_desktop_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<DesktopWindowInfo>);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        let len = GetWindowTextLengthW(hwnd);
        let mut title = String::new();
        if len > 0 {
            let mut title_buf = vec![0u16; (len + 1) as usize];
            let copied = GetWindowTextW(hwnd, &mut title_buf);
            title = String::from_utf16_lossy(&title_buf[..copied.max(0) as usize])
                .trim()
                .to_string();
        }

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize])
            .trim()
            .to_string();

        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        // 排除長寬過小的無意義系統隱形視窗
        if width > 10 && height > 10 && !class_name.is_empty() {
            list.push(DesktopWindowInfo {
                hwnd: hwnd.0 as isize,
                pid,
                title,
                class_name,
                width,
                height,
                x: rect.left,
                y: rect.top,
            });
        }

        BOOL(1)
    }

    /// 依 PID 強制結束進程
    pub fn kill_process(pid: u32) -> Result<(), String> {
        unsafe {
            use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE};
            let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
                .map_err(|e| format!("無法開啟 PID {}: {:?}", pid, e))?;
            let res = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
            if res.is_ok() {
                Ok(())
            } else {
                Err(format!("結束 PID {} 失敗", pid))
            }
        }
    }

    /// 取得虛擬/隔離桌面參考解析度
    pub fn get_desktop_metrics() -> (i32, i32) {
        unsafe {
            let w = GetSystemMetrics(SM_CXSCREEN);
            let h = GetSystemMetrics(SM_CYSCREEN);
            (w.max(1280), h.max(720))
        }
    }
}
