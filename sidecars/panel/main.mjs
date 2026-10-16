import { readFileSync, existsSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve, basename, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { homedir } from 'node:os';
import { SidecarApp, Response } from 'sidecar_sdk';

const execFileAsync = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const app = new SidecarApp();

let engineProcess = null;
const ENGINE_PORT = 48280;

function startEngine() {
  const previous = engineProcess;
  engineProcess = null;
  if (previous && !previous.killed) {
    try {
      previous.kill();
    } catch {}
  }

  // Look for binary in bin/ or fallback to target/release/
  const binPath = join(HERE, 'bin', 'gui-stream-engine.exe');
  const fallbackPath = join(HERE, '../../engine/target/release/gui-stream-engine.exe');
  const targetBinary = existsSync(binPath) ? binPath : fallbackPath;

  // 舊程序被 kill 後需要一點時間釋放 48280 埠，否則新引擎會因 bind 失敗立即退出
  const delay = previous ? 500 : 0;

  setTimeout(() => {
    if (engineProcess) return; // 期間已有其他呼叫啟動了新引擎

    console.log(`[Cockpit Sidecar] 啟動 Rust 串流引擎: ${targetBinary} --port ${ENGINE_PORT}`);

    try {
      const proc = spawn(targetBinary, ['--port', String(ENGINE_PORT)], {
        stdio: ['ignore', 'inherit', 'inherit'],
        windowsHide: true,
        detached: false,
      });
      engineProcess = proc;

      proc.on('exit', (code, signal) => {
        console.log(`[Cockpit Sidecar] Rust 串流引擎退出: code=${code}, signal=${signal}`);
        // 只有「目前這個」程序退出才清除並自動重啟；被主動取代的舊程序不得影響新程序
        if (engineProcess !== proc) return;
        engineProcess = null;
        setTimeout(() => {
          if (!engineProcess) startEngine();
        }, 1000);
      });

      proc.on('error', (err) => {
        console.error('[Cockpit Sidecar] 無法啟動 Rust 引擎:', err);
      });
    } catch (err) {
      console.error('[Cockpit Sidecar] spawn 失敗:', err);
    }
  }, delay);
}

// Clean up child process when sidecar exits
process.on('exit', () => {
  if (engineProcess && !engineProcess.killed) {
    try {
      engineProcess.kill();
    } catch {}
  }
});
process.on('SIGINT', () => process.exit());
process.on('SIGTERM', () => process.exit());

// Launch engine immediately upon sidecar startup
startEngine();

// Serve pages and static files
app.page('/', () => readFileSync(join(HERE, 'index.html'), 'utf8'));
app.page('/app.js', () => readFileSync(join(HERE, 'app.js'), 'utf8'));
app.api('/styles.css', () =>
  new Response(readFileSync(join(HERE, 'styles.css'), 'utf8'), { contentType: 'text/css' }),
  'GET'
);

// Health check and control APIs
app.api('/api/restart-engine', () => {
  startEngine();
  return { status: 'ok', message: 'Engine restarted' };
});

app.api('/api/status', () => ({
  status: 'ok',
  engine_running: engineProcess !== null && !engineProcess.killed,
  port: ENGINE_PORT,
}));

function resolveWorkspaceRoot(data) {
  if (data?.root && existsSync(data.root)) return data.root;
  if (data?.workspaceUri) {
    try {
      const u = new URL(data.workspaceUri);
      let p = decodeURIComponent(u.pathname);
      if (process.platform === 'win32' && p.startsWith('/')) p = p.slice(1);
      if (existsSync(p)) return p;
    } catch {}
  }
  // 可攜式備援：環境變數 → 目前工作目錄（若不在 plugin 自身目錄內）→ 使用者家目錄
  const envDir = process.env.COCKPIT_WORKSPACE;
  if (envDir && existsSync(envDir)) return envDir;
  const cwd = resolve(process.cwd());
  const pluginDir = resolve(HERE, '..', '..');
  if (!cwd.toLowerCase().startsWith(pluginDir.toLowerCase())) return cwd;
  return homedir();
}

// 1. Save annotated image to disk (.gemini/annotated/)
app.api('/api/save-annotated-image', (data) => {
  const root = resolveWorkspaceRoot(data);
  if (!data?.base64) return { error: 'No image data provided' };

  try {
    const rawData = data.base64.replace(/^data:image\/\w+;base64,/, '');
    const buffer = Buffer.from(rawData, 'base64');

    const outDir = join(root, '.gemini', 'annotated');
    if (!existsSync(outDir)) {
      mkdirSync(outDir, { recursive: true });
    }

    const timestamp = Date.now();
    const origBase = data.title ? data.title.replace(/[^a-zA-Z0-9_\-\.]/g, '_') : 'cockpit_capture';
    const fileName = `annotated_${origBase}_${timestamp}.png`;
    const fullPath = join(outDir, fileName);

    writeFileSync(fullPath, buffer);

    return {
      success: true,
      ok: true,
      fullPath,
      name: fileName,
    };
  } catch (err) {
    return { error: String(err.message || err) };
  }
});

// 2. Copy annotated image directly to Windows OS system clipboard
app.api('/api/copy-image-to-clipboard', async (data) => {
  const root = resolveWorkspaceRoot(data);
  if (!data?.base64) return { error: 'No image data provided' };

  try {
    const rawData = data.base64.replace(/^data:image\/\w+;base64,/, '');
    const buffer = Buffer.from(rawData, 'base64');

    const outDir = join(root, '.gemini', 'annotated');
    if (!existsSync(outDir)) {
      mkdirSync(outDir, { recursive: true });
    }

    const timestamp = Date.now();
    const origBase = data.title ? data.title.replace(/[^a-zA-Z0-9_\-\.]/g, '_') : 'cockpit_capture';
    const fileName = `annotated_${origBase}_${timestamp}.png`;
    const fullPath = join(outDir, fileName);

    writeFileSync(fullPath, buffer);

    // Direct Windows OS Clipboard write via PowerShell
    // 路徑經由環境變數傳入，腳本本身為固定字串，避免路徑中的 ' $ ` 造成失敗或指令注入
    const psScript =
      "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; " +
      "$img = [System.Drawing.Image]::FromFile($env:COCKPIT_IMG_PATH); " +
      "try { [System.Windows.Forms.Clipboard]::SetImage($img) } finally { $img.Dispose() }";
    await execFileAsync(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass', '-Command', psScript],
      { env: { ...process.env, COCKPIT_IMG_PATH: fullPath }, windowsHide: true }
    );

    return {
      success: true,
      ok: true,
      fullPath,
      name: fileName,
    };
  } catch (err) {
    return { error: String(err.message || err) };
  }
});

app.run();
