use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

fn main() {
    unsafe {
        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, 1920, 1080);
        let old_obj = SelectObject(hdc_mem, hbm);

        // Copy (3200, 0, 1920, 1080) from virtual desktop
        let ok = BitBlt(hdc_mem, 0, 0, 1920, 1080, hdc_screen, 3200, 0, SRCCOPY);
        println!("BitBlt result: {:?}", ok);

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 1920,
                biHeight: -1080,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 1920 * 1080 * 4,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default()],
        };

        let mut raw = vec![0u8; 1920 * 1080 * 4];
        let lines = GetDIBits(
            hdc_mem,
            hbm,
            0,
            1080,
            Some(raw.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        println!("GetDIBits lines: {}", lines);

        SelectObject(hdc_mem, old_obj);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        if lines > 0 {
            // Encode to jpeg
            let mut rgb = vec![0u8; 1920 * 1080 * 3];
            let mut dst = 0;
            for chunk in raw.chunks_exact(4) {
                rgb[dst] = chunk[2];
                rgb[dst + 1] = chunk[1];
                rgb[dst + 2] = chunk[0];
                dst += 3;
            }

            let mut jpeg = Vec::new();
            let enc = jpeg_encoder::Encoder::new(&mut jpeg, 75);
            enc.encode(&rgb, 1920, 1080, jpeg_encoder::ColorType::Rgb).unwrap();
            std::fs::write("D:\\Users\\arco0\\.gemini\\antigravity\\brain\\a37c0d39-fc53-4582-b89e-bb8dffe1c938\\test_gdi_virtual_frame.jpg", &jpeg).unwrap();
            println!("Saved 1080p frame: {} bytes", jpeg.len());
        }
    }
}
