# Desktop Cockpit API 參考文件 (API Reference)

> **版本**：2.0 (Virtual Display + DXGI 60 FPS + RustDesk-Grade Direct Pass-through)  
> **端點**：`ws://127.0.0.1:48280`  
> **通訊協議**：標準 WebSocket（文字格式為 JSON，影像訊框為二進位 JPEG ArrayBuffer）

---

## 🌟 目錄
1. [連線與架構概述](#1-連線與架構概述)
2. [指令總覽 (API Matrix)](#2-指令總覽-api-matrix)
3. [詳細指令規範 (JSON Schema)](#3-詳細指令規範-json-schema)
   - [視窗與顯示拓撲](#視窗與顯示拓撲)
   - [滑鼠與鍵盤操作](#滑鼠與鍵盤操作)
   - [語意與視覺感知 (UIA + Visual Memory)](#語意與視覺感知-uia--visual-memory)
4. [伺服器推播事件與影像訊框](#4-伺服器推播事件與影像訊框)
5. [呼叫範例代碼 (Python / Node.js)](#5-呼叫範例代碼-python--nodejs)

---

## 1. 連線與架構概述

Desktop Cockpit 串流引擎（`gui-stream-engine`）本機監聽 `127.0.0.1:48280`。  
任何客戶端（Antigravity UI Extension 面板、AI Agent 自動化腳本、外部程式）皆可透過 WebSocket 連線：

- **雙軌對稱**：人類在 UI 面板手動點擊/打字，與 AI 透過腳本發送 JSON 指令，使用的是完全相同的後端仲裁管道。
- **人機仲裁機制 (Human-First Priority Arbiter)**：
  - 當包含 `"source": "user"` 的指令抵達時，引擎標記人類活躍並暫停 `"source": "agent"` 的操作 1.5 秒。
  - 當人類停止操作 1.5 秒後，AI 自動恢復控制權。
- **虛擬多螢幕絕對座標**：
  - 目標應用程式挪移至虛擬第二螢幕（如 `\\.\DISPLAY82`，位於 $X \ge 3200$）。
  - 實體滑鼠 0 位移、0 搶奪焦點、0 彈窗干擾。

---

## 2. 指令總覽 (API Matrix)

| Action (`action`) | 功能說明 | 核心參數 | AI / 程式調用場景 |
| :--- | :--- | :--- | :--- |
| **`get_windows`** | 查詢系統所有在線活動視窗清單 | 無 | 尋找目標應用程式的 HWND 視窗控制代碼 |
| **`get_monitors`** | 查詢 Windows 顯示器拓撲 | 無 | 偵測虛擬第二螢幕狀態 |
| **`plug_in_virtual_display`** | 程式化掛載虛擬第二螢幕 (0 批次檔、0 UAC) | 無 | 透過 Win32 DeviceIoControl 直通掛載虛擬螢幕 |
| **`plug_out_virtual_display`** | 程式化卸載虛擬第二螢幕 (恢復單螢幕) | 無 | 透過 Win32 DeviceIoControl 直通拔除虛擬螢幕 |
| **`dispatch_to_virtual`** | 將視窗挪移至虛擬第二螢幕 | `hwnd` | 視窗隔離：未在線時自動自癒掛載並移出主螢幕 |
| **`restore_to_primary`** | 將視窗移回主實體螢幕 | `hwnd` | 任務完成後將視窗還原回主螢幕給使用者查看 |
| **`set_window`** | 聚焦並鎖定目標串流視窗 | `hwnd` | 切換串流目標，並自動還原最小化視窗 |
| **`set_capture_mode`** | 設定串流擷取與輸入模式 | `mode` (`"direct"` / `"background"`) | 強制鎖定為 DXGI 直通模式 |
| **`mouse`** | 投遞滑鼠事件 (移動/點擊/滾輪/拖曳) | `source`, `mode`, `subaction`, `button`, `u`, `v`, `delta` | 模擬滑鼠在視窗畫布上的所有精確操作 |
| **`key`** | 投遞鍵盤單鍵或組合鍵事件 | `source`, `mode`, `subaction`, `vk`, `char`, `ctrl`, `alt`, `shift` | 快捷鍵觸發、全鍵盤穿透 |
| **`type_text`** | 逐字注入任意多語系文字或代碼 | `text`, `hwnd` | 高速文字輸入（相容中文、Emoji、符號） |
| **`get_uia_tree`** | 提取目標視窗 UIA 語意控制項樹 | `hwnd`, `depth` | 取得按鈕、文字框、選單座標與控制項類型 |
| **`get_visual_diff`** | Visual Memory 比對快照差異 | 無 | 動作後偵測是否有新彈出對話框或報錯視窗 |
| **`set_fps`** | 動態調整擷取幀率 | `fps` (1~60) | 調整效能檔位 (例如 30 FPS 或 60 FPS) |

---

## 3. 詳細指令規範 (JSON Schema)

### 視窗與顯示拓撲

#### `get_windows`
- **Request**:
  ```json
  { "action": "get_windows" }
  ```
- **Response Event**:
  ```json
  {
    "event": "windows",
    "status": "ok",
    "data": [
      {
        "hwnd": 132924,
        "title": "Untitled - Notepad",
        "width": 1920,
        "height": 1080
      }
    ]
  }
  ```

#### `dispatch_to_virtual`
- **Request**:
  ```json
  { "action": "dispatch_to_virtual", "hwnd": 132924 }
  ```
- **Response Event**:
  ```json
  {
    "event": "window_dispatched",
    "status": "ok",
    "message": "視窗已成功派駐至虛擬螢幕 (\\\\.\\DISPLAY82 @ 1920x1080)",
    "data": {
      "name": "\\\\.\\DISPLAY82",
      "x": 3200,
      "y": 0,
      "width": 1920,
      "height": 1080,
      "is_virtual_candidate": true
    }
  }
  ```

---

### 滑鼠與鍵盤操作

#### `mouse`
- **Request 範例（點擊按鈕）**:
  ```json
  {
    "action": "mouse",
    "source": "agent",
    "mode": "direct",
    "subaction": "down",
    "button": "left",
    "u": 0.4521,
    "v": 0.6180,
    "delta": 0
  }
  ```
- **參數說明**:
  - `source`: `"agent"`（AI 操作）或 `"user"`（人類操作）。
  - `mode`: `"direct"`（Direct Pass-through 物理座標直通，推薦）或 `"background"`。
  - `subaction`: `"down"`（按下）、`"up"`（放開）、`"move"`（移動）、`"wheel"`（滾輪）。
  - `button`: `"left"`、`"middle"`（3D CAD/Blender 視角旋轉）、`"right"`、`"none"`。
  - `u`, `v`: 歸一化視窗相對座標（範圍 $0.0 \sim 1.0$，自動映射至虛擬螢幕絕對物理像素）。
  - `delta`: 滾輪滾動量（正數向上，負數向下）。

#### `key`
- **Request 範例（快捷鍵 Ctrl+S）**:
  ```json
  {
    "action": "key",
    "source": "agent",
    "mode": "direct",
    "subaction": "down",
    "vk": 83,
    "char": null,
    "ctrl": true,
    "alt": false,
    "shift": false
  }
  ```
- **參數說明**:
  - `vk`: Windows 虛擬鍵碼（Virtual Key Code，例如 A=65, Enter=13, Esc=27）。
  - `char`: 可列印 Unicode 字元（若為純控制鍵傳 `null`）。有提供 `char` 時後端採單一職責投遞，絕不連發。
  - `ctrl`, `alt`, `shift`: 布林旗標。

#### `type_text`
- **Request 範例**:
  ```json
  {
    "action": "type_text",
    "text": "Hello Antigravity! 🚀",
    "hwnd": 132924
  }
  ```

---

### 語意與視覺感知 (UIA + Visual Memory)

#### `get_uia_tree`
- **Request**:
  ```json
  {
    "action": "get_uia_tree",
    "hwnd": 132924,
    "depth": 4
  }
  ```
- **Response Event**:
  ```json
  {
    "event": "uia_tree",
    "status": "ok",
    "data": {
      "name": "Untitled - Notepad",
      "control_type": "Window",
      "bounds": [3200, 0, 1920, 1080],
      "children": [
        {
          "name": "Text Editor",
          "control_type": "Document",
          "bounds": [3200, 80, 1920, 1000],
          "children": []
        }
      ]
    }
  }
  ```

#### `get_visual_diff`
- **Request**:
  ```json
  { "action": "get_visual_diff" }
  ```
- **Response Event**:
  ```json
  {
    "event": "visual_diff",
    "status": "ok",
    "data": {
      "new_dialogs": [],
      "added_windows": [],
      "closed_windows": [],
      "added_elements": []
    }
  }
  ```

---

## 4. 伺服器推播事件與影像訊框

### 影像訊框 (Binary JPEG Frames)
WebSocket 會在連線成功與目標視窗更新時，自動推播二進位 JPEG 資料流：
- **格式**：二進位 ArrayBuffer（完整的 JPEG 檔案格式，Quality=75）。
- **幀率**：自適應 1~60 FPS（動態髒矩形偵測，畫面靜止時 0% CPU）。
- **首幀保護**：新連線進來時立即主動交付快取首幀，防止黑屏。

---

## 5. 呼叫範例代碼 (Python / Node.js)

### Python 完整操作範例

```python
import asyncio
import json
import websockets

async def run_cockpit():
    uri = "ws://127.0.0.1:48280"
    async with websockets.connect(uri) as ws:
        # 1. 取得活動視窗
        await ws.send(json.dumps({"action": "get_windows"}))
        target_hwnd = None
        while True:
            msg = await ws.recv()
            if isinstance(msg, str):
                data = json.loads(msg)
                if data.get("event") == "windows":
                    for w in data.get("data", []):
                        if "notepad" in w.get("title", "").lower():
                            target_hwnd = w.get("hwnd")
                            break
                    break

        if not target_hwnd:
            print("找不到目標視窗！")
            return

        # 2. 派駐至虛擬第二螢幕
        await ws.send(json.dumps({"action": "dispatch_to_virtual", "hwnd": target_hwnd}))
        
        # 3. 點擊輸入文字
        await ws.send(json.dumps({
            "action": "mouse",
            "source": "agent",
            "mode": "direct",
            "subaction": "down",
            "button": "left",
            "u": 0.5, "v": 0.5
        }))
        await ws.send(json.dumps({
            "action": "mouse",
            "source": "agent",
            "mode": "direct",
            "subaction": "up",
            "button": "left",
            "u": 0.5, "v": 0.5
        }))

        # 4. 注入文字
        await ws.send(json.dumps({
            "action": "type_text",
            "text": "Hello from AI Agent!\\n",
            "hwnd": target_hwnd
        }))

        # 5. 接收畫面截圖驗證
        for _ in range(5):
            frame = await ws.recv()
            if isinstance(frame, bytes):
                with open("screen_result.jpg", "wb") as f:
                    f.write(frame)
                print(f"成功保存即時影像幀: {len(frame)} bytes")
                break

asyncio.run(run_cockpit())
```
