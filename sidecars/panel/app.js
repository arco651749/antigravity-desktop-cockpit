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

// Annotation & Visual Feedback Elements
const btnAnnotateToggle = document.getElementById('btnAnnotateToggle');
const annotCanvas = document.getElementById('annotationCanvas');
const annotToolbar = document.getElementById('annotToolbar');
const annotCtx = annotCanvas ? annotCanvas.getContext('2d') : null;
const annotSendPopover = document.getElementById('annotSendPopover');
const popoverInput = document.getElementById('popoverInput');
const popoverClose = document.getElementById('popoverClose');
const popoverCancelBtn = document.getElementById('popoverCancelBtn');
const popoverSubmitBtn = document.getElementById('popoverSubmitBtn');

let isAnnotating = false;
let annotTool = 'pen'; // 'pen' | 'rect' | 'arrow'
let annotColor = '#ef4444';
let annotLineWidth = 4;
let annotHistory = [];
let isDrawingAnnot = false;
let drawStartX = 0;
let drawStartY = 0;
let strokeSnapshot = null;

// Explicitly ensure hidden on initial load
if (annotCanvas) annotCanvas.hidden = true;
if (annotToolbar) annotToolbar.hidden = true;
if (annotSendPopover) annotSendPopover.hidden = true;

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
        if (isAnnotating) {
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

// Human input interactions
function createClickRipple(e, type = 'user') {
  const ripple = document.createElement('div');
  ripple.className = `click-ripple ripple-${type}`;
  const rect = viewport.getBoundingClientRect();
  ripple.style.left = `${e.clientX - rect.left}px`;
  ripple.style.top = `${e.clientY - rect.top}px`;
  viewport.appendChild(ripple);
  setTimeout(() => ripple.remove(), 600);
}

function getNormalizedCoords(e) {
  const rect = canvas.getBoundingClientRect();
  const u = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  const v = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
  return { u: parseFloat(u.toFixed(3)), v: parseFloat(v.toFixed(3)) };
}

let downCoords = { u: 0.5, v: 0.5 };
let downClientPos = { x: 0, y: 0 };
let isPointerDown = false;
let isDragging = false;
let dragButton = 'left';
let lastMoveTime = 0;

viewport.addEventListener('pointerdown', (e) => {
  if (isAnnotating) return;
  e.preventDefault();
  viewport.focus();

  if (!windowSelect.value) {
    log('系統警告', '⚠️ 請先在上方選單選擇欲控制的目標視窗！', 'agent');
    windowSelect.focus();
    windowSelect.classList.add('pulse-highlight');
    setTimeout(() => windowSelect.classList.remove('pulse-highlight'), 1200);
    return;
  }

  try {
    viewport.setPointerCapture(e.pointerId);
  } catch (_) {}

  isPointerDown = true;
  isDragging = false;
  downClientPos = { x: e.clientX, y: e.clientY };

  const btnMap = ['left', 'middle', 'right'];
  dragButton = btnMap[e.button] || 'left';

  createClickRipple(e, 'user');

  const { u, v } = getNormalizedCoords(e);
  downCoords = { u, v };

  // 立即在畫布上渲染虛擬 AI 游標位置 (即時視覺反饋，實體滑鼠 100% 留在畫布原位)
  handleAiCursorEvent({ u, v, subaction: 'move', source: 'user' });

  const btnLabels = { left: '左鍵點擊', middle: '中鍵 (Blender 旋轉)', right: '右鍵選單' };
  setHumanActive(btnLabels[dragButton]);
});

viewport.addEventListener('pointermove', (e) => {
  if (isAnnotating) return;
  const { u, v } = getNormalizedCoords(e);
  updateHudStat(`(${u.toFixed(3)}, ${v.toFixed(3)})`);

  if (!isPointerDown) return;

  const dist = Math.hypot(e.clientX - downClientPos.x, e.clientY - downClientPos.y);

  // 當移動距離超過 4 像素時，判定為有意圖的「拖曳 (Drag)」！
  if (dist > 4) {
    const now = performance.now();

    if (!isDragging) {
      isDragging = true;
      // 拖曳開始：第一時間發送 down 事件鎖定目標拖曳起點
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({
          action: 'mouse',
          source: 'user',
          mode: getMode(),
          subaction: 'down',
          button: dragButton,
          u: downCoords.u,
          v: downCoords.v,
          delta: 0
        }));
      }
      setHumanActive(dragButton === 'middle' ? '中鍵 3D 視角旋轉' : '滑鼠拖曳中');
    }

    if (now - lastMoveTime >= 16) {
      lastMoveTime = now;
      handleAiCursorEvent({ u, v, subaction: 'move', source: 'user' });
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({
          action: 'mouse',
          source: 'user',
          mode: getMode(),
          subaction: 'move',
          button: dragButton,
          u, v, delta: 0
        }));
      }
    }
  }
});

viewport.addEventListener('pointerup', (e) => {
  if (isAnnotating) return;
  if (!isPointerDown) return;
  isPointerDown = false;

  try {
    viewport.releasePointerCapture(e.pointerId);
  } catch (_) {}

  const { u, v } = getNormalizedCoords(e);
  const mode = getMode();

  if (isDragging) {
    // 拖曳結束：發送 up 釋放按鈕
    isDragging = false;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'mouse',
        source: 'user',
        mode,
        subaction: 'up',
        button: dragButton,
        u, v, delta: 0
      }));
      handleAiCursorEvent({ u, v, subaction: 'click', source: 'user', focus_restored: mode !== 'direct' });
      const isDir = mode === 'direct';
      log('人類操作', `拖曳釋放 ${dragButton} (${u.toFixed(3)}, ${v.toFixed(3)}) ${isDir ? '→ 游標停留在第二螢幕' : '→ 滑鼠與焦點已歸還主畫面'}`, 'user');
    }
  } else {
    // 純點擊 (未拖曳超過 4px)：發送原子化 click 指令！
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'mouse',
        source: 'user',
        mode,
        subaction: 'click',
        button: dragButton,
        u: downCoords.u,
        v: downCoords.v,
        delta: 0
      }));
      handleAiCursorEvent({ u: downCoords.u, v: downCoords.v, subaction: 'click', source: 'user', focus_restored: mode !== 'direct' });
      const isDir = mode === 'direct';
      log('人類操作', `點擊 (${downCoords.u.toFixed(3)}, ${downCoords.v.toFixed(3)}) [${dragButton}] ${isDir ? '→ 游標已送至第二螢幕' : '→ 主畫面滑鼠 0 位移'}`, 'user');
    }
  }
});

viewport.addEventListener('pointercancel', (e) => {
  if (isPointerDown) {
    isPointerDown = false;
    try {
      viewport.releasePointerCapture(e.pointerId);
    } catch (_) {}
    if (isDragging) {
      isDragging = false;
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({
          action: 'mouse',
          source: 'user',
          mode: getMode(),
          subaction: 'up',
          button: dragButton,
          u: downCoords.u,
          v: downCoords.v,
          delta: 0
        }));
      }
    }
  }
});

// 全域拖曳安全釋放防護：若使用者在畫布外或快速滑動中釋放滑鼠，確保 100% 派發 UP 結束拖曳
window.addEventListener('pointerup', (e) => {
  if (isDragging) {
    isDragging = false;
    isPointerDown = false;
    const { u, v } = getNormalizedCoords(e);
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'mouse',
        source: 'user',
        mode: getMode(),
        subaction: 'up',
        button: dragButton,
        u, v, delta: 0
      }));
      log('人類操作', `全域安全拖曳釋放 ${dragButton} (${u.toFixed(3)}, ${v.toFixed(3)})`, 'user');
    }
  }
});

viewport.addEventListener('mouseleave', () => {
  if (isAnnotating) return;
  // 在直通模式下不擅自觸發焦點歸還，游標與焦點完全留給第二螢幕，使用者自已透過 80px 重疊區滑回
  if (getMode() === 'direct') return;
  if (!isPointerDown && !isDragging) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'restore_primary_focus' }));
    }
  }
});

viewport.addEventListener('wheel', (e) => {
  if (isAnnotating) return;
  e.preventDefault();
  const { u, v } = getNormalizedCoords(e);
  // 換算標準 Win32 WHEEL_DELTA (每次滾動 1 格為 120，往上為正 +120，往下為負 -120)
  const notch = Math.sign(-e.deltaY) || (e.deltaY < 0 ? 1 : -1);
  const delta = notch * 120;
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({
      action: 'mouse',
      source: 'user',
      mode: getMode(),
      subaction: 'wheel',
      button: 'left',
      u, v, delta
    }));
  }
  setHumanActive('滾輪縮放/滾動');
  log('人類操作', `滾輪滾動 delta=${delta} [${getMode()}]`, 'user');
}, { passive: false });

viewport.addEventListener('keydown', (e) => {
  if (isAnnotating) {
    if (e.key === 'Escape') {
      exitAnnotateMode();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && (e.key === 'z' || e.key === 'Z')) {
      e.preventDefault();
      undoAnnot();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && (e.key === 'c' || e.key === 'C')) {
      e.preventDefault();
      copyAnnotatedImageToClipboard();
      return;
    }
    return;
  }

  if (e.key === 'Escape') {
    viewport.blur();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ action: 'restore_primary_focus' }));
    }
    log('系統', '使用者按下 Esc，鍵盤焦點已釋放回 Antigravity 聊天室', 'sys');
    return;
  }
  e.preventDefault();

  const isCtrl = e.ctrlKey;
  const isAlt = e.altKey;
  const isShift = e.shiftKey;
  const isModifierCombo = isCtrl || isAlt;
  const char = (!isModifierCombo && e.key.length === 1) ? e.key : null;

  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({
      action: 'key',
      source: 'user',
      mode: getMode(),
      subaction: 'down',
      vk: e.keyCode,
      char: char,
      ctrl: isCtrl,
      alt: isAlt,
      shift: isShift
    }));
  }

  let label = e.key;
  if (isCtrl && e.key !== 'Control') label = `Ctrl+${e.key.toUpperCase()}`;
  if (isAlt && e.key !== 'Alt') label = `Alt+${e.key.toUpperCase()}`;
  if (isShift && isCtrl && e.key !== 'Shift' && e.key !== 'Control') label = `Ctrl+Shift+${e.key.toUpperCase()}`;

  setHumanActive(`快捷鍵 ${label}`);
  log('人類操作', `鍵盤按下: ${label} (VK: ${e.keyCode}) [${getMode()}]`, 'user');
});

viewport.addEventListener('keyup', (e) => {
  if (isAnnotating) return;
  if (e.key !== 'Escape') {
    e.preventDefault();
    const isCtrl = e.ctrlKey;
    const isAlt = e.altKey;
    const isShift = e.shiftKey;
    const isModifierCombo = isCtrl || isAlt;
    const char = (!isModifierCombo && e.key.length === 1) ? e.key : null;

    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'key',
        source: 'user',
        mode: getMode(),
        subaction: 'up',
        vk: e.keyCode,
        char: char,
        ctrl: isCtrl,
        alt: isAlt,
        shift: isShift
      }));
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

// Initialize connection
connectWebSocket();

// ==========================================================================
// Desktop Cockpit Annotation & Visual Communication Engine
// ==========================================================================

function showToast(msg) {
  const existing = document.querySelector('.cockpit-toast');
  if (existing) existing.remove();
  const toast = document.createElement('div');
  toast.className = 'cockpit-toast';
  toast.textContent = msg;
  document.body.appendChild(toast);
  setTimeout(() => {
    toast.style.transition = 'opacity 0.25s ease';
    toast.style.opacity = '0';
    setTimeout(() => toast.remove(), 250);
  }, 2300);
}

function drawArrow(ctx, fromx, fromy, tox, toy, color, lineWidth) {
  const headlen = Math.max(14, lineWidth * 3.5);
  const angle = Math.atan2(toy - fromy, tox - fromx);
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';

  ctx.beginPath();
  ctx.moveTo(fromx, fromy);
  ctx.lineTo(tox, toy);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(tox, toy);
  ctx.lineTo(tox - headlen * Math.cos(angle - Math.PI / 6), toy - headlen * Math.sin(angle - Math.PI / 6));
  ctx.lineTo(tox - headlen * Math.cos(angle + Math.PI / 6), toy - headlen * Math.sin(angle + Math.PI / 6));
  ctx.closePath();
  ctx.fill();
}

function getAnnotCoords(e) {
  if (!annotCanvas) return { x: 0, y: 0 };
  const rect = annotCanvas.getBoundingClientRect();
  const scaleX = rect.width > 0 ? (annotCanvas.width / rect.width) : 1;
  const scaleY = rect.height > 0 ? (annotCanvas.height / rect.height) : 1;
  return {
    x: (e.clientX - rect.left) * scaleX,
    y: (e.clientY - rect.top) * scaleY
  };
}

function saveAnnotHistory() {
  if (!annotCtx || !annotCanvas) return;
  try {
    const snap = annotCtx.getImageData(0, 0, annotCanvas.width, annotCanvas.height);
    annotHistory.push(snap);
    if (annotHistory.length > 20) annotHistory.shift();
  } catch (_) {}
}

function undoAnnot() {
  if (annotHistory.length > 0) {
    const last = annotHistory.pop();
    if (annotCtx && last) {
      annotCtx.putImageData(last, 0, 0);
    }
  } else if (annotCtx && annotCanvas) {
    annotCtx.clearRect(0, 0, annotCanvas.width, annotCanvas.height);
  }
}

function enterAnnotateMode() {
  if (isAnnotating) return;
  isAnnotating = true;

  if (annotCanvas) {
    annotCanvas.width = canvas.width || 800;
    annotCanvas.height = canvas.height || 600;
    annotCanvas.hidden = false;
    if (annotCtx) {
      annotCtx.clearRect(0, 0, annotCanvas.width, annotCanvas.height);
    }
  }

  // Switch bottom area: hide logs, show docked annotation toolbar
  if (consoleLogs) consoleLogs.hidden = true;
  if (annotToolbar) annotToolbar.hidden = false;

  if (btnAnnotateToggle) {
    btnAnnotateToggle.classList.add('active');
    btnAnnotateToggle.textContent = '📸 標註中...';
  }

  annotHistory = [];
  saveAnnotHistory();

  showToast('📸 畫面已定格！工具列已置於底部欄位，請自由標註');
  log('標註模式', '📸 畫面已定格！工具列已切換至底部欄位，請自由標註（按 Esc 或點擊關閉退出）', 'user');
}

function exitAnnotateMode() {
  if (!isAnnotating) return;
  isAnnotating = false;

  if (annotCanvas) {
    annotCanvas.hidden = true;
    if (annotCtx) annotCtx.clearRect(0, 0, annotCanvas.width, annotCanvas.height);
  }

  // Restore bottom area: hide annotation toolbar, restore logs
  if (annotToolbar) annotToolbar.hidden = true;
  if (consoleLogs) consoleLogs.hidden = false;

  if (btnAnnotateToggle) {
    btnAnnotateToggle.classList.remove('active');
    btnAnnotateToggle.textContent = '📸 截圖標註';
  }
  if (annotSendPopover) annotSendPopover.hidden = true;
  annotHistory = [];

  log('標註模式', '已退出標註模式，底部恢復日誌顯示，串流與滑鼠控制已恢復', 'sys');
}

if (annotCanvas) {
  annotCanvas.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    annotCanvas.setPointerCapture(e.pointerId);
    isDrawingAnnot = true;
    saveAnnotHistory();

    const { x, y } = getAnnotCoords(e);
    drawStartX = x;
    drawStartY = y;

    strokeSnapshot = annotCtx.getImageData(0, 0, annotCanvas.width, annotCanvas.height);

    if (annotTool === 'pen') {
      annotCtx.beginPath();
      annotCtx.moveTo(x, y);
    }
  });

  annotCanvas.addEventListener('pointermove', (e) => {
    if (!isDrawingAnnot) return;
    e.preventDefault();
    e.stopPropagation();
    const { x, y } = getAnnotCoords(e);

    const naturalScale = Math.max(1, annotCanvas.width / 800);
    const actualWidth = annotLineWidth * naturalScale;

    if (annotTool === 'pen') {
      annotCtx.strokeStyle = annotColor;
      annotCtx.lineWidth = actualWidth;
      annotCtx.lineCap = 'round';
      annotCtx.lineJoin = 'round';
      annotCtx.lineTo(x, y);
      annotCtx.stroke();
    } else if (annotTool === 'rect') {
      if (strokeSnapshot) annotCtx.putImageData(strokeSnapshot, 0, 0);
      annotCtx.strokeStyle = annotColor;
      annotCtx.lineWidth = actualWidth;
      annotCtx.strokeRect(drawStartX, drawStartY, x - drawStartX, y - drawStartY);
    } else if (annotTool === 'arrow') {
      if (strokeSnapshot) annotCtx.putImageData(strokeSnapshot, 0, 0);
      drawArrow(annotCtx, drawStartX, drawStartY, x, y, annotColor, actualWidth);
    }
  });

  const finishAnnotDrawing = (e) => {
    if (!isDrawingAnnot) return;
    isDrawingAnnot = false;
    try { annotCanvas.releasePointerCapture(e.pointerId); } catch (_) {}
  };

  annotCanvas.addEventListener('pointerup', finishAnnotDrawing);
  annotCanvas.addEventListener('pointercancel', finishAnnotDrawing);
}

if (btnAnnotateToggle) {
  btnAnnotateToggle.addEventListener('click', () => {
    if (isAnnotating) {
      exitAnnotateMode();
    } else {
      enterAnnotateMode();
    }
  });
}

if (annotToolbar) {
  // Tool buttons
  annotToolbar.querySelectorAll('.annot-tool-group .annot-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      annotToolbar.querySelectorAll('.annot-tool-group .annot-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      annotTool = btn.dataset.tool;
    });
  });

  // Color selection
  annotToolbar.querySelectorAll('.color-dot').forEach(dot => {
    dot.addEventListener('click', () => {
      annotToolbar.querySelectorAll('.color-dot').forEach(d => d.classList.remove('active'));
      dot.classList.add('active');
      annotColor = dot.dataset.color;
    });
  });

  // Size selection
  annotToolbar.querySelectorAll('.size-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      annotToolbar.querySelectorAll('.size-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      annotLineWidth = parseInt(btn.dataset.size, 10);
    });
  });

  // Undo button
  document.getElementById('btnAnnotUndo')?.addEventListener('click', () => {
    undoAnnot();
  });

  // Clear button
  document.getElementById('btnAnnotClear')?.addEventListener('click', () => {
    saveAnnotHistory();
    if (annotCtx && annotCanvas) {
      annotCtx.clearRect(0, 0, annotCanvas.width, annotCanvas.height);
    }
  });

  // Copy button (Ctrl+C, V)
  document.getElementById('btnAnnotCopy')?.addEventListener('click', async () => {
    await copyAnnotatedImageToClipboard();
  });

  // Ask Agent button
  document.getElementById('btnAnnotSend')?.addEventListener('click', (e) => {
    e.stopPropagation();
    openAskPopover();
  });

  // Close button
  document.getElementById('btnAnnotClose')?.addEventListener('click', () => {
    exitAnnotateMode();
  });
}

async function copyAnnotatedImageToClipboard() {
  const btn = document.getElementById('btnAnnotCopy');
  if (btn) {
    btn.disabled = true;
    btn.textContent = '⏳ 複製中...';
  }
  try {
    const naturalW = canvas.width;
    const naturalH = canvas.height;
    if (!naturalW || !naturalH) throw new Error('畫布無有效圖像');

    const merged = document.createElement('canvas');
    merged.width = naturalW;
    merged.height = naturalH;
    const mCtx = merged.getContext('2d');

    // 1. Draw base stream image
    mCtx.drawImage(canvas, 0, 0, naturalW, naturalH);
    // 2. Draw annotation layer
    if (annotCanvas) {
      mCtx.drawImage(annotCanvas, 0, 0, naturalW, naturalH);
    }

    const base64 = merged.toDataURL('image/png');
    let copied = false;

    // A. Browser Clipboard API (modern, zero overhead)
    if (navigator.clipboard && window.ClipboardItem) {
      try {
        const blobPromise = new Promise((resolve, reject) => {
          merged.toBlob(blob => blob ? resolve(blob) : reject(new Error('toBlob failed')), 'image/png');
        });
        const item = new ClipboardItem({ 'image/png': blobPromise });
        await navigator.clipboard.write([item]);
        copied = true;

        // Background save
        if (window.sidecar && window.sidecar.fetch) {
          window.sidecar.fetch('/api/save-annotated-image', {
            method: 'POST',
            body: JSON.stringify({ base64, title: windowSelect.options[windowSelect.selectedIndex]?.text || 'cockpit' })
          }).catch(() => {});
        }
      } catch (err) {
        console.warn('Browser clipboard.write error:', err);
      }
    }

    // B. Fallback to Sidecar PowerShell Clipboard API
    if (!copied && window.sidecar && window.sidecar.fetch) {
      try {
        const res = await window.sidecar.fetch('/api/copy-image-to-clipboard', {
          method: 'POST',
          body: JSON.stringify({ base64, title: windowSelect.options[windowSelect.selectedIndex]?.text || 'cockpit' })
        });
        const data = await res.json();
        if (data && (data.ok || data.success)) copied = true;
      } catch (err) {
        console.warn('Sidecar clipboard API error:', err);
      }
    }

    if (copied) {
      showToast('📋 標註圖片已複製到系統剪貼簿！可直接在輸入框 Ctrl+V 貼上');
      log('標註匯出', '標註圖片已複製至系統剪貼簿，請在對話輸入框按 Ctrl+V 貼上發送', 'user');
    } else {
      throw new Error('無法存取系統剪貼簿');
    }
  } catch (err) {
    showToast(`複製失敗: ${err.message}`);
    log('標註錯誤', `複製失敗: ${err.message}`, 'agent');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '📋 Ctrl + C, V';
    }
  }
}

async function sendAnnotatedImageToAgent(customMessage = '') {
  const btn = document.getElementById('btnAnnotSend');
  if (btn) {
    btn.disabled = true;
    btn.textContent = '⏳ 送出中...';
  }
  try {
    const naturalW = canvas.width;
    const naturalH = canvas.height;
    if (!naturalW || !naturalH) throw new Error('畫布無有效圖像');

    const merged = document.createElement('canvas');
    merged.width = naturalW;
    merged.height = naturalH;
    const mCtx = merged.getContext('2d');
    mCtx.drawImage(canvas, 0, 0, naturalW, naturalH);
    if (annotCanvas) {
      mCtx.drawImage(annotCanvas, 0, 0, naturalW, naturalH);
    }

    const base64 = merged.toDataURL('image/png');
    const winTitle = windowSelect.options[windowSelect.selectedIndex]?.text || 'Desktop Window';

    let data = null;
    if (window.sidecar && window.sidecar.fetch) {
      const res = await window.sidecar.fetch('/api/copy-image-to-clipboard', {
        method: 'POST',
        body: JSON.stringify({ base64, title: winTitle })
      });
      data = await res.json();
    }

    if (data && data.fullPath) {
      const cleanPath = data.fullPath.replace(/\\/g, '/');
      let prompt = `Here is the annotated screenshot from Desktop Cockpit (\`${winTitle}\`):\n\n![Annotated Desktop](file:///${cleanPath})\n[📁 開啟標註圖片原檔](file:///${cleanPath})\n`;
      if (customMessage) {
        prompt += `\n**使用者說明 / 提問**：\n${customMessage}\n`;
      }

      const agent = window.sidecar?.agent || (typeof sidecar !== 'undefined' ? sidecar?.agent : null);
      if (agent && agent.sendMessage) {
        await agent.sendMessage(prompt);
      }

      showToast(customMessage ? '💬 標註圖片與訊息已送至 Agent！' : '💬 標註圖片已送至 Agent！');
      log('標註溝通', `已將標註圖片傳送給 Agent: ${cleanPath}`, 'user');
      exitAnnotateMode();
      return;
    }

    showToast('📋 標註圖片已存檔並複製到剪貼簿（請在輸入框 Ctrl+V 貼上）！');
  } catch (err) {
    showToast(`發送失敗: ${err.message}`);
    log('標註錯誤', `發送失敗: ${err.message}`, 'agent');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '💬 Ask';
    }
  }
}

function openAskPopover() {
  if (!annotSendPopover) return;
  annotSendPopover.hidden = false;
  if (popoverInput) {
    popoverInput.value = '';
    setTimeout(() => popoverInput.focus(), 60);
  }
}

if (popoverClose) {
  popoverClose.addEventListener('click', () => {
    annotSendPopover.hidden = true;
  });
}
if (popoverCancelBtn) {
  popoverCancelBtn.addEventListener('click', () => {
    annotSendPopover.hidden = true;
  });
}
if (popoverSubmitBtn) {
  popoverSubmitBtn.addEventListener('click', async () => {
    const msg = popoverInput ? popoverInput.value.trim() : '';
    annotSendPopover.hidden = true;
    await sendAnnotatedImageToAgent(msg);
  });
}
if (popoverInput) {
  popoverInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      popoverSubmitBtn?.click();
    } else if (e.key === 'Escape') {
      annotSendPopover.hidden = true;
    }
  });
}

// Global hotkey: Ctrl+Shift+A to toggle annotation mode
window.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === 'a' || e.key === 'A')) {
    e.preventDefault();
    if (isAnnotating) {
      exitAnnotateMode();
    } else {
      enterAnnotateMode();
    }
  }
});
