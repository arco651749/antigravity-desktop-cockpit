use windows::core::*;
use windows::Win32::Graphics::Dxgi::*;

fn main() -> Result<()> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut a_idx = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a_idx) {
            let mut a_desc = DXGI_ADAPTER_DESC1::default();
            let _ = adapter.GetDesc1(&mut a_desc);
            let a_name = String::from_utf16_lossy(&a_desc.Description);
            println!("=== Adapter {}: '{}' ===", a_idx, a_name.trim_matches('\0'));

            let mut o_idx = 0;
            while let Ok(output) = adapter.EnumOutputs(o_idx) {
                let mut o_desc = DXGI_OUTPUT_DESC::default();
                let _ = output.GetDesc(&mut o_desc);
                let o_name = String::from_utf16_lossy(&o_desc.DeviceName);
                println!(
                    "  Output {}: '{}', Attached: {}, Coords: {:?}",
                    o_idx,
                    o_name.trim_matches('\0'),
                    o_desc.AttachedToDesktop.as_bool(),
                    o_desc.DesktopCoordinates
                );
                o_idx += 1;
            }
            if o_idx == 0 {
                println!("  (No outputs on this adapter)");
            }
            a_idx += 1;
        }
    }
    Ok(())
}
