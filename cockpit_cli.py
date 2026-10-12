import sys
import json
import asyncio
import argparse
from pathlib import Path

# 強制 UTF-8 編碼避免 Windows 終端亂碼
sys.stdout.reconfigure(encoding="utf-8")

WS_URL = "ws://127.0.0.1:48280"

VK_MAP = {
    "enter": 13,
    "return": 13,
    "backspace": 8,
    "tab": 9,
    "esc": 27,
    "escape": 27,
    "space": 32,
    "left": 37,
    "up": 38,
    "right": 39,
    "down": 40,
    "delete": 46,
    "home": 36,
    "end": 35,
    "pageup": 33,
    "pagedown": 34,
}

def parse_hwnd(val):
    if val is None:
        return None
    if isinstance(val, int):
        return val
    s = str(val).strip()
    if s.lower().startswith("0x"):
        return int(s, 16)
    return int(s)

async def send_cmd(cmd_dict, wait_for_event=None, capture_frame_path=None, timeout=6.0):
    import websockets
    try:
        async with websockets.connect(WS_URL, max_size=15 * 1024 * 1024) as ws:
            # 確保處於 direct 直通模式
            await ws.send(json.dumps({"action": "set_capture_mode", "mode": "direct"}))

            if cmd_dict:
                await ws.send(json.dumps(cmd_dict))

            start_t = asyncio.get_event_loop().time()
            result_event = None
            captured_bytes = None

            while True:
                elapsed = asyncio.get_event_loop().time() - start_t
                if elapsed > timeout:
                    break

                try:
                    msg = await asyncio.wait_for(ws.recv(), timeout=timeout - elapsed)
                except asyncio.TimeoutError:
                    break

                if isinstance(msg, bytes):
                    if capture_frame_path and not captured_bytes:
                        captured_bytes = msg
                        if not wait_for_event:
                            break
                elif isinstance(msg, str):
                    try:
                        data = json.loads(msg)
                        event = data.get("event")
                        if wait_for_event:
                            if event == wait_for_event:
                                result_event = data
                                if not capture_frame_path or captured_bytes:
                                    break
                        else:
                            if event not in ("capture_mode_set",):
                                result_event = data
                                if not capture_frame_path or captured_bytes:
                                    break
                    except Exception:
                        pass

            if capture_frame_path and captured_bytes:
                out_path = Path(capture_frame_path).resolve()
                out_path.parent.mkdir(parents=True, exist_ok=True)
                out_path.write_bytes(captured_bytes)
                print(f"[SUCCESS] 畫面已保存至: {out_path} ({len(captured_bytes)} bytes)")

            return result_event

    except ConnectionRefusedError:
        print("[ERROR] 無法連線至 Desktop Cockpit 引擎 (ws://127.0.0.1:48280)，請確認服務已啟動。")
        sys.exit(1)
    except Exception as e:
        print(f"[ERROR] 通訊異常: {e}")
        sys.exit(1)

def main():
    parser = argparse.ArgumentParser(description="Desktop Cockpit CLI (ADC 2.0)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # list
    p_list = subparsers.add_parser("list", help="列出系統活動視窗清單")
    p_list.add_argument("-f", "--filter", help="過濾視窗標題關鍵字")

    # dispatch
    p_dispatch = subparsers.add_parser("dispatch", help="派駐視窗至虛擬第二螢幕")
    p_dispatch.add_argument("hwnd", help="視窗 HWND (可為 0xHEX 或十進位)")

    # restore
    p_restore = subparsers.add_parser("restore", help="將視窗移回主實體螢幕")
    p_restore.add_argument("hwnd", help="視窗 HWND")

    # set
    p_set = subparsers.add_parser("set", help="鎖定目標串流視窗")
    p_set.add_argument("hwnd", help="視窗 HWND")

    # capture
    p_cap = subparsers.add_parser("capture", help="截取目標視窗或虛擬螢幕當前 DXGI 畫面")
    p_cap.add_argument("-o", "--output", default="screen.jpg", help="輸出 JPEG 檔案路徑")
    p_cap.add_argument("--hwnd", help="可選特定視窗 HWND")

    # hotkey
    p_hotkey = subparsers.add_parser("hotkey", help="直通發送快捷鍵 (如 ctrl+l, enter, esc)")
    p_hotkey.add_argument("keys", help="按鍵組合 (如: 'ctrl+l', 'enter', 'alt+f4')")

    # type
    p_type = subparsers.add_parser("type", help="向目標視窗注入文字")
    p_type.add_argument("text", help="欲輸入的文字字串")
    p_type.add_argument("--hwnd", help="可選特定視窗 HWND")

    # mouse
    p_mouse = subparsers.add_parser("mouse", help="投遞滑鼠操作")
    p_mouse.add_argument("subaction", choices=["click", "down", "up", "move"], help="滑鼠動作")
    p_mouse.add_argument("u", type=float, help="歸一化 X 座標 (0.0~1.0)")
    p_mouse.add_argument("v", type=float, help="歸一化 Y 座標 (0.0~1.0)")
    p_mouse.add_argument("-b", "--button", default="left", choices=["left", "middle", "right"], help="按鍵")

    # open-url
    p_url = subparsers.add_parser("open-url", help="在瀏覽器視窗中導航至指定網址 (自動 Ctrl+L -> 輸入網址 -> Enter)")
    p_url.add_argument("url", help="目標網址 (如: https://www.104.com.tw)")
    p_url.add_argument("--hwnd", help="可選特定瀏覽器 HWND")

    # uia
    p_uia = subparsers.add_parser("uia", help="取得目標視窗之 UIA 語意樹")
    p_uia.add_argument("-d", "--depth", type=int, default=4, help="遍歷深度")
    p_uia.add_argument("--hwnd", help="可選視窗 HWND")

    # diff
    p_diff = subparsers.add_parser("diff", help="比對 Visual Memory 快照差異")

    args = parser.parse_args()

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
        cmd = None
        if args.hwnd:
            cmd = {"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}
        asyncio.run(send_cmd(cmd, capture_frame_path=args.output, timeout=3.0))

    elif args.command == "hotkey":
        parts = [p.strip().lower() for p in args.keys.split("+")]
        ctrl = "ctrl" in parts or "control" in parts
        alt = "alt" in parts
        shift = "shift" in parts
        main_key = parts[-1]

        vk = VK_MAP.get(main_key)
        char = None
        if not vk:
            if len(main_key) == 1:
                vk = ord(main_key.upper())
                char = main_key.upper() if shift else main_key.lower()
            else:
                print(f"[ERROR] 未知按鍵: {main_key}")
                sys.exit(1)

        async def exec_hotkey():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                # down
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                    "vk": vk, "char": char, "ctrl": ctrl, "alt": alt, "shift": shift
                }))
                await asyncio.sleep(0.08)
                # up
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                    "vk": vk, "char": char, "ctrl": ctrl, "alt": alt, "shift": shift
                }))
                res = await ws.recv()
                while isinstance(res, bytes):
                    res = await ws.recv()
                print(f"[HOTKEY] 已發送: {args.keys} -> {res}")

        asyncio.run(exec_hotkey())

    elif args.command == "type":
        hwnd = parse_hwnd(args.hwnd)
        cmd = {"action": "type_text", "text": args.text}
        if hwnd:
            cmd["hwnd"] = hwnd
        res = asyncio.run(send_cmd(cmd, wait_for_event="text_typed"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "mouse":
        sub = args.subaction
        async def exec_mouse():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                if sub == "click":
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "down",
                        "button": args.button, "u": args.u, "v": args.v, "delta": 0
                    }))
                    await asyncio.sleep(0.05)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "up",
                        "button": args.button, "u": args.u, "v": args.v, "delta": 0
                    }))
                else:
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": sub,
                        "button": args.button, "u": args.u, "v": args.v, "delta": 0
                    }))
                print(f"[MOUSE] 已執行滑鼠 {sub} 於 ({args.u}, {args.v}) [{args.button}]")
        asyncio.run(exec_mouse())

    elif args.command == "open-url":
        async def exec_open_url():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    h = parse_hwnd(args.hwnd)
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.4)

                # 1. 聚焦網址列 (Ctrl+L)
                print("[OPEN-URL] 正在聚焦網址列 (Ctrl+L)...")
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                    "vk": 76, "char": None, "ctrl": True, "alt": False, "shift": False
                }))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                    "vk": 76, "char": None, "ctrl": True, "alt": False, "shift": False
                }))
                await asyncio.sleep(0.3)

                # 2. 注入網址
                print(f"[OPEN-URL] 正在注入目標網址: {args.url}...")
                await ws.send(json.dumps({"action": "type_text", "text": args.url}))
                await asyncio.sleep(0.4)

                # 3. 按下 Enter
                print("[OPEN-URL] 正在送出 Enter 進行導航...")
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                    "vk": 13, "char": None, "ctrl": False, "alt": False, "shift": False
                }))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                    "vk": 13, "char": None, "ctrl": False, "alt": False, "shift": False
                }))
                print("[OPEN-URL] 導航指令已全數成功發送！")

        asyncio.run(exec_open_url())

    elif args.command == "uia":
        cmd = {"action": "get_uia_tree", "depth": args.depth}
        if args.hwnd:
            cmd["hwnd"] = parse_hwnd(args.hwnd)
        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_tree"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "diff":
        res = asyncio.run(send_cmd({"action": "get_visual_diff"}, wait_for_event="visual_diff"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
