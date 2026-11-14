// Desktop Cockpit Stream Input Handler (Mouse / Drag / Wheel / Keyboard)
export function initStreamInput({ canvas, viewport, getWs, getMode, isAnnotating, log, setHumanActive, updateHudStat }) {
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
    if (isAnnotating()) return;
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
    if (isAnnotating()) return;
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
        const ws = getWs();
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
        const ws = getWs();
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
    if (isAnnotating()) return;
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
      const ws = getWs();
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
      const ws = getWs();
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
        const ws = getWs();
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
      const ws = getWs();
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
    if (isAnnotating()) return;
    // 在直通模式下不擅自觸發焦點歸還，游標與焦點完全留給第二螢幕，使用者自已透過 80px 重疊區滑回
    if (getMode() === 'direct') return;
    if (!isPointerDown && !isDragging) {
      const ws = getWs();
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ action: 'restore_primary_focus' }));
      }
    }
  });
  
  viewport.addEventListener('wheel', (e) => {
    if (isAnnotating()) return;
    e.preventDefault();
    const { u, v } = getNormalizedCoords(e);
    // 換算標準 Win32 WHEEL_DELTA (每次滾動 1 格為 120，往上為正 +120，往下為負 -120)
    const notch = Math.sign(-e.deltaY) || (e.deltaY < 0 ? 1 : -1);
    const delta = notch * 120;
    const ws = getWs();
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
      const ws = getWs();
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
  
    const ws = getWs();
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
    if (isAnnotating()) return;
    if (e.key !== 'Escape') {
      e.preventDefault();
      const isCtrl = e.ctrlKey;
      const isAlt = e.altKey;
      const isShift = e.shiftKey;
      const isModifierCombo = isCtrl || isAlt;
      const char = (!isModifierCombo && e.key.length === 1) ? e.key : null;
  
      const ws = getWs();
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
  
}
