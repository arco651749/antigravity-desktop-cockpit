import sys
import json
import asyncio
import argparse
from pathlib import Path

# 強制 UTF-8 編碼避免 Windows 終端亂碼
sys.stdout.reconfigure(encoding="utf-8")

WS_URL = "ws://127.0.0.1:48280"

import time
import ctypes
from ctypes import wintypes

class GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", wintypes.DWORD),
        ("Data2", wintypes.WORD),
        ("Data3", wintypes.WORD),
        ("Data4", ctypes.c_byte * 8)
    ]

GUID_DEVINTERFACE_USBMMIDD = GUID(
    0xb5ffd75f,
    0xda40,
    0x4353,
    (ctypes.c_byte * 8)(0x8f, 0xf8, 0xb6, 0xda, 0xf6, 0xf1, 0xd8, 0xca)
)

DIGCF_PRESENT = 0x00000002
DIGCF_DEVICEINTERFACE = 0x00000010
IOCTL_PLUG_MONITOR = 2307084  # 0x23340C

class SP_DEVICE_INTERFACE_DATA(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("InterfaceClassGuid", GUID),
        ("Flags", wintypes.DWORD),
        ("Reserved", ctypes.POINTER(ctypes.c_ulong))
    ]

class RECT(ctypes.Structure):
    _fields_ = [
        ('left', ctypes.c_long),
        ('top', ctypes.c_long),
        ('right', ctypes.c_long),
        ('bottom', ctypes.c_long)
    ]

class MONITORINFOEXW(ctypes.Structure):
    _fields_ = [
        ('cbSize', wintypes.DWORD),
        ('rcMonitor', RECT),
        ('rcWork', RECT),
        ('dwFlags', wintypes.DWORD),
        ('szDevice', wintypes.WCHAR * 32)
    ]

def list_system_monitors():
    user32 = ctypes.windll.user32
    monitors = []
    def cb(hmon, hdc, lprc, lparam):
        mi = MONITORINFOEXW()
        mi.cbSize = ctypes.sizeof(MONITORINFOEXW)
        user32.GetMonitorInfoW(hmon, ctypes.byref(mi))
        is_primary = bool(mi.dwFlags & 1)
        w = mi.rcMonitor.right - mi.rcMonitor.left
        h = mi.rcMonitor.bottom - mi.rcMonitor.top
        monitors.append({
            "name": mi.szDevice,
            "primary": is_primary,
            "x": mi.rcMonitor.left,
            "y": mi.rcMonitor.top,
            "width": w,
            "height": h,
            "is_virtual_candidate": not is_primary
        })
        return 1
    CMP = ctypes.WINFUNCTYPE(ctypes.c_int, wintypes.HMONITOR, wintypes.HDC, ctypes.POINTER(RECT), wintypes.LPARAM)
    user32.EnumDisplayMonitors(0, 0, CMP(cb), 0)
    return monitors

def get_virtual_device_path():
    setupapi = ctypes.windll.setupapi
    setupapi.SetupDiGetClassDevsW.restype = wintypes.HANDLE
    setupapi.SetupDiGetClassDevsW.argtypes = [
        ctypes.POINTER(GUID), wintypes.LPCWSTR, wintypes.HWND, wintypes.DWORD
    ]
    setupapi.SetupDiEnumDeviceInterfaces.restype = wintypes.BOOL
    setupapi.SetupDiEnumDeviceInterfaces.argtypes = [
        wintypes.HANDLE, ctypes.c_void_p, ctypes.POINTER(GUID), wintypes.DWORD,
        ctypes.POINTER(SP_DEVICE_INTERFACE_DATA)
    ]
    setupapi.SetupDiGetDeviceInterfaceDetailW.restype = wintypes.BOOL
    setupapi.SetupDiGetDeviceInterfaceDetailW.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(SP_DEVICE_INTERFACE_DATA), ctypes.c_void_p,
        wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p
    ]
    setupapi.SetupDiDestroyDeviceInfoList.restype = wintypes.BOOL
    setupapi.SetupDiDestroyDeviceInfoList.argtypes = [wintypes.HANDLE]

    h_dev_info = setupapi.SetupDiGetClassDevsW(
        ctypes.byref(GUID_DEVINTERFACE_USBMMIDD),
        None,
        None,
        DIGCF_PRESENT | DIGCF_DEVICEINTERFACE
    )
    if h_dev_info == -1 or not h_dev_info:
        return None

    dev_data = SP_DEVICE_INTERFACE_DATA()
    dev_data.cbSize = ctypes.sizeof(SP_DEVICE_INTERFACE_DATA)

    if not setupapi.SetupDiEnumDeviceInterfaces(
        h_dev_info, None, ctypes.byref(GUID_DEVINTERFACE_USBMMIDD), 0, ctypes.byref(dev_data)
    ):
        setupapi.SetupDiDestroyDeviceInfoList(h_dev_info)
        return None

    req_size = wintypes.DWORD(0)
    setupapi.SetupDiGetDeviceInterfaceDetailW(h_dev_info, ctypes.byref(dev_data), None, 0, ctypes.byref(req_size), None)
    buf = ctypes.create_string_buffer(req_size.value)
    ctypes.memmove(buf, ctypes.byref(wintypes.DWORD(8)), 4)

    if not setupapi.SetupDiGetDeviceInterfaceDetailW(h_dev_info, ctypes.byref(dev_data), buf, req_size.value, None, None):
        setupapi.SetupDiDestroyDeviceInfoList(h_dev_info)
        return None

    path = ctypes.wstring_at(ctypes.addressof(buf) + 4)
    setupapi.SetupDiDestroyDeviceInfoList(h_dev_info)
    return path

def send_virtual_display_ioctl(cmd_byte: int) -> bool:
    kernel32 = ctypes.windll.kernel32
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
        wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE
    ]
    kernel32.DeviceIoControl.restype = wintypes.BOOL
    kernel32.DeviceIoControl.argtypes = [
        wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
        ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p
    ]
    kernel32.CloseHandle.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]

    path = get_virtual_device_path()
    if not path:
        raise RuntimeError("未檢測到虛擬顯示器驅動裝置介面 (GUID: {b5ffd75f-da40-4353-8ff8-b6daf6f1d8ca})")

    GENERIC_READ = 0x80000000
    GENERIC_WRITE = 0x40000000
    FILE_SHARE_READ = 0x00000001
    FILE_SHARE_WRITE = 0x00000002
    OPEN_EXISTING = 3

    h_dev = kernel32.CreateFileW(
        path,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        0,
        None
    )
    if h_dev == -1 or not h_dev:
        raise RuntimeError(f"無法打開虛擬顯示器裝置 Handle, LastError={kernel32.GetLastError()}")

    try:
        in_buf = (ctypes.c_byte * 4)(cmd_byte, 0, 0, 0)
        bytes_ret = wintypes.DWORD(0)
        ok = kernel32.DeviceIoControl(
            h_dev,
            IOCTL_PLUG_MONITOR,
            ctypes.byref(in_buf),
            4,
            None,
            0,
            ctypes.byref(bytes_ret),
            None
        )
        if not ok:
            raise RuntimeError(f"DeviceIoControl 發送失敗, LastError={kernel32.GetLastError()}")
        return True
    finally:
        kernel32.CloseHandle(h_dev)

def ensure_virtual_display_on():
    mons = list_system_monitors()
    virtual_mon = next((m for m in mons if m["is_virtual_candidate"]), None)
    if virtual_mon:
        return virtual_mon

    send_virtual_display_ioctl(0x10)
    for _ in range(20):
        time.sleep(0.15)
        mons = list_system_monitors()
        virtual_mon = next((m for m in mons if m["is_virtual_candidate"]), None)
        if virtual_mon:
            return virtual_mon
    raise TimeoutError("已發送 DeviceIoControl 插入指令，但等待 Windows DWM 註冊虛擬螢幕超時 (3秒)")

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
    p_mouse.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    # open-url
    p_url = subparsers.add_parser("open-url", help="在瀏覽器視窗中導航至指定網址 (自動 Ctrl+L -> 輸入網址 -> Enter)")
    p_url.add_argument("url", help="目標網址 (如: https://www.104.com.tw)")
    p_url.add_argument("--hwnd", help="可選特定瀏覽器 HWND")

    # uia
    p_uia = subparsers.add_parser("uia", help="取得目標視窗之 UIA 語意樹")
    p_uia.add_argument("-d", "--depth", type=int, default=8, help="遍歷深度 (預設 8)")
    p_uia.add_argument("--hwnd", help="可選視窗 HWND")
    p_uia.add_argument("-f", "--find", help="搜尋特定名稱、控制項類型或 ID 之節點")
    p_uia.add_argument("-s", "--summary", action="store_true", help="輸出精簡概覽與統計資訊")

    # diff
    p_diff = subparsers.add_parser("diff", help="比對 Visual Memory 快照差異")

    # display
    p_display = subparsers.add_parser("display", help="控制或查詢虛擬顯示器硬體狀態 (0 批次檔、0 UAC 彈窗)")
    p_display.add_argument("action", choices=["on", "off", "status"], help="on=開啟, off=關閉, status=查詢狀態")

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
        u_val = args.u
        v_val = args.v
        if u_val > 1.0 or v_val > 1.0:
            mons = list_system_monitors()
            vm = next((m for m in mons if not m["primary"]), None)
            if vm and vm["width"] > 0 and vm["height"] > 0:
                orig_u, orig_v = u_val, v_val
                u_val = round((u_val - vm["x"]) / vm["width"], 4)
                v_val = round((v_val - vm["y"]) / vm["height"], 4)
                u_val = max(0.0, min(1.0, u_val))
                v_val = max(0.0, min(1.0, v_val))
                print(f"[MOUSE] 絕對座標 ({orig_u}, {orig_v}) 已自動轉換為虛擬螢幕歸一化座標 ({u_val}, {v_val})")

        async def exec_mouse():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    h = parse_hwnd(args.hwnd)
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.1)

                if sub == "click":
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "down",
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.05)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "up",
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0
                    }))
                else:
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": sub,
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0
                    }))
                print(f"[MOUSE] 已執行滑鼠 {sub} 於 ({u_val}, {v_val}) [{args.button}]")
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
        tree = res.get("data", {}) if isinstance(res, dict) else {}

        mons = list_system_monitors()
        vm = next((m for m in mons if not m["primary"]), None)

        def calc_uv(cx, cy):
            if vm and vm["width"] > 0 and vm["height"] > 0:
                u = round((cx - vm["x"]) / vm["width"], 4)
                v = round((cy - vm["y"]) / vm["height"], 4)
                return u, v
            return None, None

        if args.find or args.summary:
            all_nodes = []
            matches = []

            def walk(node, depth=0):
                name = node.get("name", "").strip()
                ctype = node.get("control_type", "")
                rect = node.get("bounding_rect", [0, 0, 0, 0])
                center = node.get("center_point", [0, 0])
                cid = node.get("id", "")

                has_size = rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0
                if name or has_size:
                    u, v = calc_uv(center[0], center[1]) if center and len(center) == 2 else (None, None)
                    item = {
                        "depth": depth, "type": ctype, "name": name,
                        "rect": rect, "center": center, "id": cid,
                        "u": u, "v": v
                    }
                    all_nodes.append(item)

                    if args.find:
                        kw = args.find.lower()
                        if kw in name.lower() or kw in ctype.lower() or kw in cid.lower():
                            matches.append(item)

                for child in node.get("children", []):
                    walk(child, depth + 1)

            walk(tree)

            if args.find:
                print(f"[UIA-FIND] 關鍵字 '{args.find}' 搜尋結果 (共 {len(matches)} 筆匹配):")
                print("-" * 115)
                print(f"{'深度':<4} | {'類型':<12} | {'中心點 (X, Y)':<15} | {'歸一化 (u, v)':<17} | {'名稱 / 標籤'}")
                print("-" * 115)
                for item in matches:
                    cent_str = f"({item['center'][0]},{item['center'][1]})" if item['center'] else "-"
                    uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                    name_display = item['name'][:55] if item['name'] else f"[ID: {item['id']}]"
                    print(f"{item['depth']:<4} | {item['type']:<12} | {cent_str:<15} | {uv_str:<17} | {name_display}")
                print("-" * 115)
                if matches and matches[0]['u'] is not None:
                    m0 = matches[0]
                    print(f"💡 快速點擊提示: python.exe cockpit_cli.py mouse click {m0['u']} {m0['v']}")
            elif args.summary:
                from collections import Counter
                ctls = Counter(n["type"] for n in all_nodes)
                print(f"[UIA-SUMMARY] 節點總數: {len(all_nodes)}")
                print("控制項類型分布:")
                for ctl, count in ctls.most_common(12):
                    print(f"  - {ctl:<16}: {count}")
                named = [n for n in all_nodes if n["name"]]
                print(f"具備文字語意名稱節點數: {len(named)}")
        else:
            print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "diff":
        res = asyncio.run(send_cmd({"action": "get_visual_diff"}, wait_for_event="visual_diff"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "display":
        if args.action == "status":
            mons = list_system_monitors()
            print("[DISPLAY] 系統顯示器拓撲清單:")
            for m in mons:
                tag = "主要螢幕 🏠" if m["primary"] else "虛擬第二螢幕 🟢"
                print(f"  - {m['name']}: {m['width']}x{m['height']} @ ({m['x']}, {m['y']}) [{tag}]")
            has_virtual = any(m["is_virtual_candidate"] for m in mons)
            if not has_virtual:
                print("  提示: 目前虛擬螢幕處於拔除 (Off) 狀態。使用 `cockpit_cli.py display on` 即可瞬間掛載。")

        elif args.action == "on":
            print("[DISPLAY] 正在發送 Win32 DeviceIoControl 指令掛載虛擬顯示器 (0 批次檔、0 UAC)...")
            try:
                vm = ensure_virtual_display_on()
                print(f"[SUCCESS] 虛擬顯示器已就緒！")
                print(f"  名稱: {vm['name']}")
                print(f"  解析度: {vm['width']}x{vm['height']}")
                print(f"  座標: ({vm['x']}, {vm['y']})")
            except Exception as e:
                print(f"[ERROR] 掛載失敗: {e}")
                sys.exit(1)

        elif args.action == "off":
            print("[DISPLAY] 正在發送 Win32 DeviceIoControl 指令拔除虛擬顯示器...")
            try:
                send_virtual_display_ioctl(0x00)
                time.sleep(0.5)
                mons = list_system_monitors()
                print(f"[SUCCESS] 虛擬顯示器已成功拔除，系統已恢復為單螢幕狀態。")
                for m in mons:
                    print(f"  - {m['name']}: {m['width']}x{m['height']} @ ({m['x']}, {m['y']})")
            except Exception as e:
                print(f"[ERROR] 拔除失敗: {e}")
                sys.exit(1)

if __name__ == "__main__":
    main()
