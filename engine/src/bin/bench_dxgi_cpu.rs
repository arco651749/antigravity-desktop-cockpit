use std::time::{Duration, Instant};
use windows::core::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::StationsAndDesktops::*;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn get_cpu_time_ms() -> (u64, u64) {
    unsafe {
        let mut creation = Default::default();
        let mut exit = Default::default();
        let mut kernel = Default::default();
        let mut user = Default::default();
        let _ = GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user);
        let k_ms = ((kernel.dwHighDateTime as u64) << 32 | kernel.dwLowDateTime as u64) / 10_000;
        let u_ms = ((user.dwHighDateTime as u64) << 32 | user.dwLowDateTime as u64) / 10_000;
        (k_ms, u_ms)
    }
}

fn main() -> Result<()> {
    unsafe {
        let desk = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x01FF));
        if let Ok(d) = desk {
            let _ = SetThreadDesktop(d);
        }

        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let adapter = factory.EnumAdapters1(0)?;
        
        let mut target_output: Option<IDXGIOutput> = None;
        let mut output_idx = 0;
        while let Ok(output) = adapter.EnumOutputs(output_idx) {
            let out_desc = output.GetDesc().unwrap_or_default();
            let dev_name = String::from_utf16_lossy(&out_desc.DeviceName);
            let dev_clean = dev_name.trim_matches('\0');
            if dev_clean == r"\\.\DISPLAY7" {
                target_output = Some(output);
                break;
            }
            output_idx += 1;
        }

        let output = target_output.expect("DISPLAY7 not found");
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
        )?;

        let device = device.unwrap();
        let _context = context.unwrap();
        let output1: IDXGIOutput1 = output.cast()?;
        let dupl = output1.DuplicateOutput(&device)?;

        println!("[Benchmark] 開始進行 5 秒 DXGI 捕捉循環測試 (DISPLAY7)...");
        let start_time = Instant::now();
        let (k_start, u_start) = get_cpu_time_ms();
        let mut timeout_count = 0;
        let mut frame_count = 0;

        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut desktop_resource: Option<IDXGIResource> = None;

        while start_time.elapsed() < Duration::from_secs(5) {
            let res = dupl.AcquireNextFrame(100, &mut frame_info, &mut desktop_resource);
            match res {
                Ok(()) => {
                    frame_count += 1;
                    let _ = dupl.ReleaseFrame();
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    timeout_count += 1;
                }
                Err(e) => {
                    eprintln!("Acquire error: {:?}", e);
                    break;
                }
            }
        }

        let elapsed = start_time.elapsed();
        let (k_end, u_end) = get_cpu_time_ms();
        let k_used = k_end.saturating_sub(k_start);
        let u_used = u_end.saturating_sub(u_start);
        let total_cpu_ms = k_used + u_used;
        let total_wall_ms = elapsed.as_millis() as u64;
        let cpu_percent = (total_cpu_ms as f64 / total_wall_ms as f64) * 100.0;

        println!("========== DXGI 5 秒基準測試結果 ==========");
        println!("總耗時: {:.2} 秒", elapsed.as_secs_f64());
        println!("新影格次數 (Frames): {}", frame_count);
        println!("超時靜止次數 (Timeouts): {}", timeout_count);
        println!("核心態 CPU 耗時: {} ms", k_used);
        println!("使用者態 CPU 耗時: {} ms", u_used);
        println!("總計 CPU 佔用時間: {} ms", total_cpu_ms);
        println!("換算平均 CPU 使用率: {:.3}%", cpu_percent);
        println!("==========================================");
    }
    Ok(())
}
