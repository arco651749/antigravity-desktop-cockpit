import asyncio
import json
import sys
import time
from pathlib import Path

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


def list_system_monitors():
    """透過 Rust 引擎統一查詢系統顯示器拓撲，不再於 Python 端重複實作 Win32 API。"""
    res = asyncio.run(send_cmd({"action": "get_monitors"}, wait_for_event="monitors"))
    mons = res.get("data", []) if isinstance(res, dict) else []
    for m in mons:
        if "primary" not in m and "is_primary" in m:
            m["primary"] = m["is_primary"]
    return mons


def ensure_virtual_display_on():
    """透過 Rust 引擎統一掛載虛擬顯示器，消除雙端狀態不同步。"""
    mons = list_system_monitors()
    virtual_mon = next((m for m in mons if m.get("is_virtual_candidate")), None)
    if virtual_mon:
        return virtual_mon

    res = asyncio.run(
        send_cmd({"action": "plug_in_virtual_display"}, wait_for_event="virtual_display_plugged")
    )
    if not res or res.get("status") != "ok":
        msg = res.get("message") if isinstance(res, dict) else "引擎無回應"
        raise RuntimeError(f"掛載虛擬顯示器失敗: {msg}")

    for _ in range(15):
        mons = list_system_monitors()
        virtual_mon = next((m for m in mons if m.get("is_virtual_candidate")), None)
        if virtual_mon:
            return virtual_mon
        time.sleep(0.15)
    raise TimeoutError("已透過 Rust 引擎發送掛載指令，但等待 Windows DWM 註冊虛擬螢幕超時")


def plug_out_virtual_display():
    res = asyncio.run(
        send_cmd({"action": "plug_out_virtual_display"}, wait_for_event="virtual_display_unplugged")
    )
    if not res or res.get("status") != "ok":
        msg = res.get("message") if isinstance(res, dict) else "引擎無回應"
        raise RuntimeError(f"卸載虛擬顯示器失敗: {msg}")
    return res.get("data", [])


def get_clipboard_text():
    res = asyncio.run(send_cmd({"action": "get_clipboard"}, wait_for_event="clipboard"))
    if isinstance(res, dict) and res.get("status") == "ok":
        data = res.get("data") or {}
        return data.get("text")
    return None


def set_clipboard_text(text: str) -> bool:
    res = asyncio.run(
        send_cmd({"action": "set_clipboard", "text": text}, wait_for_event="clipboard_set")
    )
    return bool(isinstance(res, dict) and res.get("status") == "ok")


def normalize_center_to_uv(center, root_rect=None):
    mons = list_system_monitors()
    vm = next((m for m in mons if not m.get("primary")), None)
    pm = next((m for m in mons if m.get("primary")), None)
    if not center or len(center) != 2:
        return 0.5, 0.5
    cx, cy = center[0], center[1]
    if vm and vm["width"] > 0 and vm["height"] > 0 and cx >= vm["x"] - 50:
        return (
            max(0.0, min(1.0, round((cx - vm["x"]) / vm["width"], 4))),
            max(0.0, min(1.0, round((cy - vm["y"]) / vm["height"], 4))),
        )
    if root_rect and len(root_rect) == 4 and root_rect[2] > 0 and root_rect[3] > 0:
        if root_rect[0] <= cx <= root_rect[0] + root_rect[2] and root_rect[1] <= cy <= root_rect[1] + root_rect[3]:
            return (
                max(0.0, min(1.0, round((cx - root_rect[0]) / root_rect[2], 4))),
                max(0.0, min(1.0, round((cy - root_rect[1]) / root_rect[3], 4))),
            )
    if pm and pm["width"] > 0 and pm["height"] > 0:
        return (
            max(0.0, min(1.0, round((cx - pm["x"]) / pm["width"], 4))),
            max(0.0, min(1.0, round((cy - pm["y"]) / pm["height"], 4))),
        )
    return 0.5, 0.5
