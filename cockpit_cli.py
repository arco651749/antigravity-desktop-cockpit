import sys
import json
import asyncio
import argparse
from pathlib import Path

# 強制 UTF-8 編碼避免 Windows 終端亂碼
sys.stdout.reconfigure(encoding="utf-8")

WS_URL = "ws://127.0.0.1:48280"

async def send_cmd(cmd_dict, wait_for_event=None, capture_frame_path=None, timeout=5.0):
    import websockets
    try:
        async with websockets.connect(WS_URL, max_size=10*1024*1024) as ws:
            # 若為截圖請求，先接收當前影格
            if capture_frame_path:
                # 若需要刷新，可先發送 get_isolated_windows 或 needs_refresh
                pass

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
                            # 預設接收任何非串流常規事件
                            result_event = data
                            if not capture_frame_path or captured_bytes:
                                break
                    except Exception:
                        pass

            if capture_frame_path and captured_bytes:
                out_path = Path(capture_frame_path).resolve()
                out_path.parent.mkdir(parents=True, exist_ok=True)
                out_path.write_bytes(captured_bytes)
                print(f"[SUCCESS] 截圖已保存至: {out_path} ({len(captured_bytes)} bytes)")

            return result_event

    except ConnectionRefusedError:
        print("[ERROR] 無法連線至 Desktop Cockpit 引擎 (ws://127.0.0.1:48280)，請確認服務已啟動。")
        sys.exit(1)
    except Exception as e:
        print(f"[ERROR] 通訊異常: {e}")
        sys.exit(1)

def main():
    parser = argparse.ArgumentParser(description="Antigravity Desktop Cockpit (ADC 2.0) 控制命令列")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # 1. launch
    p_launch = subparsers.add_parser("launch", help="在隔離桌面中啟動應用程式")
    p_launch.add_argument("app", default="notepad++", nargs="?", help="應用程式名稱或路徑 (預設: notepad++)")

    # 2. capture
    p_cap = subparsers.add_parser("capture", help="截取隔離桌面即時畫面為 JPEG")
    p_cap.add_argument("-o", "--output", default="cockpit_screenshot.jpg", help="輸出圖片檔案路徑")

    # 3. uia
    p_uia = subparsers.add_parser("uia", help="取得目標視窗之 UIA 語意樹")
    p_uia.add_argument("-d", "--depth", type=int, default=4, help="遍歷深度 (預設: 4)")
    p_uia.add_argument("--hwnd", type=int, default=None, help="可選特定視窗 HWND")

    # 4. diff
    p_diff = subparsers.add_parser("diff", help="比對 Visual Memory 快照，偵測新跳出的視窗或對話框")

    # 5. type
    p_type = subparsers.add_parser("type", help="向目標視窗注入文字")
    p_type.add_argument("text", help="欲輸入的文字字串")
    p_type.add_argument("--hwnd", type=int, default=None, help="可選特定視窗 HWND")

    # 6. click
    p_click = subparsers.add_parser("click", help="透過 UIA InvokePattern 靜默點擊按鈕或元件")
    p_click.add_argument("name", help="按鈕文字或元件名稱 (例如: 'No', '儲存', '確定')")
    p_click.add_argument("--hwnd", type=int, default=None, help="可選特定視窗 HWND")

    # 7. close
    p_close = subparsers.add_parser("close", help="向視窗發送關閉訊息 (觸發儲存詢問等)")
    p_close.add_argument("--hwnd", type=int, default=None, help="可選特定視窗 HWND")

    # 8. list
    p_list = subparsers.add_parser("list", help="列出隔離桌面中的所有視窗")

    # 9. kill
    p_kill = subparsers.add_parser("kill", help="強制結束隔離進程")
    p_kill.add_argument("--pid", type=int, default=None, help="可選特定進程 PID")

    args = parser.parse_args()

    if args.command == "launch":
        cmd = {"action": "launch_isolated", "app": args.app}
        res = asyncio.run(send_cmd(cmd, wait_for_event="app_launched"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "capture":
        # 截取一幀 JPEG
        cmd = {"action": "get_isolated_windows"}
        res = asyncio.run(send_cmd(cmd, capture_frame_path=args.output, timeout=3.0))

    elif args.command == "uia":
        cmd = {"action": "get_uia_tree", "depth": args.depth}
        if args.hwnd:
            cmd["hwnd"] = args.hwnd
        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_tree"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "diff":
        cmd = {"action": "get_visual_diff"}
        res = asyncio.run(send_cmd(cmd, wait_for_event="visual_diff"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "type":
        cmd = {"action": "type_text", "text": args.text}
        if args.hwnd:
            cmd["hwnd"] = args.hwnd
        res = asyncio.run(send_cmd(cmd, wait_for_event="text_typed"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "click":
        cmd = {"action": "uia_invoke", "name": args.name}
        if args.hwnd:
            cmd["hwnd"] = args.hwnd
        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_invoked"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "close":
        cmd = {"action": "close_window"}
        if args.hwnd:
            cmd["hwnd"] = args.hwnd
        res = asyncio.run(send_cmd(cmd, wait_for_event="window_closed"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "list":
        cmd = {"action": "get_isolated_windows"}
        res = asyncio.run(send_cmd(cmd, wait_for_event="isolated_windows"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

    elif args.command == "kill":
        cmd = {"action": "kill_isolated_app"}
        if args.pid:
            cmd["pid"] = args.pid
        res = asyncio.run(send_cmd(cmd, wait_for_event="app_killed"))
        print(json.dumps(res, ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
