import argparse
import sys
from pathlib import Path

# 確保可匯入同目錄下的模組（支援從任何工作目錄執行）
_SCRIPT_DIR = str(Path(__file__).resolve().parent)
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

# 強制 UTF-8 編碼避免 Windows 終端亂碼
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

import cmd_input
import cmd_uia
import cmd_window


def build_parser():
    parser = argparse.ArgumentParser(description="Desktop Cockpit CLI (ADC 2.0)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    p_list = subparsers.add_parser("list", help="列出系統活動視窗清單")
    p_list.add_argument("-f", "--filter", help="過濾視窗標題關鍵字")

    p_dispatch = subparsers.add_parser("dispatch", help="派駐視窗至虛擬第二螢幕")
    p_dispatch.add_argument("hwnd", help="視窗 HWND (可為 0xHEX 或十進位)")

    p_restore = subparsers.add_parser("restore", help="將視窗移回主實體螢幕")
    p_restore.add_argument("hwnd", help="視窗 HWND")

    p_set = subparsers.add_parser("set", help="鎖定目標串流視窗")
    p_set.add_argument("hwnd", help="視窗 HWND")

    p_cap = subparsers.add_parser("capture", help="截取目標視窗或虛擬螢幕當前 DXGI 畫面")
    p_cap.add_argument("-o", "--output", default="screen.jpg", help="輸出 JPEG 檔案路徑")
    p_cap.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_hotkey = subparsers.add_parser("hotkey", help="直通發送快捷鍵 (如 ctrl+l, enter, esc)")
    p_hotkey.add_argument("keys", help="按鍵組合 (如: 'ctrl+l', 'enter', 'alt+f4')")
    p_hotkey.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    p_type = subparsers.add_parser("type", help="向目標視窗注入文字")
    p_type.add_argument("text", help="欲輸入的文字字串")
    p_type.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_mouse = subparsers.add_parser("mouse", help="投遞滑鼠操作")
    p_mouse.add_argument("subaction", choices=["click", "down", "up", "move", "wheel"], help="滑鼠動作")
    p_mouse.add_argument("u", type=float, help="歸一化 X 座標 (0.0~1.0)")
    p_mouse.add_argument("v", type=float, help="歸一化 Y 座標 (0.0~1.0)")
    p_mouse.add_argument("-b", "--button", default="left", choices=["left", "middle", "right"], help="按鍵")
    p_mouse.add_argument("-d", "--delta", type=int, default=120, help="滾動量 (+120 向上, -120 向下，僅在 wheel 時生效)")
    p_mouse.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    p_drag = subparsers.add_parser("drag", help="執行平滑連續拖曳操作 (down -> 多幀 move -> up)")
    p_drag.add_argument("start_u", type=float, help="起點歸一化 X 座標 (0.0~1.0)")
    p_drag.add_argument("start_v", type=float, help="起點歸一化 Y 座標 (0.0~1.0)")
    p_drag.add_argument("end_u", type=float, help="終點歸一化 X 座標 (0.0~1.0)")
    p_drag.add_argument("end_v", type=float, help="終點歸一化 Y 座標 (0.0~1.0)")
    p_drag.add_argument("-b", "--button", default="left", choices=["left", "middle", "right"], help="按鍵 (預設 left，3D 視角旋轉可選 middle)")
    p_drag.add_argument("--steps", type=int, default=8, help="中間插值步數 (預設 8)")
    p_drag.add_argument("--duration", type=float, default=0.25, help="拖曳耗時秒數 (預設 0.25s)")
    p_drag.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    p_wheel = subparsers.add_parser("wheel", help="投遞滑鼠滾輪滾動")
    p_wheel.add_argument("u", type=float, default=0.5, nargs="?", help="歸一化 X 座標 (預設 0.5)")
    p_wheel.add_argument("v", type=float, default=0.5, nargs="?", help="歸一化 Y 座標 (預設 0.5)")
    p_wheel.add_argument("-d", "--delta", type=int, default=120, help="滾動量 (+120 向上, -120 向下)")
    p_wheel.add_argument("--hwnd", help="可選特定目標視窗 HWND")

    p_url = subparsers.add_parser("open-url", help="在瀏覽器視窗中導航至指定網址 (自動 Ctrl+L -> 輸入網址 -> Enter)")
    p_url.add_argument("url", help="目標網址 (如: https://www.104.com.tw)")
    p_url.add_argument("--hwnd", help="可選特定瀏覽器 HWND")

    p_uia = subparsers.add_parser("uia", help="取得目標視窗之 UIA 語意樹 (預設輸出精簡 @Ref 清單)")
    p_uia.add_argument("-d", "--depth", type=int, default=8, help="邏輯遍歷深度 (預設 8，自動穿透透明容器)")
    p_uia.add_argument("--hwnd", help="可選視窗 HWND")
    p_uia.add_argument("-f", "--find", help="搜尋特定名稱、控制項類型、@Ref 或 ID 之節點")
    p_uia.add_argument("-s", "--summary", action="store_true", help="輸出精簡概覽與統計資訊")
    p_uia.add_argument("-j", "--json", action="store_true", help="輸出原始完整 JSON 結構")

    p_act = subparsers.add_parser("act", aliases=["invoke"], help="透過 @Ref 或名稱執行語意觸發 (優先 COM Pattern，失敗自動降級虛擬游標點擊)")
    p_act.add_argument("target", help="目標 @Ref (如 '@1') 或控制項名稱/AutomationId (如 '儲存')")
    p_act.add_argument("--hwnd", help="可選特定視窗 HWND")
    p_act.add_argument("--no-fallback", action="store_true", help="若無對應 COM Pattern 時不自動回退至 (u, v) 座標點擊")

    p_fill = subparsers.add_parser("fill", aliases=["set-value"], help="透過 @Ref 或名稱向輸入框/滑桿瞬間寫入文字或數值")
    p_fill.add_argument("target", help="目標 @Ref (如 '@3') 或控制項名稱/AutomationId")
    p_fill.add_argument("text", nargs="?", default="", help="欲寫入的文字字串或數值 (省略則清空)")
    p_fill.add_argument("--hwnd", help="可選特定視窗 HWND")
    p_fill.add_argument("--append", action="store_true", help="降級為 WM_CHAR 輸入時不先執行 Ctrl+A 清空")
    p_fill.add_argument("--no-fallback", action="store_true", help="若不支援 ValuePattern 時不自動降級為 WM_CHAR 輸入")

    p_toggle = subparsers.add_parser("toggle", help="透過 @Ref 或名稱切換核取方塊/開關狀態 (TogglePattern)")
    p_toggle.add_argument("target", help="目標 @Ref (如 '@2') 或控制項名稱")
    p_toggle.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_expand = subparsers.add_parser("expand", help="展開指定選單、下拉框或樹狀節點 (ExpandCollapsePattern)")
    p_expand.add_argument("target", help="目標 @Ref 或名稱")
    p_expand.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_collapse = subparsers.add_parser("collapse", help="收合指定選單、下拉框或樹狀節點 (ExpandCollapsePattern)")
    p_collapse.add_argument("target", help="目標 @Ref 或名稱")
    p_collapse.add_argument("--hwnd", help="可選特定視窗 HWND")

    p_diff = subparsers.add_parser("diff", help="比對 Visual Memory 快照差異 (含視窗、@Ref 節點新增/移除/狀態與數值變化)")
    p_diff.add_argument("--hwnd", help="可選特定視窗 HWND (預設使用目前鎖定視窗)")
    p_diff.add_argument("-j", "--json", action="store_true", help="輸出原始 JSON")

    p_display = subparsers.add_parser("display", help="控制或查詢虛擬顯示器硬體狀態 (0 批次檔、0 UAC 彈窗)")
    p_display.add_argument("action", choices=["on", "off", "status"], help="on=開啟, off=關閉, status=查詢狀態")

    p_launch = subparsers.add_parser("launch", help="啟動應用程式並支援直接派駐至虛擬第二螢幕 (0 彈窗干擾)")
    p_launch.add_argument("path", help="可執行檔路徑或指令名稱 (如: 'notepad.exe', 'C:\\...\\app.exe')")
    p_launch.add_argument("-a", "--args", default="", help="傳遞給應用程式的參數字串 (如: '--force-renderer-accessibility https://...')")
    p_launch.add_argument("--screen", choices=["virtual", "primary"], default="virtual", help="目標顯示螢幕 (預設 virtual 虛擬第二螢幕)")
    p_launch.add_argument("--working-dir", help="指定工作目錄")
    p_launch.add_argument("--no-wait", action="store_true", help="不等待頂層視窗建立立即返回")
    p_launch.add_argument("--timeout", type=int, default=8000, help="等待視窗超時毫秒數 (預設 8000)")

    p_close = subparsers.add_parser("close", help="關閉目標視窗或結束進程 (先優雅 WM_CLOSE，可選 --force 強殺)")
    p_close.add_argument("target", nargs="?", help="目標視窗 HWND 或 PID (可為 0xHEX 或十進位，若省略則關閉當前鎖定視窗)")
    p_close.add_argument("-f", "--force", action="store_true", help="若超時未退出則強制 TerminateProcess 結束進程")
    p_close.add_argument("--timeout", type=int, default=2500, help="等待視窗退出的超時毫秒數 (預設 2500)")

    return parser


def main():
    args = build_parser().parse_args()
    if args.command in cmd_window.HANDLED_COMMANDS:
        cmd_window.handle(args)
    elif args.command in cmd_input.HANDLED_COMMANDS:
        cmd_input.handle(args)
    elif args.command in cmd_uia.HANDLED_COMMANDS:
        cmd_uia.handle(args)


if __name__ == "__main__":
    main()
