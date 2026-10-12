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
const hudStat = document.getElementById('hudStat');
const focusBadge = document.getElementById('focusBadge');
const consoleLogs = document.getElementById('consoleLogs');

let ws = null;
let currentFPS = 30;
let frameCount = 0;
let lastFpsTime = performance.now();
let displayedFps = 30;
let isDragging = false;
let dragButton = 'left';
let humanTimeout = null;
let lastMoveTime = 0;
let detectedVirtualMon = null;

function log(sender, text, type = 'sys') {
  const now = new Date();
  const time = `[${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}.${String(now.getMilliseconds()).padStart(3, '0')}]`;
  const div = document.createElement('div');
  div.className = 'log-entry';
  let cls = 'log-sys';
  if (type === 'user') cls = 'log-user';
  if (type === 'agent') cls = 'log-agent';
  div.innerHTML = `<span class="log-time">${time}</span><span class="${cls}">[${sender}] ${text}</span>`;
  consoleLogs.appendChild(div);
  consoleLogs.scrollTop = consoleLogs.scrollHeight;
}

function setHumanActive(desc) {
  arbiterStatus.className = 'status-pill status-human';
  arbiterText.textContent = `👨‍💻 人類操作中 (${desc}) - AI 佇列已暫停`;
  clearTimeout(humanTimeout);
  humanTimeout = setTimeout(() => {
    arbiterStatus.className = 'status-pill status-idle';
    arbiterText.textContent = '待機 (隨時接受人類或 AI 操作)';
  }, 1500);
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
        } else if (msg.event === 'window_dispatched') {
          if (msg.status === 'ok') {
            log('虛擬派駐', msg.message || '視窗已派駐至虛擬螢幕', 'sys');
            setVirtualBadgeActive(true, msg.data);
            if (modeSelect) {
              modeSelect.value = 'direct';
              localStorage.setItem('desktop_cockpit_input_mode', 'direct');
            }
          } else {
            log('派駐失敗', msg.message || '無法派駐視窗', 'agent');
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
            updateMonitorStatus(msg.data);
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

  // 若無先前紀錄，優先智慧預選 Notepad、Blender、KiCad 或第一個視窗
  if (!autoSelectHwnd && windows.length > 0) {
    const preferred = windows.find((w) => /notepad|記事本|blender|kicad/i.test(w.title)) || windows[0];
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
    virtualMonText.textContent = `🟢 虛擬顯示器在線 (${detectedVirtualMon.width}x${detectedVirtualMon.height})`;
    log('顯示拓撲', `檢測到虛擬螢幕: ${detectedVirtualMon.name} (${detectedVirtualMon.width}x${detectedVirtualMon.height} @ X=${detectedVirtualMon.x})`, 'sys');
  } else if (virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-idle';
    virtualMonText.textContent = '⚪ 虛擬顯示器未啟用';
  }
}

function setVirtualBadgeActive(active, monData) {
  if (active && virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-virtual-active';
    const resText = monData ? `${monData.width}x${monData.height}` : '1024x768';
    virtualMonText.textContent = `🛡️ 隔離派駐中 (${resText})`;
  } else if (detectedVirtualMon && virtualMonStatus) {
    virtualMonStatus.className = 'status-pill status-virtual';
    virtualMonText.textContent = `🟢 虛擬顯示器在線 (${detectedVirtualMon.width}x${detectedVirtualMon.height})`;
  }
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
    const modeName = modeSelect.value === 'direct' ? '⚡ 直通模式 (DXGI / SendInput)' : '🌿 背景模式 (Win32 PostMessage)';
    log('模式切換', `切換為 ${modeName}`, 'sys');
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
  return { u: parseFloat(u.toFixed(4)), v: parseFloat(v.toFixed(4)) };
}

viewport.addEventListener('mousedown', (e) => {
  e.preventDefault();
  viewport.focus();

  if (!windowSelect.value) {
    log('系統警告', '⚠️ 請先在上方選單選擇欲控制的目標視窗！', 'agent');
    windowSelect.focus();
    windowSelect.classList.add('pulse-highlight');
    setTimeout(() => windowSelect.classList.remove('pulse-highlight'), 1200);
    return;
  }

  isDragging = true;
  const btnMap = ['left', 'middle', 'right'];
  dragButton = btnMap[e.button] || 'left';

  createClickRipple(e, 'user');

  const { u, v } = getNormalizedCoords(e);
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({
      action: 'mouse',
      source: 'user',
      mode: getMode(),
      subaction: 'down',
      button: dragButton,
      u, v, delta: 0
    }));
  }

  const btnLabels = { left: '左鍵點擊', middle: '中鍵 (Blender 旋轉)', right: '右鍵選單' };
  setHumanActive(btnLabels[dragButton]);
  log('人類操作', `${btnLabels[dragButton]} 座標: (${u}, ${v}) [${getMode()}]`, 'user');
});

window.addEventListener('mouseup', (e) => {
  if (isDragging) {
    isDragging = false;
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
    }
    log('人類操作', `釋放 ${dragButton} [${getMode()}]`, 'user');
  }
});

viewport.addEventListener('mouseleave', () => {
  if (isDragging) {
    isDragging = false;
    log('人類操作', '滑鼠移出畫布，重置拖曳狀態', 'sys');
  }
});

viewport.addEventListener('mousemove', (e) => {
  const { u, v } = getNormalizedCoords(e);
  hudStat.textContent = `FPS: ${displayedFps} | 模式: ${getMode().toUpperCase()} | 座標: (${u}, ${v})`;

  if (isDragging) {
    const now = performance.now();
    if (now - lastMoveTime >= 16) {
      lastMoveTime = now;
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
      setHumanActive(dragButton === 'middle' ? '中鍵 3D 視角旋轉' : '滑鼠拖曳軌跡');
    }
  }
});

viewport.addEventListener('wheel', (e) => {
  e.preventDefault();
  const { u, v } = getNormalizedCoords(e);
  const delta = Math.round(-e.deltaY);
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
  if (e.key === 'Escape') {
    viewport.blur();
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

viewport.addEventListener('focus', () => {
  focusBadge.textContent = '🟢 畫布已鎖定鍵盤焦點 (快捷鍵穿透中，按 Esc 釋放)';
  focusBadge.style.color = '#4ade80';
});
viewport.addEventListener('blur', () => {
  focusBadge.textContent = '🔘 點擊畫布捕捉鍵盤輸入 (按 Esc 釋放焦點回到聊天室)';
  focusBadge.style.color = 'var(--text-muted)';
});

// ADC 2.0 UIA Semantic Sensing & Visual Memory
const btnScanUIA = document.getElementById('btnScanUIA');
const btnDiff = document.getElementById('btnDiff');

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
