use gui_stream_engine::input::InputArbiter;
use gui_stream_engine::protocol::{ClientCommand, ServerEvent};
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::HWND;

#[test]
fn cursor_borrow_guard_restores_on_normal_drop() {
    let arbiter = InputArbiter::new();
    assert!(!arbiter.is_borrowing_cursor.load(Ordering::SeqCst));

    {
        let _guard = unsafe { arbiter.borrow_cursor(HWND::default(), false) };
        assert!(arbiter.is_borrowing_cursor.load(Ordering::SeqCst));
    }

    assert!(!arbiter.is_borrowing_cursor.load(Ordering::SeqCst));
}

#[test]
fn cursor_borrow_guard_restores_on_panic_unwind() {
    let arbiter = InputArbiter::new();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = unsafe { arbiter.borrow_cursor(HWND::default(), false) };
        assert!(arbiter.is_borrowing_cursor.load(Ordering::SeqCst));
        panic!("simulated panic during borrowed cursor");
    }));

    assert!(res.is_err());
    assert!(
        !arbiter.is_borrowing_cursor.load(Ordering::SeqCst),
        "CursorBorrowGuard 必須在 panic unwind 時歸還借用旗標"
    );
}

#[test]
fn protocol_clipboard_and_monitor_commands_deserialize() {
    let cmd1: ClientCommand = serde_json::from_str(r#"{"action":"get_clipboard"}"#).unwrap();
    assert!(matches!(cmd1, ClientCommand::GetClipboard));

    let cmd2: ClientCommand =
        serde_json::from_str(r#"{"action":"set_clipboard","text":"hello"}"#).unwrap();
    assert!(matches!(cmd2, ClientCommand::SetClipboard { text } if text == "hello"));

    let cmd3: ClientCommand = serde_json::from_str(r#"{"action":"get_monitors"}"#).unwrap();
    assert!(matches!(cmd3, ClientCommand::GetMonitors));

    let ev = ServerEvent::ok("clipboard", None, Some(serde_json::json!({"text": "ok"})));
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""status":"ok""#));
}
