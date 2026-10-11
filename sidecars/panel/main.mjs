import { readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { SidecarApp, Response } from 'sidecar_sdk';

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

app.run();
