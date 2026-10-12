---
name: desktop-cockpit
description: 人機零干擾 Windows 虛擬顯示器桌面駕駛艙 (ADC 2.0)。在虛擬第二螢幕中自動化調度、操作、文字輸入、60 FPS DXGI 串流、UIA 樹感知與 Visual Memory 差異對話框偵測，實體滑鼠 0 位移、0 彈窗干擾。
---

# Desktop Cockpit 2.0 (ADC 2.0) AI Agent 操作指引

本 Skill 提供 AI Agent 在 Windows 環境下操控桌面應用程式的**零干擾 (Zero-Interference)** 規範。
所有目標應用程式均運行於獨立的虛擬第二螢幕 (`\\.\DISPLAY82`，位於 $X \ge 3200$)，**使用者的主實體螢幕完全不受遮蔽，實體滑鼠 0 位移、0 奪取焦點**。

---

## 1. 核心操控方式：WebSocket API (`ws://127.0.0.1:48280`)

AI Agent 可直接透過本機 WebSocket 連線至串流引擎發送 JSON 指令：

### 核心指令矩陣

| 指令 (`action`) | 說明 | 參數範例 |
| :--- | :--- | :--- |
| `get_windows` | 取得系統所有在線視窗與 HWND | `{ "action": "get_windows" }` |
| `get_monitors` | 取得 Windows 顯示器拓撲 | `{ "action": "get_monitors" }` |
| `plug_in_virtual_display` | 程式化掛載虛擬第二螢幕 (0 批次檔、0 UAC) | `{ "action": "plug_in_virtual_display" }` |
| `plug_out_virtual_display` | 程式化卸載虛擬第二螢幕 (恢復單螢幕) | `{ "action": "plug_out_virtual_display" }` |
| `dispatch_to_virtual` | 派駐視窗至虛擬螢幕 (未在線時自動掛載自癒) | `{ "action": "dispatch_to_virtual", "hwnd": 132924 }` |
| `restore_to_primary` | 將視窗移回主實體螢幕 | `{ "action": "restore_to_primary", "hwnd": 132924 }` |
| `mouse` | 投遞滑鼠操作 (點擊/移動/滾輪) | `{ "action": "mouse", "source": "agent", "mode": "direct", "subaction": "down", "button": "left", "u": 0.5, "v": 0.5 }` |
| `key` | 快捷鍵直通 (Ctrl+C, Ctrl+V 等) | `{ "action": "key", "source": "agent", "mode": "direct", "subaction": "down", "vk": 83, "ctrl": true }` |
| `type_text` | 逐字注入多語系文字或代碼 | `{ "action": "type_text", "text": "Hello World", "hwnd": 132924 }` |
| `get_uia_tree` | 提取視窗 UIA 語意控制項樹 | `{ "action": "get_uia_tree", "hwnd": 132924, "depth": 4 }` |
| `get_visual_diff` | Visual Memory 比對快照差異 | `{ "action": "get_visual_diff" }` |

### CLI 命令列快捷操作 (`cockpit_cli.py`)

AI Agent 或終端使用者可直接調用 Python CLI 執行全流程控制：
- **虛擬螢幕硬體控制 (零 UAC 彈窗)**：
  - `python.exe -X utf8 cockpit_cli.py display on`：掛載虛擬第二螢幕。
  - `python.exe -X utf8 cockpit_cli.py display off`：拔除虛擬螢幕，恢復為單螢幕。
  - `python.exe -X utf8 cockpit_cli.py display status`：檢視所有顯示器拓撲與在線狀態。
- **視窗派駐 (具備自動自癒掛載機制)**：
  - `python.exe -X utf8 cockpit_cli.py dispatch <hwnd>`：即使虛擬螢幕當前未開啟，指令會自動透過底層 IOCTL 掛載虛擬螢幕並精確派駐。
  - `python.exe -X utf8 cockpit_cli.py restore <hwnd>`：將視窗移回主螢幕。
- **畫面截取與即時輸入**：
  - `python.exe -X utf8 cockpit_cli.py capture -o screen.jpg`：截取 60 FPS DXGI 畫面。
  - `python.exe -X utf8 cockpit_cli.py type "文字內容"`：注入多語系文字。
  - `python.exe -X utf8 cockpit_cli.py hotkey "ctrl+l"`：發送快捷鍵。

完整參數規範請參閱：[`API_REFERENCE.md`](../../API_REFERENCE.md)

---

## 2. 標準自動化流程範例 (Standard Recipe)

當需要操作應用程式完成「啟動 -> 派駐 -> 編輯 -> 儲存或關閉」任務時，應依循以下精簡流程：

### 範例：目標應用自動化與彈窗處理

1. **派駐目標視窗至虛擬第二螢幕**：
   發送 `dispatch_to_virtual` 將目標視窗移往 $X=3200$。
2. **語意結構感知**：
   發送 `get_uia_tree` 獲取視窗內部各控制項（編輯區、功能按鈕、選單項目）之精確座標與屬性。
3. **精準點擊與文字輸入**：
   - 透過 `mouse` 點擊指定歸一化座標 $(u, v)$。
   - 透過 `type_text` 注入程式碼或內容。
4. **狀態差異比對 (Visual Memory)**：
   在執行關閉或確認動作後，呼叫 `get_visual_diff` 檢查是否有 `#32770` 等新彈出對話框。
5. **影像訊框驗收**：
   監聽二進位 JPEG 訊框，以視覺模型比對介面呈現是否符合預期。

---

## 3. 注意事項與最佳實踐

1. **零干擾保證**：所有操作必須發送至虛擬第二螢幕區域。
2. **人類第一優先仲裁**：發送指令時一律帶上 `"source": "agent"`。當使用者在右側面板進行手動操作時，引擎會自動暫停 AI 動作 1.5 秒禮讓人類。
3. **單一職責文字輸入**：一般文字輸入一律使用 `type_text` 或帶有 `char` 屬性之 `key`，杜絕任何重複字元連發。
