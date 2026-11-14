import asyncio
import json
import sys

from ws_client import (
    VK_MAP,
    WS_URL,
    get_clipboard_text,
    list_system_monitors,
    parse_hwnd,
    send_cmd,
    set_clipboard_text,
)

HANDLED_COMMANDS = {"hotkey", "type", "mouse", "drag", "wheel", "open-url"}


def handle(args):
    if args.command == "hotkey":
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

            send_char = None if (ctrl or alt) else char
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}))
                    await asyncio.sleep(0.08)

                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                    "vk": vk, "char": send_char, "ctrl": ctrl, "alt": alt, "shift": shift,
                }))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                    "vk": vk, "char": send_char, "ctrl": ctrl, "alt": alt, "shift": shift,
                }))
                await asyncio.sleep(0.06)

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
        u_val = args.u
        v_val = args.v
        if u_val > 1.0 or v_val > 1.0:
            mons = list_system_monitors()
            vm = next((m for m in mons if not m["primary"]), None)
            if vm and vm["width"] > 0 and vm["height"] > 0:
                orig_u, orig_v = u_val, v_val
                u_val = max(0.0, min(1.0, round((u_val - vm["x"]) / vm["width"], 4)))
                v_val = max(0.0, min(1.0, round((v_val - vm["y"]) / vm["height"], 4)))
                print(f"[MOUSE] 絕對座標 ({orig_u}, {orig_v}) 已自動轉換為虛擬螢幕歸一化座標 ({u_val}, {v_val})")

        async def exec_mouse():
            import websockets

            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}))
                    await asyncio.sleep(0.1)

                delta_val = getattr(args, "delta", 120)
                if sub == "click":
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": "none", "u": u_val, "v": v_val, "delta": 0,
                    }))
                    await asyncio.sleep(0.16)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "click",
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0,
                    }))
                    await asyncio.sleep(0.08)
                elif sub == "wheel":
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "wheel",
                        "button": "left", "u": u_val, "v": v_val, "delta": delta_val,
                    }))
                    await asyncio.sleep(0.08)
                else:
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": sub,
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0,
                    }))
                    await asyncio.sleep(0.06)
                print(f"[MOUSE] 已執行 AI 虛擬游標 {sub} 於 ({u_val}, {v_val}) [{args.button}]")

        asyncio.run(exec_mouse())

    elif args.command == "drag":
        start_u, start_v = args.start_u, args.start_v
        end_u, end_v = args.end_u, args.end_v
        btn = args.button
        steps = max(1, args.steps)
        dur = max(0.05, args.duration)

        async def exec_drag():
            import websockets

            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}))
                    await asyncio.sleep(0.08)

                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                    "button": "none", "u": start_u, "v": start_v, "delta": 0,
                }))
                await asyncio.sleep(0.06)
                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "down",
                    "button": btn, "u": start_u, "v": start_v, "delta": 0,
                }))
                await asyncio.sleep(0.04)

                step_dt = dur / steps
                for i in range(1, steps + 1):
                    frac = i / steps
                    cur_u = round(start_u + (end_u - start_u) * frac, 4)
                    cur_v = round(start_v + (end_v - start_v) * frac, 4)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": btn, "u": cur_u, "v": cur_v, "delta": 0,
                    }))
                    await asyncio.sleep(step_dt)

                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "up",
                    "button": btn, "u": end_u, "v": end_v, "delta": 0,
                }))
                await asyncio.sleep(0.06)
                print(f"[DRAG] 已完成拖曳操作: ({start_u}, {start_v}) -> ({end_u}, {end_v}) [按鍵: {btn}, 步數: {steps}]")

        asyncio.run(exec_drag())

    elif args.command == "wheel":
        u_val, v_val, delta_val = args.u, args.v, args.delta

        async def exec_wheel():
            import websockets

            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}))
                    await asyncio.sleep(0.08)

                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "wheel",
                    "button": "left", "u": u_val, "v": v_val, "delta": delta_val,
                }))
                await asyncio.sleep(0.08)
                print(f"[WHEEL] 已執行滾輪滾動: 座標 ({u_val}, {v_val}), delta={delta_val} ({'向上滾動' if delta_val > 0 else '向下滾動'})")

        asyncio.run(exec_wheel())

    elif args.command == "open-url":
        saved_clipboard = get_clipboard_text()

        async def exec_open_url():
            import websockets

            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": parse_hwnd(args.hwnd)}))
                    await asyncio.sleep(0.3)

                print("[OPEN-URL] 正在聚焦網址列 (Ctrl+L)...")
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 17}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 76}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 76}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 17}))
                await asyncio.sleep(0.25)

                print(f"[OPEN-URL] 正在貼入目標網址 ({len(args.url)} 字元): {args.url}...")
                await ws.send(json.dumps({"action": "set_clipboard", "text": args.url}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 17}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 86}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 86}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 17}))
                await asyncio.sleep(0.20)

                if saved_clipboard is not None:
                    await ws.send(json.dumps({"action": "set_clipboard", "text": saved_clipboard}))
                    await asyncio.sleep(0.03)

                print("[OPEN-URL] 正在送出 Enter 進行導航...")
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 13}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 13}))
                await asyncio.sleep(0.15)
                await ws.send(json.dumps({"action": "restore_primary_focus"}))
                await asyncio.sleep(0.25)
                print("[OPEN-URL] 導航指令已發送，剪貼簿與主畫面焦點已自動歸還！")

        asyncio.run(exec_open_url())
