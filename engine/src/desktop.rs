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
    STARTF_USEPOSITION, STARTF_USESIZE, STARTF_USESHOWWINDOW,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchResult {
    pub pid: u32,
    pub hwnd: Option<isize>,
    pub title: Option<String>,
    pub target_screen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseResult {
    pub hwnd: Option<isize>,
    pub pid: Option<u32>,
    pub forced: bool,
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

    /// 啟動指定應用程式並支援直接指派至虛擬顯示器 (0 彈窗干擾)
    pub fn launch_app(
        path: &str,
        args: &[String],
        target_screen: &str,
        working_dir: Option<&str>,
        wait_for_window: bool,
        timeout_ms: u64,
    ) -> Result<LaunchResult, String> {
        crate::capture::ensure_default_desktop();

        let is_virtual = target_screen.eq_ignore_ascii_case("virtual");
        let vmon = if is_virtual {
            Some(crate::monitor::MonitorManager::ensure_virtual_monitor()?)
        } else {
            None
        };

        // 構造完整命令列: "path" arg1 arg2 ...
        let full_cmd = if path.starts_with('"') && path.ends_with('"') {
            if args.is_empty() {
                path.to_string()
            } else {
                format!("{} {}", path, args.join(" "))
            }
        } else if path.contains(' ') {
            if args.is_empty() {
                format!("\"{}\"", path)
            } else {
                format!("\"{}\" {}", path, args.join(" "))
            }
        } else {
            if args.is_empty() {
                path.to_string()
            } else {
                format!("{} {}", path, args.join(" "))
            }
        };

        let mut cmd_wide: Vec<u16> = full_cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let working_dir_wide = working_dir.map(|d| d.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>());

        let mut si = STARTUPINFOW::default();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;

        if let Some(ref vm) = vmon {
            // 直接指定視窗出生座標在虛擬第二螢幕！
            si.dwFlags = STARTF_USEPOSITION | STARTF_USESIZE | STARTF_USESHOWWINDOW;
            si.dwX = vm.x as u32;
            si.dwY = vm.y as u32;
            si.dwXSize = vm.width as u32;
            si.dwYSize = vm.height as u32;
            si.wShowWindow = windows::Win32::UI::WindowsAndMessaging::SW_SHOWMAXIMIZED.0 as u16;
        } else {
            si.dwFlags = STARTF_USESHOWWINDOW;
            si.wShowWindow = windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0 as u16;
        }

        let mut pi = PROCESS_INFORMATION::default();

        // 記錄啟動前所有頂層視窗 HWND 集合，以便啟動後精確匹配新增視窗
        let before_hwnds: std::collections::HashSet<isize> = crate::capture::WindowEnumerator::list_windows()
            .into_iter()
            .map(|w| w.hwnd)
            .collect();

        unsafe {
            let pcw_work_dir = match working_dir_wide.as_ref() {
                Some(w) => PCWSTR(w.as_ptr()),
                None => PCWSTR::null(),
            };

            let res = CreateProcessW(
                PCWSTR::null(),
                PWSTR(cmd_wide.as_mut_ptr()),
                None,
                None,
                false,
                windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(0),
                None,
                pcw_work_dir,
                &si,
                &mut pi,
            );

            if res.is_err() {
                return Err(format!(
                    "CreateProcessW 啟動失敗 (命令: {}): {:?}",
                    full_cmd,
                    windows::core::Error::from_win32()
                ));
            }

            let pid = pi.dwProcessId;
            let _ = CloseHandle(pi.hProcess);
            let _ = CloseHandle(pi.hThread);

            println!("[DesktopManager] 程式已啟動 (PID: {}): {}", pid, full_cmd);

            if !wait_for_window {
                return Ok(LaunchResult {
                    pid,
                    hwnd: None,
                    title: None,
                    target_screen: target_screen.to_string(),
                });
            }

            // 輪詢等待頂層可見視窗生成
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_millis(timeout_ms.max(500));
            let mut found_hwnd: Option<isize> = None;
            let mut found_title: Option<String> = None;

            while start.elapsed() < timeout {
                std::thread::sleep(std::time::Duration::from_millis(80));

                let current_windows = crate::capture::WindowEnumerator::list_windows();

                // 策略 1: 依 PID 精確匹配
                for w in &current_windows {
                    let hwnd = HWND(w.hwnd as *mut _);
                    let mut win_pid = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut win_pid));
                    if win_pid == pid && !w.title.is_empty() {
                        found_hwnd = Some(w.hwnd);
                        found_title = Some(w.title.clone());
                        break;
                    }
                }

                // 策略 2: 若 PID 相同未找到（例如 Chrome 代理進程架構），匹配啟動後新增之頂層視窗
                if found_hwnd.is_none() {
                    for w in &current_windows {
                        if !before_hwnds.contains(&w.hwnd) && !w.title.is_empty() && w.width > 100 && w.height > 100 {
                            found_hwnd = Some(w.hwnd);
                            found_title = Some(w.title.clone());
                            break;
                        }
                    }
                }

                if let Some(h) = found_hwnd {
                    // 若目標螢幕為 virtual，強制執行 dispatch_to_virtual 確保它在虛擬螢幕最大化並置頂重繪！
                    if is_virtual {
                        let _ = crate::monitor::MonitorManager::dispatch_to_virtual(h);
                    }
                    break;
                }
            }

            Ok(LaunchResult {
                pid,
                hwnd: found_hwnd,
                title: found_title,
                target_screen: target_screen.to_string(),
            })
        }
    }

    /// 關閉或強制終止應用程式視窗與進程
    pub fn close_app(
        target_hwnd: Option<isize>,
        target_pid: Option<u32>,
        force: bool,
        timeout_ms: u64,
    ) -> Result<CloseResult, String> {
        crate::capture::ensure_default_desktop();
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                IsWindow, PostMessageW, WM_CLOSE,
            };

            let mut actual_hwnd = target_hwnd;
            let mut actual_pid = target_pid;

            // 若有 HWND 但無 PID，反查 PID
            if let Some(h) = actual_hwnd {
                if actual_pid.is_none() {
                    let mut p = 0u32;
                    GetWindowThreadProcessId(HWND(h as *mut _), Some(&mut p));
                    if p > 0 {
                        actual_pid = Some(p);
                    }
                }
            } else if let Some(p) = actual_pid {
                // 若有 PID 但無 HWND，找該 PID 的視窗
                let wins = crate::capture::WindowEnumerator::list_windows();
                for w in wins {
                    let mut wp = 0u32;
                    GetWindowThreadProcessId(HWND(w.hwnd as *mut _), Some(&mut wp));
                    if wp == p {
                        actual_hwnd = Some(w.hwnd);
                        break;
                    }
                }
            }

            if actual_hwnd.is_none() && actual_pid.is_none() {
                return Err("未指定有效的 HWND 或 PID 進行關閉".into());
            }

            // 1. 若有視窗，先發送 WM_CLOSE 優雅退出
            if let Some(h) = actual_hwnd {
                let hwnd = HWND(h as *mut _);
                if IsWindow(hwnd).as_bool() {
                    let _ = PostMessageW(
                        hwnd,
                        WM_CLOSE,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
            }

            let mut killed = false;
            // 2. 如果要求 force，或者只有 PID 沒有視窗
            if force {
                let start = std::time::Instant::now();
                let timeout = std::time::Duration::from_millis(timeout_ms.max(300));

                while start.elapsed() < timeout {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if let Some(h) = actual_hwnd {
                        if !IsWindow(HWND(h as *mut _)).as_bool() {
                            break;
                        }
                    }
                }

                let still_alive = match actual_hwnd {
                    Some(h) => IsWindow(HWND(h as *mut _)).as_bool(),
                    None => true,
                };

                if still_alive {
                    if let Some(p) = actual_pid {
                        let _ = Self::kill_process(p);
                        killed = true;
                    }
                }
            } else if actual_hwnd.is_none() {
                // 沒有視窗只有 PID 時，非強制也必須調用 kill_process
                if let Some(p) = actual_pid {
                    Self::kill_process(p)?;
                    killed = true;
                }
            }

            Ok(CloseResult {
                hwnd: actual_hwnd,
                pid: actual_pid,
                forced: force || killed,
            })
        }
    }
}
