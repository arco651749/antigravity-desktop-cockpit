import asyncio
import json
import shlex
import sys

from ws_client import (
    ensure_virtual_display_on,
    list_system_monitors,
    parse_hwnd,
    plug_out_virtual_display,
    send_cmd,
)

HANDLED_COMMANDS = {"list", "dispatch", "restore", "set", "capture", "display", "launch", "close"}


def handle(args):
    if args.command == "list":
        res = asyncio.run(send_cmd({"action": "get_windows"}, wait_for_event="windows"))
        wins = res.get("data", []) if res else []
        if args.filter:
            wins = [w for w in wins if args.filter.lower() in w.get("title", "").lower()]
        for w in wins:
            hwnd_hex = f"0x{w['hwnd']:X}"
            print(f"[{hwnd_hex}] {w['title']} ({w['width']}x{w['height']})")

    elif args.command == "dispatch":
        hwnd = parse_hwnd(args.hwnd)
        try:
            ensure_virtual_display_on()
        except Exception as e:
            print(f"[WARN] 虛擬螢幕自動掛載防呆提示: {e}")
        res = asyncio.run(send_cmd({"action": "dispatch_to_virtual", "hwnd": hwnd}, wait_for_event="window_dispatched"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "restore":
        hwnd = parse_hwnd(args.hwnd)
        res = asyncio.run(send_cmd({"action": "restore_to_primary", "hwnd": hwnd}, wait_for_event="window_restored"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "set":
        hwnd = parse_hwnd(args.hwnd)
        res = asyncio.run(send_cmd({"action": "set_window", "hwnd": hwnd}, wait_for_event="window_set"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "capture":
        cmd = {"action": "set_window", "hwnd": parse_hwnd(args.hwnd)} if args.hwnd else None
        asyncio.run(send_cmd(cmd, capture_frame_path=args.output, timeout=3.0))

    elif args.command == "display":
        if args.action == "status":
            mons = list_system_monitors()
            print("[DISPLAY] 系統顯示器拓撲清單:")
            for m in mons:
                tag = "主要螢幕 🏠" if m["primary"] else "虛擬第二螢幕 🟢"
                print(f"  - {m['name']}: {m['width']}x{m['height']} @ ({m['x']}, {m['y']}) [{tag}]")
            if not any(m["is_virtual_candidate"] for m in mons):
                print("  提示: 目前虛擬螢幕處於拔除 (Off) 狀態。使用 `cockpit_cli.py display on` 即可瞬間掛載。")

        elif args.action == "on":
            print("[DISPLAY] 正在透過 Rust 引擎掛載虛擬顯示器 (0 批次檔、0 UAC 彈窗)...")
            try:
                vm = ensure_virtual_display_on()
                print("[SUCCESS] 虛擬顯示器已就緒！")
                print(f"  名稱: {vm['name']}")
                print(f"  解析度: {vm['width']}x{vm['height']}")
                print(f"  座標: ({vm['x']}, {vm['y']})")
            except Exception as e:
                print(f"[ERROR] 掛載失敗: {e}")
                sys.exit(1)

        elif args.action == "off":
            print("[DISPLAY] 正在透過 Rust 引擎拔除虛擬顯示器...")
            try:
                mons = plug_out_virtual_display()
                print("[SUCCESS] 虛擬顯示器已成功拔除，系統已恢復為單螢幕狀態。")
                for m in mons:
                    print(f"  - {m['name']}: {m['width']}x{m['height']} @ ({m['x']}, {m['y']})")
            except Exception as e:
                print(f"[ERROR] 拔除失敗: {e}")
                sys.exit(1)

    elif args.command == "launch":
        parsed_args = (
            shlex.split(args.args)
            if isinstance(args.args, str) and args.args.strip()
            else (args.args if isinstance(args.args, list) else [])
        )
        cmd = {
            "action": "launch",
            "path": args.path,
            "args": parsed_args,
            "target_screen": args.screen,
            "wait_for_window": not args.no_wait,
            "timeout_ms": args.timeout,
        }
        if args.working_dir:
            cmd["working_dir"] = args.working_dir
        timeout_val = max(15.0, (args.timeout / 1000.0) + 5.0)
        res = asyncio.run(send_cmd(cmd, wait_for_event="app_launched", timeout=timeout_val))
        if res and res.get("status") == "ok":
            data = res.get("data", {})
            h_val = data.get("hwnd")
            h_str = f"0x{h_val:X}" if h_val else "無 (尚未偵測到頂層視窗)"
            print("[SUCCESS] 應用程式已成功啟動！")
            print(f"  PID:        {data.get('pid')}")
            print(f"  目標螢幕:   {data.get('target_screen')}")
            print(f"  頂層視窗:   {h_str}")
            if data.get("title"):
                print(f"  視窗標題:   {data.get('title')}")
        else:
            msg = res.get("message") if res else "未知錯誤"
            print(f"[ERROR] 啟動失敗: {msg}")
            sys.exit(1)

    elif args.command == "close":
        cmd = {
            "action": "close",
            "force": args.force,
            "timeout_ms": args.timeout,
        }
        if args.target:
            cmd["hwnd"] = parse_hwnd(args.target)
        timeout_val = max(8.0, (args.timeout / 1000.0) + 4.0)
        res = asyncio.run(send_cmd(cmd, wait_for_event="window_closed", timeout=timeout_val))
        if res and res.get("status") == "ok":
            data = res.get("data", {})
            h_val = data.get("hwnd")
            h_str = f"0x{h_val:X}" if h_val else "未鎖定"
            forced_str = " (強制終止)" if data.get("forced") else " (優雅退出)"
            print(f"[SUCCESS] 視窗/進程已關閉{forced_str}: HWND={h_str}, PID={data.get('pid')}")
        else:
            msg = res.get("message") if res else "未知錯誤"
            print(f"[ERROR] 關閉失敗: {msg}")
            sys.exit(1)
