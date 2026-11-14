#[path = "../capture.rs"]
mod capture;
use capture::WindowCapturer;

fn main() {
    let hwnd_val = 0x34071A;
    println!("Testing WindowCapturer on HWND 0x{:X}...", hwnd_val);
    let mut capturer = WindowCapturer::new(hwnd_val);

    println!("Calling capturer.capture_jpeg(75)...");
    let res = capturer.capture_jpeg(75);
    match res {
        Ok(Some(frame)) => {
            println!("SUCCESS: Captured frame of size {} bytes ({}x{})", frame.len(), capturer.width, capturer.height);
            std::fs::write("D:\\Users\\arco0\\.gemini\\antigravity\\brain\\a37c0d39-fc53-4582-b89e-bb8dffe1c938\\test_diag_capture.jpg", &*frame).unwrap();
        }
        Ok(None) => {
            println!("Got Ok(None) - no change");
        }
        Err(e) => {
            println!("ERROR: Capture failed with: {}", e);
        }
    }
}
