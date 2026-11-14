use std::ptr;
use windows::core::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::System::StationsAndDesktops::*;

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
            println!("Found DXGI Output {}: '{}', Coords: {:?}", output_idx, dev_clean, out_desc.DesktopCoordinates);
            if dev_clean.starts_with(r"\\.\DISPLAY") && dev_clean != r"\\.\DISPLAY1" {
                target_output = Some(output);
            }
            output_idx += 1;
        }

        let output = match target_output {
            Some(o) => o,
            None => {
                println!("Non-primary DXGI display not found!");
                return Ok(());
            }
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
        )?;

        let device = device.unwrap();
        let context = context.unwrap();

        let output1: IDXGIOutput1 = output.cast()?;
        let dupl = output1.DuplicateOutput(&device)?;
        println!("DuplicateOutput initialized successfully for DISPLAY7!");

        // Test frame capture
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut desktop_resource: Option<IDXGIResource> = None;

        // Try AcquireNextFrame with 100ms timeout
        println!("Calling AcquireNextFrame(100ms)...");
        let hr = dupl.AcquireNextFrame(100, &mut frame_info, &mut desktop_resource);
        println!("AcquireNextFrame result: {:?}", hr);

        if let Ok(()) = hr {
            println!("Frame acquired! LastPresentTime: {:?}", frame_info.LastPresentTime);
            let resource = desktop_resource.unwrap();
            let texture2d: ID3D11Texture2D = resource.cast()?;
            let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
            texture2d.GetDesc(&mut tex_desc);
            println!("Texture size: {}x{}, Format: {:?}", tex_desc.Width, tex_desc.Height, tex_desc.Format);

            // Create staging texture to read into CPU
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: tex_desc.Width,
                Height: tex_desc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: tex_desc.Format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging_tex: Option<ID3D11Texture2D> = None;
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging_tex))?;
            let staging = staging_tex.unwrap();

            // Copy GPU to staging
            context.CopyResource(&staging, &texture2d);

            // Map and read
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            println!("Mapped successfully! pData: {:?}, RowPitch: {}", mapped.pData, mapped.RowPitch);

            // Inspect first 16 bytes
            let slice = std::slice::from_raw_parts(mapped.pData as *const u8, 16);
            println!("First 16 bytes: {:?}", slice);

            context.Unmap(&staging, 0);
            dupl.ReleaseFrame()?;
            println!("Frame released cleanly!");
        } else if let Err(e) = hr {
            println!("Frame error (could be timeout if static): {:?}", e);
        }
    }
    Ok(())
}
