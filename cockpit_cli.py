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

def get_clipboard_text():
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    user32.OpenClipboard.argtypes = [wintypes.HWND]
    user32.OpenClipboard.restype = wintypes.BOOL
    user32.CloseClipboard.restype = wintypes.BOOL
    user32.GetClipboardData.argtypes = [wintypes.UINT]
    user32.GetClipboardData.restype = wintypes.HANDLE
    kernel32.GlobalLock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalUnlock.restype = wintypes.BOOL

    CF_UNICODETEXT = 13
    for _ in range(5):
        if user32.OpenClipboard(None):
            break
        time.sleep(0.03)
    else:
        return None
    try:
        h_mem = user32.GetClipboardData(CF_UNICODETEXT)
        if not h_mem:
            return None
        ptr = kernel32.GlobalLock(h_mem)
        if not ptr:
            return None
        try:
            return ctypes.wstring_at(ptr)
        finally:
            kernel32.GlobalUnlock(h_mem)
    finally:
        user32.CloseClipboard()

def set_clipboard_text(text: str) -> bool:
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    user32.OpenClipboard.argtypes = [wintypes.HWND]
    user32.OpenClipboard.restype = wintypes.BOOL
    user32.CloseClipboard.restype = wintypes.BOOL
    user32.EmptyClipboard.restype = wintypes.BOOL
    user32.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
    user32.SetClipboardData.restype = wintypes.HANDLE
    kernel32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
    kernel32.GlobalAlloc.restype = wintypes.HGLOBAL
    kernel32.GlobalLock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalUnlock.restype = wintypes.BOOL

    CF_UNICODETEXT = 13
    GMEM_MOVEABLE = 0x0002
    for _ in range(5):
        if user32.OpenClipboard(None):
            break
        time.sleep(0.04)
    else:
        return False
    try:
        user32.EmptyClipboard()
        encoded = (text + '\0').encode('utf-16le')
        h_mem = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(encoded))
        if not h_mem:
            return False
        ptr = kernel32.GlobalLock(h_mem)
        if not ptr:
            return False
        ctypes.memmove(ptr, encoded, len(encoded))
        kernel32.GlobalUnlock(h_mem)
        if not user32.SetClipboardData(CF_UNICODETEXT, h_mem):
            return False
        return True
    finally:
        user32.CloseClipboard()

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
    p_hotkey.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    # type
    p_type = subparsers.add_parser("type", help="向目標視窗注入文字")
    p_type.add_argument("text", help="欲輸入的文字字串")
    p_type.add_argument("--hwnd", help="可選特定視窗 HWND")

    # mouse
    p_mouse = subparsers.add_parser("mouse", help="投遞滑鼠操作")
    p_mouse.add_argument("subaction", choices=["click", "down", "up", "move", "wheel"], help="滑鼠動作")
    p_mouse.add_argument("u", type=float, help="歸一化 X 座標 (0.0~1.0)")
    p_mouse.add_argument("v", type=float, help="歸一化 Y 座標 (0.0~1.0)")
    p_mouse.add_argument("-b", "--button", default="left", choices=["left", "middle", "right"], help="按鍵")
    p_mouse.add_argument("-d", "--delta", type=int, default=120, help="滾動量 (+120 向上, -120 向下，僅在 wheel 時生效)")
    p_mouse.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    # drag
    p_drag = subparsers.add_parser("drag", help="執行平滑連續拖曳操作 (down -> 多幀 move -> up)")
    p_drag.add_argument("start_u", type=float, help="起點歸一化 X 座標 (0.0~1.0)")
    p_drag.add_argument("start_v", type=float, help="起點歸一化 Y 座標 (0.0~1.0)")
    p_drag.add_argument("end_u", type=float, help="終點歸一化 X 座標 (0.0~1.0)")
    p_drag.add_argument("end_v", type=float, help="終點歸一化 Y 座標 (0.0~1.0)")
    p_drag.add_argument("-b", "--button", default="left", choices=["left", "middle", "right"], help="按鍵 (預設 left，3D 視角旋轉可選 middle)")
    p_drag.add_argument("--steps", type=int, default=8, help="中間插值步數 (預設 8)")
    p_drag.add_argument("--duration", type=float, default=0.25, help="拖曳耗時秒數 (預設 0.25s)")
    p_drag.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    # wheel
    p_wheel = subparsers.add_parser("wheel", help="投遞滑鼠滾輪滾動")
    p_wheel.add_argument("u", type=float, default=0.5, nargs="?", help="歸一化 X 座標 (預設 0.5)")
    p_wheel.add_argument("v", type=float, default=0.5, nargs="?", help="歸一化 Y 座標 (預設 0.5)")
    p_wheel.add_argument("-d", "--delta", type=int, default=120, help="滾動量 (+120 向上, -120 向下)")
    p_wheel.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    # open-url
    p_url = subparsers.add_parser("open-url", help="在瀏覽器視窗中導航至指定網址 (自動 Ctrl+L -> 輸入網址 -> Enter)")
    p_url.add_argument("url", help="目標網址 (如: https://www.104.com.tw)")
    p_url.add_argument("--hwnd", help="可選特定瀏覽器 HWND")

    # uia
    p_uia = subparsers.add_parser("uia", help="取得目標視窗之 UIA 語意樹 (預設輸出精簡 @Ref 清單)")
    p_uia.add_argument("-d", "--depth", type=int, default=8, help="邏輯遍歷深度 (預設 8，自動穿透透明容器)")
    p_uia.add_argument("--hwnd", help="可選視窗 HWND")
    p_uia.add_argument("-f", "--find", help="搜尋特定名稱、控制項類型、@Ref 或 ID 之節點")
    p_uia.add_argument("-s", "--summary", action="store_true", help="輸出精簡概覽與統計資訊")
    p_uia.add_argument("-j", "--json", action="store_true", help="輸出原始完整 JSON 結構")

    # act (語意動作鏈：Invoke -> Toggle -> Select -> Expand -> 自動降級 (u, v) 點擊)
    p_act = subparsers.add_parser("act", aliases=["invoke"], help="透過 @Ref 或名稱執行語意觸發 (優先 COM Pattern，失敗自動降級虛擬游標點擊)")
    p_act.add_argument("target", help="目標 @Ref (如 '@1') 或控制項名稱/AutomationId (如 '儲存')")
    p_act.add_argument("--hwnd", help="可選特定視窗 HWND")
    p_act.add_argument("--no-fallback", action="store_true", help="若無對應 COM Pattern 時不自動回退至 (u, v) 座標點擊")

    # fill (語意填值鏈：ValuePattern -> RangeValue -> Legacy -> 自動降級聚焦 + WM_CHAR 輸入)
    p_fill = subparsers.add_parser("fill", aliases=["set-value"], help="透過 @Ref 或名稱向輸入框/滑桿瞬間寫入文字或數值")
    p_fill.add_argument("target", help="目標 @Ref (如 '@3') 或控制項名稱/AutomationId")
    p_fill.add_argument("text", nargs="?", default="", help="欲寫入的文字字串或數值 (省略則清空)")
    p_fill.add_argument("--hwnd", help="可選特定視窗 HWND")
    p_fill.add_argument("--append", action="store_true", help="降級為 WM_CHAR 輸入時不先執行 Ctrl+A 清空")
    p_fill.add_argument("--no-fallback", action="store_true", help="若不支援 ValuePattern 時不自動降級為 WM_CHAR 輸入")

    # toggle
    p_toggle = subparsers.add_parser("toggle", help="透過 @Ref 或名稱切換核取方塊/開關狀態 (TogglePattern)")
    p_toggle.add_argument("target", help="目標 @Ref (如 '@2') 或控制項名稱")
    p_toggle.add_argument("--hwnd", help="可選特定視窗 HWND")

    # expand / collapse / focus
    p_expand = subparsers.add_parser("expand", help="展開指定選單、下拉框或樹狀節點 (ExpandCollapsePattern)")
    p_expand.add_argument("target", help="目標 @Ref 或名稱")
    p_expand.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_collapse = subparsers.add_parser("collapse", help="收合指定選單、下拉框或樹狀節點 (ExpandCollapsePattern)")
    p_collapse.add_argument("target", help="目標 @Ref 或名稱")
    p_collapse.add_argument("--hwnd", help="可選特定視窗 HWND")

    # diff
    p_diff = subparsers.add_parser("diff", help="比對 Visual Memory 快照差異 (含視窗、@Ref 節點新增/移除/狀態與數值變化)")
    p_diff.add_argument("--hwnd", help="可選特定視窗 HWND (預設使用目前鎖定視窗)")
    p_diff.add_argument("-j", "--json", action="store_true", help="輸出原始 JSON")

    # display
    p_display = subparsers.add_parser("display", help="控制或查詢虛擬顯示器硬體狀態 (0 批次檔、0 UAC 彈窗)")
    p_display.add_argument("action", choices=["on", "off", "status"], help="on=開啟, off=關閉, status=查詢狀態")

    # launch
    p_launch = subparsers.add_parser("launch", help="啟動應用程式並支援直接派駐至虛擬第二螢幕 (0 彈窗干擾)")
    p_launch.add_argument("path", help="可執行檔路徑或指令名稱 (如: 'notepad.exe', 'C:\\...\\app.exe')")
    p_launch.add_argument("-a", "--args", default="", help="傳遞給應用程式的參數字串 (如: '--force-renderer-accessibility https://...')")
    p_launch.add_argument("--screen", choices=["virtual", "primary"], default="virtual", help="目標顯示螢幕 (預設 virtual 虛擬第二螢幕)")
    p_launch.add_argument("--working-dir", help="指定工作目錄")
    p_launch.add_argument("--no-wait", action="store_true", help="不等待頂層視窗建立立即返回")
    p_launch.add_argument("--timeout", type=int, default=8000, help="等待視窗超時毫秒數 (預設 8000)")

    # close
    p_close = subparsers.add_parser("close", help="關閉目標視窗或結束進程 (先優雅 WM_CLOSE，可選 --force 強殺)")
    p_close.add_argument("target", nargs="?", help="目標視窗 HWND 或 PID (可為 0xHEX 或十進位，若省略則關閉當前鎖定視窗)")
    p_close.add_argument("-f", "--force", action="store_true", help="若超時未退出則強制 TerminateProcess 結束進程")
    p_close.add_argument("--timeout", type=int, default=2500, help="等待視窗退出的超時毫秒數 (預設 2500)")

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
            send_char = None if (ctrl or alt) else char
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    h = parse_hwnd(args.hwnd)
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.08)

                # execute_key_direct 已內建修飾鍵 (Ctrl/Alt/Shift) 原子按壓與釋放及主畫面焦點歸還
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                    "vk": vk, "char": send_char, "ctrl": ctrl, "alt": alt, "shift": shift
                }))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({
                    "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                    "vk": vk, "char": send_char, "ctrl": ctrl, "alt": alt, "shift": shift
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

                delta_val = getattr(args, "delta", 120)
                if sub == "click":
                    # 先讓 AI 虛擬游標平滑飛行至目標座標 (0% 實體滑鼠位移)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": "none", "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.16)
                    # 執行原子化 click
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "click",
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.08)
                elif sub == "wheel":
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "wheel",
                        "button": "left", "u": u_val, "v": v_val, "delta": delta_val
                    }))
                    await asyncio.sleep(0.08)
                else:
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": sub,
                        "button": args.button, "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.06)
                print(f"[MOUSE] 已執行 AI 虛擬游標 {sub} 於 ({u_val}, {v_val}) [{args.button}]")
        asyncio.run(exec_mouse())

    elif args.command == "drag":
        start_u = args.start_u
        start_v = args.start_v
        end_u = args.end_u
        end_v = args.end_v
        btn = args.button
        steps = max(1, args.steps)
        dur = max(0.05, args.duration)

        async def exec_drag():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    h = parse_hwnd(args.hwnd)
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.08)

                # 1. 移動游標至拖曳起點
                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                    "button": "none", "u": start_u, "v": start_v, "delta": 0
                }))
                await asyncio.sleep(0.06)

                # 2. 按下滑鼠按鈕 (down)
                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "down",
                    "button": btn, "u": start_u, "v": start_v, "delta": 0
                }))
                await asyncio.sleep(0.04)

                # 3. 平滑連續插值移動 (move)
                step_dt = dur / steps
                for i in range(1, steps + 1):
                    frac = i / steps
                    cur_u = round(start_u + (end_u - start_u) * frac, 4)
                    cur_v = round(start_v + (end_v - start_v) * frac, 4)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": btn, "u": cur_u, "v": cur_v, "delta": 0
                    }))
                    await asyncio.sleep(step_dt)

                # 4. 釋放滑鼠按鈕 (up)
                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "up",
                    "button": btn, "u": end_u, "v": end_v, "delta": 0
                }))
                await asyncio.sleep(0.06)
                print(f"[DRAG] 已完成拖曳操作: ({start_u}, {start_v}) -> ({end_u}, {end_v}) [按鍵: {btn}, 步數: {steps}]")
        asyncio.run(exec_drag())

    elif args.command == "wheel":
        u_val = args.u
        v_val = args.v
        delta_val = args.delta

        async def exec_wheel():
            import websockets
            async with websockets.connect(WS_URL) as ws:
                if args.hwnd:
                    h = parse_hwnd(args.hwnd)
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.08)

                await ws.send(json.dumps({
                    "action": "mouse", "source": "agent", "mode": "direct", "subaction": "wheel",
                    "button": "left", "u": u_val, "v": v_val, "delta": delta_val
                }))
                await asyncio.sleep(0.08)
                print(f"[WHEEL] 已執行滾輪滾動: 座標 ({u_val}, {v_val}), delta={delta_val} ({'向上滾動' if delta_val > 0 else '向下滾動'})")
        asyncio.run(exec_wheel())

    elif args.command == "open-url":
        async def exec_open_url():
            import websockets
            saved_clipboard = get_clipboard_text()
            async with websockets.connect(WS_URL) as ws:
                h = parse_hwnd(args.hwnd) if args.hwnd else None
                if h:
                    await ws.send(json.dumps({"action": "set_window", "hwnd": h}))
                    await asyncio.sleep(0.3)

                # 1. 聚焦網址列 (Ctrl+L)
                print("[OPEN-URL] 正在聚焦網址列 (Ctrl+L)...")
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 17}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 76}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 76}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 17}))
                await asyncio.sleep(0.25)

                # 2. 注入網址 (Windows 原生剪貼簿 + Ctrl+V 即時貼入，並自動備份還原使用者剪貼簿)
                print(f"[OPEN-URL] 正在貼入目標網址 ({len(args.url)} 字元): {args.url}...")
                set_clipboard_text(args.url)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 17}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 86}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 86}))
                await asyncio.sleep(0.04)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 17}))
                await asyncio.sleep(0.20)

                if saved_clipboard is not None:
                    set_clipboard_text(saved_clipboard)

                # 3. 按下 Enter 進行導航 (VK_RETURN 13)，key up 時引擎會自動歸還主螢幕焦點
                print("[OPEN-URL] 正在送出 Enter 進行導航...")
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 13}))
                await asyncio.sleep(0.05)
                await ws.send(json.dumps({"action": "key", "source": "agent", "mode": "direct", "subaction": "up", "vk": 13}))
                await asyncio.sleep(0.15)
                await ws.send(json.dumps({"action": "restore_primary_focus"}))
                await asyncio.sleep(0.25)
                print("[OPEN-URL] 導航指令已發送，剪貼簿與主畫面焦點已自動歸還！")

        asyncio.run(exec_open_url())

    elif args.command == "uia":
        cmd = {"action": "get_uia_tree", "depth": args.depth}
        if args.hwnd:
            cmd["hwnd"] = parse_hwnd(args.hwnd)
        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_tree"))
        if not res or res.get("status") == "error":
            msg = res.get("message") if isinstance(res, dict) else "無法取得 UIA 樹"
            print(f"[ERROR] UIA 掃描失敗: {msg}")
            sys.exit(1)

        tree = res.get("data", {}) if isinstance(res, dict) else {}

        if args.json:
            print(json.dumps(res, ensure_ascii=False, indent=2))
            return

        mons = list_system_monitors()
        vm = next((m for m in mons if not m["primary"]), None)
        pm = next((m for m in mons if m["primary"]), None)
        root_rect = tree.get("bounding_rect", [0, 0, 0, 0])

        def calc_uv(cx, cy):
            if vm and vm["width"] > 0 and vm["height"] > 0:
                if vm["x"] <= cx <= vm["x"] + vm["width"] and vm["y"] <= cy <= vm["y"] + vm["height"]:
                    return round((cx - vm["x"]) / vm["width"], 4), round((cy - vm["y"]) / vm["height"], 4)
            if root_rect and len(root_rect) == 4 and root_rect[2] > 0 and root_rect[3] > 0:
                if root_rect[0] <= cx <= root_rect[0] + root_rect[2] and root_rect[1] <= cy <= root_rect[1] + root_rect[3]:
                    if vm and vm["width"] > 0 and vm["height"] > 0 and root_rect[0] >= vm["x"] - 50:
                        return round((cx - vm["x"]) / vm["width"], 4), round((cy - vm["y"]) / vm["height"], 4)
                    return round((cx - root_rect[0]) / root_rect[2], 4), round((cy - root_rect[1]) / root_rect[3], 4)
            if pm and pm["width"] > 0 and pm["height"] > 0:
                return round((cx - pm["x"]) / pm["width"], 4), round((cy - pm["y"]) / pm["height"], 4)
            return None, None

        all_nodes = []
        ref_nodes = []
        matches = []

        def walk(node, depth=0):
            ref_id = node.get("ref_id")
            name = node.get("name", "").strip()
            ctype = node.get("control_type", "")
            rect = node.get("bounding_rect", [0, 0, 0, 0])
            center = node.get("center_point", [0, 0])
            cid = node.get("id", "")
            val = node.get("value")
            tog = node.get("toggle_state")
            exp = node.get("expand_state")
            sel = node.get("is_selected")
            focused = node.get("is_focused", False)
            enabled = node.get("is_enabled", True)
            pats = node.get("supported_patterns", [])

            has_size = rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0
            if name or has_size or ref_id:
                u, v = calc_uv(center[0], center[1]) if center and len(center) == 2 else (None, None)
                state_parts = []
                if val is not None:
                    v_disp = val if len(str(val)) <= 24 else str(val)[:21] + "..."
                    state_parts.append(f'val="{v_disp}"')
                if tog is not None:
                    state_parts.append(f"toggle={tog}")
                if exp is not None:
                    state_parts.append(f"{exp}")
                if sel is True:
                    state_parts.append("Selected")
                if focused:
                    state_parts.append("FOCUSED")
                if not enabled:
                    state_parts.append("DISABLED")

                item = {
                    "ref": ref_id or "-",
                    "depth": depth,
                    "type": ctype,
                    "name": name,
                    "rect": rect,
                    "center": center,
                    "id": cid,
                    "u": u,
                    "v": v,
                    "state": ", ".join(state_parts) if state_parts else "-",
                    "patterns": pats,
                }
                all_nodes.append(item)
                if ref_id:
                    ref_nodes.append(item)

                if args.find:
                    kw = args.find.lower()
                    if (
                        kw in name.lower()
                        or kw in ctype.lower()
                        or kw in cid.lower()
                        or (ref_id and kw == ref_id.lower())
                    ):
                        matches.append(item)

            for child in node.get("children", []):
                walk(child, depth + 1)

        walk(tree)

        if args.find:
            print(f"[UIA-FIND] 關鍵字 '{args.find}' 搜尋結果 (共 {len(matches)} 筆匹配):")
            print("-" * 122)
            print(f"{'Ref':<6} | {'深度':<4} | {'類型':<12} | {'歸一化 (u, v)':<17} | {'狀態 / 值':<22} | {'名稱 / 標籤 [動作]'}")
            print("-" * 122)
            for item in matches:
                uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                name_display = item['name'][:42] if item['name'] else (f"[ID: {item['id']}]" if item['id'] else "-")
                pats_str = f" [{','.join(item['patterns'])}]" if item['patterns'] else ""
                print(f"{item['ref']:<6} | {item['depth']:<4} | {item['type']:<12} | {uv_str:<17} | {item['state'][:22]:<22} | {name_display}{pats_str}")
            print("-" * 122)
            if matches:
                m0 = matches[0]
                if m0["ref"] != "-":
                    print(f"💡 語意觸發提示: python.exe -X utf8 cockpit_cli.py act {m0['ref']}")
                elif m0["u"] is not None:
                    print(f"💡 座標點擊提示: python.exe -X utf8 cockpit_cli.py mouse click {m0['u']} {m0['v']}")
        elif args.summary:
            from collections import Counter
            ctls = Counter(n["type"] for n in all_nodes)
            print(f"[UIA-SUMMARY] 節點總數: {len(all_nodes)} | 可互動 @Ref 總數: {len(ref_nodes)}")
            print("控制項類型分布:")
            for ctl, count in ctls.most_common(12):
                print(f"  - {ctl:<16}: {count}")
            named = [n for n in all_nodes if n["name"]]
            print(f"具備文字語意名稱節點數: {len(named)}")
        else:
            win_title = tree.get("name", "未知視窗")
            print(f"[UIA-SNAPSHOT] 視窗: \"{win_title}\" | 總節點: {len(all_nodes)} | 可互動 @Ref: {len(ref_nodes)}")
            if not ref_nodes:
                print("⚠️ 此視窗無可互動的 UIA @Ref 控制項（可能為 Blender/CAD/自繪 Canvas），請改用 `capture` + `mouse click <u, v>` 操控。")
                for item in all_nodes[:10]:
                    uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                    print(f"  - [{item['type']}] \"{item['name']}\" uv={uv_str}")
            else:
                print("-" * 122)
                print(f"{'Ref':<6} | {'類型':<12} | {'歸一化 (u, v)':<17} | {'狀態 / 當前值':<24} | {'名稱 / ID [支援 Pattern]'}")
                print("-" * 122)
                for item in ref_nodes:
                    uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                    label = f"\"{item['name'][:40]}\"" if item['name'] else (f"[ID:{item['id'][:30]}]" if item['id'] else "(無標籤)")
                    pats_str = f" [{','.join(item['patterns'])}]" if item['patterns'] else ""
                    print(f"{item['ref']:<6} | {item['type']:<12} | {uv_str:<17} | {item['state'][:24]:<24} | {label}{pats_str}")
                print("-" * 122)
                print("💡 操作提示: `cockpit_cli.py act @1` (靜默觸發/自動降級點擊) | `cockpit_cli.py fill @N \"文字\"` (瞬間設值)")

    elif args.command in ("act", "invoke", "fill", "set-value", "toggle", "expand", "collapse"):
        hwnd = parse_hwnd(args.hwnd) if getattr(args, "hwnd", None) else None
        if args.command in ("act", "invoke"):
            subaction = "act"
            val = None
        elif args.command in ("fill", "set-value"):
            subaction = "set_value"
            val = args.text
        else:
            subaction = args.command
            val = None

        cmd = {
            "action": "uia_action",
            "target": args.target,
            "subaction": subaction,
        }
        if hwnd:
            cmd["hwnd"] = hwnd
        if val is not None:
            cmd["value"] = val

        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_action_done"))
        if not res:
            print("[ERROR] UIA 動作未收到引擎回應")
            sys.exit(1)

        status = res.get("status")
        data = res.get("data", {}) if isinstance(res.get("data"), dict) else {}
        msg = res.get("message", "")

        if status == "ok":
            ver_str = f" | 狀態/驗證值: {data.get('verified_value')}" if data.get("verified_value") is not None else ""
            print(
                f"[UIA-{subaction.upper()}] ✅ 成功: {data.get('target')} -> "
                f"[{data.get('control_type')}] \"{data.get('resolved_name')}\" "
                f"(機制: {data.get('mechanism')}){ver_str}"
            )
        elif status == "fallback_required" and not getattr(args, "no_fallback", False):
            center = data.get("center_point", [0, 0])
            mons = list_system_monitors()
            vm = next((m for m in mons if not m["primary"]), None)
            pm = next((m for m in mons if m["primary"]), None)
            u_val, v_val = 0.5, 0.5
            if center and len(center) == 2:
                cx, cy = center[0], center[1]
                if vm and vm["width"] > 0 and vm["height"] > 0 and cx >= vm["x"] - 50:
                    u_val = max(0.0, min(1.0, round((cx - vm["x"]) / vm["width"], 4)))
                    v_val = max(0.0, min(1.0, round((cy - vm["y"]) / vm["height"], 4)))
                elif pm and pm["width"] > 0 and pm["height"] > 0:
                    u_val = max(0.0, min(1.0, round((cx - pm["x"]) / pm["width"], 4)))
                    v_val = max(0.0, min(1.0, round((cy - pm["y"]) / pm["height"], 4)))

            async def exec_fallback():
                import websockets
                async with websockets.connect(WS_URL) as ws:
                    if hwnd:
                        await ws.send(json.dumps({"action": "set_window", "hwnd": hwnd}))
                        await asyncio.sleep(0.08)

                    # 先點擊目標元素中心點以確保聚焦或觸發
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": "none", "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.12)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "click",
                        "button": "left", "u": u_val, "v": v_val, "delta": 0
                    }))
                    await asyncio.sleep(0.10)

                    if subaction == "set_value" and val is not None:
                        if not getattr(args, "append", False):
                            # Ctrl+A 全選後輸入新文字覆蓋
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                                "vk": ord("A"), "ctrl": True, "alt": False, "shift": False
                            }))
                            await asyncio.sleep(0.04)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                                "vk": ord("A"), "ctrl": True, "alt": False, "shift": False
                            }))
                            await asyncio.sleep(0.05)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                                "vk": 8, "ctrl": False, "alt": False, "shift": False
                            }))
                            await asyncio.sleep(0.03)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                                "vk": 8, "ctrl": False, "alt": False, "shift": False
                            }))
                            await asyncio.sleep(0.05)

                        t_cmd = {"action": "type_text", "text": val}
                        if hwnd:
                            t_cmd["hwnd"] = hwnd
                        await ws.send(json.dumps(t_cmd))
                        await asyncio.sleep(0.15)
                        print(
                            f"[UIA-FILL] ⚡ {args.target} (\"{data.get('resolved_name')}\") 不支援直接 ValuePattern，"
                            f"已自動降級執行點擊聚焦 ({u_val}, {v_val}) + WM_CHAR 注入 ({len(val)} 字元)！"
                        )
                    else:
                        print(
                            f"[UIA-{subaction.upper()}] ⚡ {args.target} (\"{data.get('resolved_name')}\") 未提供靜默 Pattern，"
                            f"已自動降級執行虛擬游標點擊 ({u_val}, {v_val})！"
                        )

            asyncio.run(exec_fallback())
        else:
            print(f"[ERROR] UIA 動作失敗 ({args.target}): {msg}")
            sys.exit(1)

    elif args.command == "diff":
        if getattr(args, "hwnd", None):
            h_val = parse_hwnd(args.hwnd)
            asyncio.run(send_cmd({"action": "set_window", "hwnd": h_val}, wait_for_event="window_set"))
        res = asyncio.run(send_cmd({"action": "get_visual_diff"}, wait_for_event="visual_diff"))
        if getattr(args, "json", False):
            print(json.dumps(res, ensure_ascii=False, indent=2))
            return
        diff = res.get("data") if isinstance(res, dict) else None
        if not diff:
            print("[DIFF] 尚無足夠的歷史快照可供比對（請至少執行 2 次 uia 或 diff）。")
        else:
            print("[DIFF] Visual Memory 狀態與版面差異比對報告:")
            add_w = diff.get("added_windows", [])
            cls_w = diff.get("closed_windows", [])
            dlg_w = diff.get("new_dialogs", [])
            add_e = diff.get("added_elements", [])
            rem_e = diff.get("removed_elements", [])
            chg_e = diff.get("state_changed_elements", [])
            if dlg_w:
                print(f"  🚨 新彈出對話框 ({len(dlg_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in dlg_w))
            if add_w:
                print(f"  ➕ 新增視窗 ({len(add_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in add_w))
            if cls_w:
                print(f"  ➖ 關閉視窗 ({len(cls_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in cls_w))
            if chg_e:
                print(f"  🔄 狀態/數值變更控制項 ({len(chg_e)}):")
                for c in chg_e[:25]:
                    print(f"     * {c}")
            if add_e:
                print(f"  🆕 新增控制項 ({len(add_e)}):")
                for a in add_e[:25]:
                    print(f"     + {a}")
            if rem_e:
                print(f"  🗑️ 移除控制項 ({len(rem_e)}):")
                for r in rem_e[:15]:
                    print(f"     - {r}")
            if not any([dlg_w, add_w, cls_w, chg_e, add_e, rem_e]):
                print("  ✅ 畫面與控制項狀態無變化。")

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

    elif args.command == "launch":
        import shlex
        parsed_args = shlex.split(args.args) if isinstance(args.args, str) and args.args.strip() else (args.args if isinstance(args.args, list) else [])
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
            print(f"[SUCCESS] 應用程式已成功啟動！")
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
            val = parse_hwnd(args.target)
            cmd["hwnd"] = val
        timeout_val = max(8.0, (args.timeout / 1000.0) + 4.0)
        res = asyncio.run(send_cmd(cmd, wait_for_event="window_closed", timeout=timeout_val))
        if res and res.get("status") == "ok":
            data = res.get("data", {})
            h_val = data.get("hwnd")
            h_str = f"0x{h_val:X}" if h_val else "未鎖定"
            p_val = data.get("pid")
            forced_str = " (強制終止)" if data.get("forced") else " (優雅退出)"
            print(f"[SUCCESS] 視窗/進程已關閉{forced_str}: HWND={h_str}, PID={p_val}")
        else:
            msg = res.get("message") if res else "未知錯誤"
            print(f"[ERROR] 關閉失敗: {msg}")
            sys.exit(1)

if __name__ == "__main__":
    main()
