// Owns the temporary server and browser. Always terminates the entire browser
// process group on success, error, Ctrl-C, or the 60-second deadline.
// node crates/lares/tools/run-benchmark.mjs [prefix] [assets-dir] [width] [height]
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { once } from 'node:events';

const [prefix = '/tmp/lares-bench/result', root = 'crates/lares/assets', width = '1280', height = '800'] = process.argv.slice(2);
const assets = resolve(root);
const profile = await mkdtemp(resolve(tmpdir(), 'lares-benchmark-'));
const server = createServer(async (request, response) => {
  try {
    const name = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const file = resolve(assets, '.' + (name === '/' ? '/index.html' : name));
    if (!file.startsWith(assets + sep)) { response.writeHead(403).end(); return; }
    const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.webp': 'image/webp' };
    const data = await readFile(file);
    response.writeHead(200, { 'Content-Type': mime[extname(file)] || 'application/octet-stream' }).end(data);
  } catch { response.writeHead(404).end(); }
});
let browser, benchmark;
let cleaning;
const cleanup = () => cleaning ||= (async () => {
  for (const child of [benchmark, browser]) {
    if (!child) continue;
    try { process.kill(-child.pid, 'SIGTERM'); } catch { }
  }
  await new Promise(resolve => setTimeout(resolve, 500));
  for (const child of [benchmark, browser]) {
    if (!child) continue;
    try { process.kill(-child.pid, 'SIGKILL'); } catch { }
  }
  server.closeAllConnections();
  server.close();
  await rm(profile, { recursive: true, force: true });
})();
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, async () => { await cleanup(); process.exit(130); });
const deadline = setTimeout(async () => { console.error('Benchmark deadline reached'); await cleanup(); process.exit(1); }, 60_000);
try {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  browser = spawn('chromium', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
    '--user-data-dir=' + profile, '--remote-debugging-port=0',
    '--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-angle=vulkan',
    '--disable-vulkan-surface', 'about:blank',
  ], { detached: true, stdio: ['ignore', 'ignore', 'pipe'] });
  const port = await new Promise((resolve, reject) => {
    let output = '';
    browser.stderr.on('data', chunk => {
      output += chunk;
      const match = output.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//);
      if (match) resolve(match[1]);
    });
    browser.once('error', reject);
    browser.once('exit', code => reject(Error('Browser exited: ' + code)));
  });
  benchmark = spawn(process.execPath, ['crates/lares/tools/benchmark.mjs', prefix,
    `http://127.0.0.1:${server.address().port}/`, width, height], {
    detached: true, stdio: 'inherit',
    env: { ...process.env, LARES_BENCH_PORT: port },
  });
  const [code] = await once(benchmark, 'exit');
  if (code !== 0) throw Error('Benchmark failed: ' + code);
} finally {
  clearTimeout(deadline);
  await cleanup();
  console.error('Benchmark browser and server stopped.');
}
