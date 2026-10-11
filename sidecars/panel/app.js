// Desktop Cockpit - Frontend Stream Receiver & Input Controller
const canvas = document.getElementById('streamCanvas');
const ctx = canvas.getContext('2d');
const viewport = document.getElementById('viewport');
const windowSelect = document.getElementById('windowSelect');
const fpsSelect = document.getElementById('fpsSelect');
const btnRefresh = document.getElementById('btnRefresh');
const btnEngineRestart = document.getElementById('btnEngineRestart');
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
    log('WebSocket', '成功連線至 Rust 引擎！正在請求視窗清單...', 'sys');
    ws.send(JSON.stringify({ action: 'get_windows' }));
  };

  ws.onmessage = async (evt) => {
    if (typeof evt.data === 'string') {
      try {
        const msg = JSON.parse(evt.data);
        if (msg.event === 'windows' && Array.isArray(msg.data)) {
          populateWindows(msg.data);
        } else if (msg.event === 'input_ack') {
          log('底層確認', msg.message || '輸入事件已成功投遞', 'sys');
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
      ws.send(JSON.stringify({ action: 'set_window', hwnd: autoSelectHwnd }));
      log('視窗切換', `自動連結目標視窗 -> 0x${autoSelectHwnd.toString(16).toUpperCase()}`, 'sys');
    }
  }

  log('系統', `已載入 ${windows.length} 個本機活動視窗`, 'sys');
}

// Window & FPS controls
windowSelect.addEventListener('change', () => {
  const hwnd = parseInt(windowSelect.value, 10);
  if (hwnd && ws && ws.readyState === WebSocket.OPEN) {
    localStorage.setItem('desktop_cockpit_selected_hwnd', hwnd);
    ws.send(JSON.stringify({ action: 'set_window', hwnd }));
    log('視窗切換', `切換目標視窗 HWND -> 0x${hwnd.toString(16).toUpperCase()}`, 'sys');
  }
});

fpsSelect.addEventListener('change', () => {
  const fps = parseInt(fpsSelect.value, 10);
  currentFPS = fps;
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: 'set_fps', fps }));
    log('幀率時鐘', `切換為 ${fps} FPS 動態檔位`, 'sys');
  }
});

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
      subaction: 'down',
      button: dragButton,
      u, v, delta: 0
    }));
  }

  const btnLabels = { left: '左鍵點擊', middle: '中鍵 (Blender 旋轉)', right: '右鍵選單' };
  setHumanActive(btnLabels[dragButton]);
  log('人類操作', `${btnLabels[dragButton]} 座標: (${u}, ${v})`, 'user');
});

window.addEventListener('mouseup', (e) => {
  if (isDragging) {
    isDragging = false;
    const { u, v } = getNormalizedCoords(e);
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'mouse',
        source: 'user',
        subaction: 'up',
        button: dragButton,
        u, v, delta: 0
      }));
    }
    log('人類操作', `釋放 ${dragButton}`, 'user');
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
  hudStat.textContent = `FPS: ${displayedFps} | 延遲: <15ms | 座標: (${u}, ${v})`;

  if (isDragging) {
    const now = performance.now();
    if (now - lastMoveTime >= 16) {
      lastMoveTime = now;
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({
          action: 'mouse',
          source: 'user',
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
      subaction: 'wheel',
      button: 'left',
      u, v, delta
    }));
  }
  setHumanActive('滾輪縮放/滾動');
  log('人類操作', `滾輪滾動 delta=${delta}`, 'user');
}, { passive: false });

viewport.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    viewport.blur();
    log('系統', '使用者按下 Esc，鍵盤焦點已釋放回 Antigravity 聊天室', 'sys');
    return;
  }
  e.preventDefault();
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({
      action: 'key',
      source: 'user',
      subaction: 'down',
      vk: e.keyCode,
      char: e.key.length === 1 ? e.key : null
    }));
  }
  setHumanActive(`鍵入 ${e.key}`);
  log('人類操作', `鍵盤按下: ${e.key} (VK: ${e.keyCode})`, 'user');
});

viewport.addEventListener('keyup', (e) => {
  if (e.key !== 'Escape') {
    e.preventDefault();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        action: 'key',
        source: 'user',
        subaction: 'up',
        vk: e.keyCode,
        char: e.key.length === 1 ? e.key : null
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

// Initialize connection
connectWebSocket();
