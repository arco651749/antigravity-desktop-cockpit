import { initAnnotator } from '/annotator.js';
import { initStreamInput } from '/input_handler.js';

// Desktop Cockpit - Frontend Stream Receiver & Input Controller
const canvas = document.getElementById('streamCanvas');
const ctx = canvas.getContext('2d');
const viewport = document.getElementById('viewport');
const windowSelect = document.getElementById('windowSelect');
const modeSelect = document.getElementById('modeSelect');
const fpsSelect = document.getElementById('fpsSelect');
const btnRefresh = document.getElementById('btnRefresh');
const btnEngineRestart = document.getElementById('btnEngineRestart');
const btnDispatchVirtual = document.getElementById('btnDispatchVirtual');
const btnRestorePrimary = document.getElementById('btnRestorePrimary');
const btnToggleDisplay = document.getElementById('btnToggleDisplay');
const virtualMonStatus = document.getElementById('virtualMonStatus');
const virtualMonText = document.getElementById('virtualMonText');
const arbiterStatus = document.getElementById('arbiterStatus');
const arbiterText = document.getElementById('arbiterText');
const aiCursorStat = document.getElementById('aiCursorStat');
const aiCursorText = document.getElementById('aiCursorText');
const aiVirtualCursor = document.getElementById('aiVirtualCursor');
const aiCursorBadge = document.getElementById('aiCursorBadge');
const aiCursorRippleHost = document.getElementById('aiCursorRippleHost');
const btnTestAiCursor = document.getElementById('btnTestAiCursor');
const hudStat = document.getElementById('hudStat');
const consoleLogs = document.getElementById('consoleLogs');


let ws = null;
let currentFPS = 30;
let frameCount = 0;
let lastFpsTime = performance.now();
let displayedFps = 30;
let currentCoords = '(0.000, 0.000)';
let aiBadgeTimer = null;

function updateHudStat(coords) {
  if (coords) currentCoords = coords;
  if (hudStat) {
    hudStat.textContent = `FPS: ${displayedFps} | 游標: ${currentCoords}`;
  }
}
let humanTimeout = null;
let detectedVirtualMon = null;

function log(sender, text, type = 'sys') {
  const now = new Date();
  const time = `[${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}.${String(now.getMilliseconds()).padStart(3, '0')}]`;
  const div = document.createElement('div');
  div.className = 'log-entry';
  let cls = 'log-sys';
  if (type === 'user') cls = 'log-user';
  if (type === 'agent') cls = 'log-agent';
  const timeSpan = document.createElement('span');
  timeSpan.className = 'log-time';
  timeSpan.textContent = time;
  const msgSpan = document.createElement('span');
  msgSpan.className = cls;
  msgSpan.textContent = `[${sender}] ${text}`;
  div.append(timeSpan, msgSpan);
  consoleLogs.appendChild(div);
  consoleLogs.scrollTop = consoleLogs.scrollHeight;
}

function setHumanActive(desc) {
  arbiterStatus.className = 'status-pill status-human';
  arbiterText.textContent = `👨‍💻 人類操作 (${desc})`;
  clearTimeout(humanTimeout);
  humanTimeout = setTimeout(() => {
    humanTimeout = null;
    arbiterStatus.className = 'status-pill status-guard';
    arbiterText.textContent = '🛡️ 主畫面焦點保護中';
  }, 1500);
}

function handleAiCursorEvent(data) {
  if (!data) return;
  const u = typeof data.u === 'number' ? Math.max(0, Math.min(1, data.u)) : 0.5;
  const v = typeof data.v === 'number' ? Math.max(0, Math.min(1, data.v)) : 0.5;
  const sub = data.subaction || 'move';

  if (aiVirtualCursor) {
    aiVirtualCursor.style.left = `${(u * 100).toFixed(2)}%`;
    aiVirtualCursor.style.top = `${(v * 100).toFixed(2)}%`;
  }

  const uvStr = `(${u.toFixed(3)}, ${v.toFixed(3)})`;
  updateHudStat(uvStr);

  if (sub === 'move') {
    if (data.source !== 'user') {
      arbiterStatus.className = 'status-pill status-ai';
      arbiterText.textContent = '🤖 AI 執行中 (焦點鎖定)';
    }
  } else if (sub === 'click' || sub === 'down') {
    if (aiVirtualCursor) {
      aiVirtualCursor.classList.add('clicking');
      setTimeout(() => aiVirtualCursor.classList.remove('clicking'), 220);
    }
    if (aiCursorRippleHost) {
      const ring = document.createElement('div');
      ring.className = 'click-ripple ripple-agent';
      aiCursorRippleHost.appendChild(ring);
      setTimeout(() => ring.remove(), 580);
    }
    if (data.source !== 'user') {
      arbiterStatus.className = 'status-pill status-guard';
      arbiterText.textContent = '🛡️ 點擊完成 (焦點已歸還)';
      log('AI 點擊', `⚡ 原子點擊 ${uvStr} 完成 → 焦點已無縫歸還主畫面`, 'agent');
    }
  } else if (sub === 'type') {
    const preview = data.text_preview || '';
    arbiterStatus.className = 'status-pill status-guard';
    arbiterText.textContent = '🛡️ 文字注入完成';
    log('AI 輸入', `⌨️ 注入文字: "${preview}" (主畫面未受干擾)`, 'agent');
  } else if (sub === 'key') {
    arbiterStatus.className = 'status-pill status-guard';
    arbiterText.textContent = '🛡️ 按鍵完成 (焦點已歸還)';
  }

  clearTimeout(aiBadgeTimer);
  aiBadgeTimer = setTimeout(() => {
    aiBadgeTimer = null;
    if (!humanTimeout) {
      arbiterStatus.className = 'status-pill status-guard';
      arbiterText.textContent = '🛡️ 主畫面焦點保護中';
    }
  }, 1400);
}

// Connect to Rust stream engine WebSocket
function connectWebSocket() {
  log('WebSocket', '正在連線至本機 Rust 串流引擎 ws://127.0.0.1:48280...', 'sys');
  ws = new WebSocket('ws://127.0.0.1:48280');
  ws.binaryType = 'arraybuffer';

  ws.onopen = () => {
    log('WebSocket', '成功連線至 Rust 引擎！正在查詢拓撲與視窗清單...', 'sys');
    ws.send(JSON.stringify({ action: 'set_capture_mode', mode: 'direct' }));
    ws.send(JSON.stringify({ action: 'get_windows' }));
    ws.send(JSON.stringify({ action: 'get_monitors' }));
  };

  ws.onmessage = async (evt) => {
    if (typeof evt.data === 'string') {
      try {
        const msg = JSON.parse(evt.data);
        if (msg.event === 'windows' && Array.isArray(msg.data)) {
          populateWindows(msg.data);
        } else if (msg.event === 'monitors' && Array.isArray(msg.data)) {
          updateMonitors(msg.data);
        } else if (msg.event === 'ai_cursor' && msg.data) {
          handleAiCursorEvent(msg.data);
        } else if (msg.event === 'window_dispatched') {
          if (msg.status === 'ok') {
            log('虛擬派駐', msg.message || '視窗已派駐至虛擬螢幕', 'sys');
            setVirtualBadgeActive(true, msg.data);
            if (msg.data && msg.data.hwnd && windowSelect) {
              const hwndStr = String(msg.data.hwnd);
              localStorage.setItem('desktop_cockpit_selected_hwnd', hwndStr);
              windowSelect.value = hwndStr;
              if (windowSelect.value !== hwndStr && ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({ action: 'get_windows' }));
              }
            }
            if (modeSelect) {
              modeSelect.value = 'direct';
              localStorage.setItem('desktop_cockpit_input_mode', 'direct');
            }
          } else {
            log('派駐失敗', msg.message || '無法派駐視窗', 'agent');
          }
        } else if (msg.event === 'window_set') {
          if (msg.status === 'ok' && msg.data && msg.data.hwnd && windowSelect) {
            const hwndStr = String(msg.data.hwnd);
            localStorage.setItem('desktop_cockpit_selected_hwnd', hwndStr);
            if (windowSelect.value !== hwndStr) {
              windowSelect.value = hwndStr;
            }
          }
        } else if (msg.event === 'window_restored') {
          if (msg.status === 'ok') {
            log('主螢幕回歸', msg.message || '視窗已移回主螢幕', 'sys');
            setVirtualBadgeActive(false);
          } else {
            log('回歸失敗', msg.message || '無法移回主螢幕', 'agent');
          }
        } else if (msg.event === 'app_launched') {
          log('隔離桌面', msg.message, 'sys');
          if (modeSelect) modeSelect.value = 'secret_desktop';
          ws.send(JSON.stringify({ action: 'get_isolated_windows' }));
        } else if (msg.event === 'isolated_windows' && Array.isArray(msg.data)) {
          log('隔離視窗', `發現 ${msg.data.length} 個隔離視窗: ` + msg.data.map(w => w.title || w.class_name).join(', '), 'sys');
          populateWindows(msg.data);
        } else if (msg.event === 'uia_tree') {
          if (msg.status === 'ok') {
            const chCount = msg.data.children ? msg.data.children.length : 0;
            log('UIA 樹', `根節點 [${msg.data.control_type}] "${msg.data.name}", 子控制項數: ${chCount}`, 'sys');
            console.log('[UIA Tree]', msg.data);
          } else {
            log('UIA 失敗', msg.message || '無法取得語意樹', 'agent');
          }
        } else if (msg.event === 'uia_invoked') {
          log('UIA 互動', msg.message, 'sys');
        } else if (msg.event === 'visual_diff') {
          if (msg.data) {
            const addedWins = msg.data.added_windows ? msg.data.added_windows.length : 0;
            const newDlgs = msg.data.new_dialogs ? msg.data.new_dialogs.length : 0;
            log('快照比對', `新增視窗: ${addedWins}, 新增對話框: ${newDlgs}`, newDlgs > 0 ? 'agent' : 'sys');
          }
        } else if (msg.event === 'input_ack') {
          log('底層確認', msg.message || '輸入事件已成功投遞', 'sys');
        } else if (msg.event === 'virtual_display_plugged' || msg.event === 'virtual_display_unplugged') {
          log('顯示控制', msg.message || '虛擬顯示器狀態已變更', 'sys');
          if (msg.data && Array.isArray(msg.data)) {
            updateMonitors(msg.data);
          } else if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ action: 'get_monitors' }));
          }
        } else if (msg.event === 'input_rejected') {
          log('輸入拒絕', msg.message || '輸入被阻擋', 'agent');
        }
      } catch (err) {
        console.error('JSON parse error:', err);
      }
    } else {
      // Binary JPEG frame
      const blob = new Blob([evt.data], { type: 'image/jpeg' });
      try {
        const bitmap = await createImageBitmap(blob);
        if (annotator.isAnnotating()) {
          // Freeze display: preserve snapshot while user is actively annotating!
          bitmap.close();
          return;
        }
        if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
          canvas.width = bitmap.width;
          canvas.height = bitmap.height;
        }
        ctx.drawImage(bitmap, 0, 0);
        bitmap.close();

        frameCount++;
        const now = performance.now();
        if (now - lastFpsTime >= 1000) {
          displayedFps = frameCount;
          frameCount = 0;
          lastFpsTime = now;
          updateHudStat();
        }
      } catch (err) {
        // Drop malformed frame
      }
    }
  };

  ws.onclose = () => {
    log('WebSocket', '連線中斷，將在 2 秒後重試連線...', 'sys');
    setTimeout(connectWebSocket, 2000);
  };

  ws.onerror = () => {
    log('WebSocket', '引擎未回應，請確認 gui-stream-engine 是否已啟動', 'sys');
  };
}

function populateWindows(windows) {
  const previousHwnd = localStorage.getItem('desktop_cockpit_selected_hwnd');
  windowSelect.innerHTML = '<option value="">-- 請選擇欲操作的視窗 --</option>';
  let autoSelectHwnd = null;

  windows.forEach((w) => {
    const opt = document.createElement('option');
    opt.value = w.hwnd;
    opt.textContent = `${w.title} (${w.width}x${w.height})`;
    windowSelect.appendChild(opt);

    if (previousHwnd && String(w.hwnd) === previousHwnd) {
      autoSelectHwnd = w.hwnd;
    }
  });

  // 若無先前紀錄，優先智慧預選 Chrome、Firefox、Notepad、Blender、KiCad 或第一個視窗
  if (!autoSelectHwnd && windows.length > 0) {
    const preferred = windows.find((w) => /chrome|firefox|edge|notepad|記事本|blender|kicad/i.test(w.title)) || windows[0];
    if (preferred) {
      autoSelectHwnd = preferred.hwnd;
    }
  }

  if (autoSelectHwnd) {
    windowSelect.value = autoSelectHwnd;
    localStorage.setItem('desktop_cockpit_selected_hwnd', autoSelectHwnd);
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'set_capture_mode', mode: getMode() }));
      ws.send(JSON.stringify({ action: 'set_window', hwnd: autoSelectHwnd }));
      log('視窗切換', `自動連結目標視窗 -> 0x${autoSelectHwnd.toString(16).toUpperCase()}`, 'sys');
    }
  }

  log('系統', `已載入 ${windows.length} 個本機活動視窗`, 'sys');
}

function updateMonitors(monitors) {
  detectedVirtualMon = monitors.find((m) => m.is_virtual_candidate);
  if (detectedVirtualMon && virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-virtual';
    virtualMonText.textContent = `🟢 虛擬艙在線 (${detectedVirtualMon.width}x${detectedVirtualMon.height})`;
    if (btnToggleDisplay) btnToggleDisplay.textContent = '🔌 拔除';
    log('顯示拓撲', `檢測到虛擬螢幕: ${detectedVirtualMon.name} (${detectedVirtualMon.width}x${detectedVirtualMon.height} @ X=${detectedVirtualMon.x})`, 'sys');
  } else if (virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-idle';
    virtualMonText.textContent = '⚪ 虛擬螢幕未啟用';
    if (btnToggleDisplay) btnToggleDisplay.textContent = '🔌 開啟';
  }
}

function setVirtualBadgeActive(active, monData) {
  if (active && virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-virtual-active';
    const resText = monData ? `${monData.width}x${monData.height}` : '1920x1080';
    virtualMonText.textContent = `🛡️ 隔離派駐中 (${resText})`;
  } else if (detectedVirtualMon && virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-virtual';
    virtualMonText.textContent = `🟢 虛擬艙在線 (${detectedVirtualMon.width}x${detectedVirtualMon.height})`;
  }
}

if (virtualMonStatus) {
  virtualMonStatus.style.cursor = 'pointer';
  virtualMonStatus.title = '點擊切換虛擬第二螢幕狀態 (0 批次檔、0 UAC)';
  virtualMonStatus.addEventListener('click', () => {
    if (btnToggleDisplay) btnToggleDisplay.click();
  });
}

// Virtual display dispatch & recall actions
if (btnDispatchVirtual) {
  btnDispatchVirtual.addEventListener('click', () => {
    const hwnd = parseInt(windowSelect.value, 10);
    if (!hwnd) {
      log('警告', '請先選擇欲派駐的視窗', 'agent');
      return;
    }
    if (ws && ws.readyState === WebSocket.OPEN) {
      log('調度', `正在派駐視窗 0x${hwnd.toString(16).toUpperCase()} 至虛擬第二螢幕...`, 'sys');
      ws.send(JSON.stringify({ action: 'dispatch_to_virtual', hwnd }));
    }
  });
}

if (btnRestorePrimary) {
  btnRestorePrimary.addEventListener('click', () => {
    const hwnd = parseInt(windowSelect.value, 10);
    if (!hwnd) {
      log('警告', '請先選擇欲移回的視窗', 'agent');
      return;
    }
    if (ws && ws.readyState === WebSocket.OPEN) {
      log('調度', `正在將視窗 0x${hwnd.toString(16).toUpperCase()} 移回主實體螢幕...`, 'sys');
      ws.send(JSON.stringify({ action: 'restore_to_primary', hwnd }));
    }
  });
}

if (btnToggleDisplay) {
  btnToggleDisplay.addEventListener('click', () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      log('警告', '引擎未連線，無法發送控制指令', 'agent');
      return;
    }
    if (detectedVirtualMon) {
      log('顯示控制', '正在透過 Win32 DeviceIoControl 卸載虛擬螢幕 (恢復單螢幕)...', 'sys');
      ws.send(JSON.stringify({ action: 'plug_out_virtual_display' }));
    } else {
      log('顯示控制', '正在透過 Win32 DeviceIoControl 掛載虛擬螢幕 (0 批次檔、0 UAC)...', 'sys');
      ws.send(JSON.stringify({ action: 'plug_in_virtual_display' }));
    }
  });
}

// Window & FPS controls
windowSelect.addEventListener('change', () => {
  const hwnd = parseInt(windowSelect.value, 10);
  if (hwnd && ws && ws.readyState === WebSocket.OPEN) {
    localStorage.setItem('desktop_cockpit_selected_hwnd', hwnd);
    ws.send(JSON.stringify({ action: 'set_capture_mode', mode: getMode() }));
    ws.send(JSON.stringify({ action: 'set_window', hwnd }));
    log('視窗切換', `切換目標視窗 HWND -> 0x${hwnd.toString(16).toUpperCase()}`, 'sys');
  }
});

if (fpsSelect) {
  fpsSelect.addEventListener('change', () => {
    const fps = parseInt(fpsSelect.value, 10);
    currentFPS = fps;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'set_fps', fps }));
      log('幀率時鐘', `切換為 ${fps} FPS 動態檔位`, 'sys');
    }
  });
}

// Mode controls & persistence
const savedMode = localStorage.getItem('desktop_cockpit_input_mode');
if (modeSelect) {
  if (savedMode === 'secret_desktop' || !savedMode) {
    modeSelect.value = 'direct';
    localStorage.setItem('desktop_cockpit_input_mode', 'direct');
  } else {
    modeSelect.value = savedMode;
  }
}

if (modeSelect) {
  modeSelect.addEventListener('change', () => {
    localStorage.setItem('desktop_cockpit_input_mode', modeSelect.value);
    const modeName = modeSelect.value === 'direct' ? '⚡ 直通模式' : '🌿 純背景模式 (PostMessage)';
    log('模式切換', `切換為 ${modeName}`, 'sys');
    updateHudStat();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'set_capture_mode', mode: modeSelect.value }));
    }
  });
}

function getMode() {
  return modeSelect ? modeSelect.value : 'direct';
}

btnRefresh.addEventListener('click', () => {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: 'get_windows' }));
    log('系統', '正在重新整理本機視窗清單...', 'sys');
  }
});

btnEngineRestart.addEventListener('click', async () => {
  log('系統', '正在重啟底層串流引擎...', 'sys');
  if (window.sidecar && window.sidecar.fetch) {
    try {
      await window.sidecar.fetch('/api/restart-engine', { method: 'POST' });
      log('系統', '已發送重啟請求', 'sys');
    } catch (e) {
      log('系統', `重啟失敗: ${e.message}`, 'sys');
    }
  }
});


// ADC 3.0 UIA Semantic Sensing, Visual Memory & AI Virtual Cursor Demo
const btnScanUIA = document.getElementById('btnScanUIA');
const btnDiff = document.getElementById('btnDiff');

if (btnTestAiCursor) {
  btnTestAiCursor.addEventListener('click', () => {
    const u = parseFloat((0.25 + Math.random() * 0.5).toFixed(3));
    const v = parseFloat((0.25 + Math.random() * 0.5).toFixed(3));
    log('AI 游標測試', `🤖 啟動虛擬 AI 游標平滑導航至 (${u}, ${v}) [主畫面實體滑鼠 0 位移]...`, 'agent');
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'mouse',
        source: 'agent',
        mode: getMode(),
        subaction: 'move',
        button: 'left',
        u, v, delta: 0
      }));
      setTimeout(() => {
        handleAiCursorEvent({ u, v, subaction: 'click', button: 'left', source: 'agent', focus_restored: true });
      }, 280);
    } else {
      handleAiCursorEvent({ u, v, subaction: 'move', button: 'left', source: 'agent' });
      setTimeout(() => {
        handleAiCursorEvent({ u, v, subaction: 'click', button: 'left', source: 'agent', focus_restored: true });
      }, 280);
    }
  });
}

if (btnScanUIA) {
  btnScanUIA.addEventListener('click', () => {
    const hwnd = parseInt(windowSelect.value, 10) || null;
    log('UIA 感知', `正在請求掃描目標視窗語意樹 (HWND: ${hwnd ? '0x' + hwnd.toString(16).toUpperCase() : '當前焦點'})...`, 'sys');
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'get_uia_tree', hwnd, depth: 4 }));
    }
  });
}

if (btnDiff) {
  btnDiff.addEventListener('click', () => {
    log('Visual Memory', '正在比對快照差異 (偵測新彈窗與狀態差異)...', 'sys');
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'get_visual_diff' }));
    }
  });
}

const annotator = initAnnotator({ canvas, log });
initStreamInput({
  canvas,
  viewport,
  getWs: () => ws,
  getMode,
  isAnnotating: () => annotator.isAnnotating(),
  log,
  setHumanActive,
  updateHudStat,
});

// Initialize connection
connectWebSocket();
