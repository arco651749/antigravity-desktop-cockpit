use windows::core::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::StationsAndDesktops::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{mouse_event, MOUSEEVENTF_MOVE};

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
            if dev_name.trim_matches('\0') == r"\\.\DISPLAY7" {
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
        let output1: IDXGIOutput1 = output.cast()?;
        let dupl = output1.DuplicateOutput(&device)?;

        // Try AcquireNextFrame(20) immediately
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut desktop_resource: Option<IDXGIResource> = None;

        let hr1 = dupl.AcquireNextFrame(20, &mut frame_info, &mut desktop_resource);
        println!("Attempt 1 AcquireNextFrame(20ms): {:?}", hr1);
        if hr1.is_ok() {
            let _ = dupl.ReleaseFrame();
        } else {
            // Trigger DWM refresh via 0-delta mouse event
            println!("Attempt 1 timed out. Triggering DWM refresh via mouse_event(MOUSEEVENTF_MOVE, 0, 0, 0, 0)...");
            mouse_event(MOUSEEVENTF_MOVE, 0, 0, 0, 0);
            std::thread::sleep(std::time::Duration::from_millis(20));

            let hr2 = dupl.AcquireNextFrame(100, &mut frame_info, &mut desktop_resource);
            println!("Attempt 2 AcquireNextFrame(100ms) after trigger: {:?}", hr2);
            if hr2.is_ok() {
                let _ = dupl.ReleaseFrame();
                println!("===> SUCCESS: Acquired frame after DWM trigger!");
            }
        }
    }
    Ok(())
}
