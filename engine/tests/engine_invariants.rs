use gui_stream_engine::desktop::DesktopWindowInfo;
use gui_stream_engine::input::InputArbiter;
use gui_stream_engine::protocol::{ClientCommand, ServerEvent};
use gui_stream_engine::uia::{normalize_ref_key, UIATreeNode, VisualMemory};
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

#[test]
fn uia_ref_key_normalization() {
    assert_eq!(normalize_ref_key("@1").as_deref(), Some("@1"));
    assert_eq!(normalize_ref_key("@e42").as_deref(), Some("@42"));
    assert_eq!(normalize_ref_key("  @7  ").as_deref(), Some("@7"));
    assert_eq!(normalize_ref_key("42"), None);
    assert_eq!(normalize_ref_key(""), None);
    assert_eq!(normalize_ref_key("Submit"), None);
}

#[test]
fn visual_memory_detects_new_window_and_dialog_diff() {
    let mem = VisualMemory::new();
    let w1 = DesktopWindowInfo {
        hwnd: 100,
        pid: 10,
        title: "Main App".into(),
        class_name: "Chrome_WidgetWin_1".into(),
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    };
    let root1 = UIATreeNode {
        ref_id: None,
        id: String::new(),
        name: "Main App".into(),
        control_type: "Window".into(),
        class_name: "Chrome_WidgetWin_1".into(),
        bounding_rect: [0, 0, 800, 600],
        center_point: [400, 300],
        is_enabled: true,
        is_focused: false,
        is_offscreen: false,
        value: None,
        toggle_state: None,
        expand_state: None,
        is_selected: None,
        supported_patterns: vec![],
        children: vec![],
    };

    mem.record_step(vec![w1.clone()], Some(root1.clone()));
    assert!(mem.compute_diff_with_previous().is_none());

    let dialog = DesktopWindowInfo {
        hwnd: 200,
        pid: 10,
        title: "Confirm Save".into(),
        class_name: "#32770".into(),
        x: 200,
        y: 200,
        width: 300,
        height: 150,
    };
    mem.record_step(vec![w1, dialog], Some(root1));
    let diff = mem.compute_diff_with_previous().expect("should produce diff");
    assert_eq!(diff.added_windows.len(), 1);
    assert_eq!(diff.new_dialogs.len(), 1);
    assert_eq!(diff.new_dialogs[0].hwnd, 200);
}

#[test]
fn protocol_uia_and_input_commands_deserialize() {
    let uia_cmd: ClientCommand = serde_json::from_str(
        r#"{"action":"uia_action","hwnd":123,"target":"@3","subaction":"set_value","value":"abc"}"#,
    )
    .unwrap();
    assert!(matches!(
        uia_cmd,
        ClientCommand::UIAAction { hwnd: Some(123), target, subaction, value: Some(v) }
        if target == "@3" && subaction == "set_value" && v == "abc"
    ));

    let mouse_cmd: ClientCommand = serde_json::from_str(
        r#"{"action":"mouse","subaction":"click","u":0.5,"v":0.25,"button":"left"}"#,
    )
    .unwrap();
    assert!(matches!(
        mouse_cmd,
        ClientCommand::Mouse { subaction, u, v, .. } if subaction == "click" && (u - 0.5).abs() < 1e-6 && (v - 0.25).abs() < 1e-6
    ));
}
