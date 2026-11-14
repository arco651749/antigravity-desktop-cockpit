// Desktop Cockpit Annotation & Visual Communication Engine
export function initAnnotator({ canvas, log }) {
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

  return {
    isAnnotating: () => isAnnotating,
    exitAnnotateMode,
  };
}
