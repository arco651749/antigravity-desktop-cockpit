import { readFileSync, existsSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve, basename, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { SidecarApp, Response } from 'sidecar_sdk';

const execFileAsync = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const app = new SidecarApp();

let engineProcess = null;
const ENGINE_PORT = 48280;

function startEngine() {
  if (engineProcess && !engineProcess.killed) {
    try {
      engineProcess.kill();
    } catch {}
  }

  // Look for binary in bin/ or fallback to target/release/
  const binPath = join(HERE, 'bin', 'gui-stream-engine.exe');
  const fallbackPath = join(HERE, '../../engine/target/release/gui-stream-engine.exe');
  const targetBinary = existsSync(binPath) ? binPath : fallbackPath;

  console.log(`[Cockpit Sidecar] 啟動 Rust 串流引擎: ${targetBinary} --port ${ENGINE_PORT}`);

  try {
    engineProcess = spawn(targetBinary, ['--port', String(ENGINE_PORT)], {
      stdio: ['ignore', 'inherit', 'inherit'],
      windowsHide: true,
      detached: false,
    });

    engineProcess.on('exit', (code, signal) => {
      console.log(`[Cockpit Sidecar] Rust 串流引擎退出: code=${code}, signal=${signal}`);
      engineProcess = null;
      setTimeout(() => {
        if (!engineProcess) startEngine();
      }, 1000);
    });

    engineProcess.on('error', (err) => {
      console.error('[Cockpit Sidecar] 無法啟動 Rust 引擎:', err);
    });
  } catch (err) {
    console.error('[Cockpit Sidecar] spawn 失敗:', err);
  }
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
  const defaultDir = 'd:\\AntiGravity\\Agent-Study';
  if (existsSync(defaultDir)) return defaultDir;
  return process.cwd();
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
    const normalizedPath = fullPath.replace(/\\/g, '\\\\');
    const psScript = `Add-Type -AssemblyName System.Windows.Forms; $img = [System.Drawing.Image]::FromFile('${normalizedPath}'); [System.Windows.Forms.Clipboard]::SetImage($img); $img.Dispose()`;
    await execFileAsync('powershell.exe', ['-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass', '-Command', psScript]);

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
