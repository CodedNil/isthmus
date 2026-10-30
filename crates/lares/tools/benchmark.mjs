// Node 22+, Chromium with --remote-debugging-port=9333 --enable-unsafe-webgpu
// --enable-features=Vulkan --use-angle=vulkan --disable-vulkan-surface.
// Prefer run-benchmark.mjs, which owns and cleans up its browser and server.
// node crates/lares/tools/benchmark.mjs [output-prefix] [url] [width] [height]
// Measures actual render-pass GPU timestamps, separately from submission latency.
import { writeFile } from 'node:fs/promises';
import { deflateSync } from 'node:zlib';

function png(width, height, rgba) {
  const chunk = (name, data) => {
    const type = Buffer.from(name);
    let crc = 0xffffffff;
    for (const byte of Buffer.concat([type, data])) {
      crc ^= byte;
      for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
    }
    const size = Buffer.alloc(4); size.writeUInt32BE(data.length);
    const checksum = Buffer.alloc(4); checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
    return Buffer.concat([size, type, data, checksum]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  const scanlines = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y++) rgba.copy(scanlines, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk('IHDR', header), chunk('IDAT', deflateSync(scanlines)), chunk('IEND', Buffer.alloc(0))]);
}

const [prefix = '/tmp/lares-bench/result', url = 'http://localhost:8128/', width = '1280', height = '800'] = process.argv.slice(2);
const pages = await (await fetch(`http://localhost:${process.env.LARES_BENCH_PORT || 9333}/json/list`)).json();
const socket = new WebSocket(pages.find(p => p.type === 'page').webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
let sequence = 0;
const pending = new Map();
socket.addEventListener('message', ({ data }) => {
  const message = JSON.parse(data);
  if (message.method === 'Runtime.consoleAPICalled') console.error(...message.params.args.map(arg => arg.value || arg.description));
  if (message.method === 'Runtime.exceptionThrown') console.error(JSON.stringify(message.params));
  if (message.id) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    message.error ? reject(Error(JSON.stringify(message.error))) : resolve(message.result);
  }
});
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++sequence;
  pending.set(id, { resolve, reject });
  socket.send(JSON.stringify({ id, method, params }));
});
const evaluate = async expression => {
  const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
};
await call('Page.enable');
await call('Runtime.enable');
await call('Emulation.setDeviceMetricsOverride', { width: +width, height: +height, deviceScaleFactor: 1, mobile: false });
const injected = await call('Page.addScriptToEvaluateOnNewDocument', {
  source: `
    const clockStart = performance.now();
    Date.now = () => 1782043200000 + (${process.env.LARES_BENCH_CLOCK === 'running'} ? performance.now() - clockStart : 0);
    window.bench = { gpu: [], latency: [], submits: 0, pending: 0, maxPending: 0, adapter: null, errors: [] };
    const configure = GPUCanvasContext.prototype.configure;
    GPUCanvasContext.prototype.configure = function(desc) { return configure.call(this, { ...desc, usage: (desc.usage || GPUTextureUsage.RENDER_ATTACHMENT) | GPUTextureUsage.COPY_SRC }); };
    const current = GPUCanvasContext.prototype.getCurrentTexture;
    let canvasTexture;
    GPUCanvasContext.prototype.getCurrentTexture = function() { return canvasTexture = current.call(this); };
    addEventListener('error', e => bench.errors.push(e.message));
    const request = GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice = async function(descriptor = {}) {
        bench.adapter = Object.fromEntries(['vendor', 'architecture', 'device', 'description'].map(key => [key, this.info[key]]));
        if (!this.features.has('timestamp-query')) throw Error('GPU timestamps unavailable');
        const device = await request.call(this, { ...descriptor, requiredFeatures: [...(descriptor.requiredFeatures || []), 'timestamp-query'] });
        console.info("Lares GPU", JSON.stringify(bench.adapter));
        device.lost.then(info => { bench.errors.push("GPU device lost: " + info.message); });
        const create = device.createCommandEncoder.bind(device);
        const results = new WeakMap();
        device.createCommandEncoder = function(descriptor) {
            const encoder = create(descriptor);
            const query = device.createQuerySet({ type: 'timestamp', count: 16 });
            const resolve = device.createBuffer({ size: 128, usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC });
            const read = device.createBuffer({ size: 128, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
            const begin = encoder.beginRenderPass.bind(encoder);
            let used = 0;
            encoder.beginRenderPass = desc => { const index = used++ * 2; return begin({ ...desc, timestampWrites: { querySet: query, beginningOfPassWriteIndex: index, endOfPassWriteIndex: index + 1 } }); };
            const finish = encoder.finish.bind(encoder);
            encoder.finish = desc => {
                if (used) { encoder.resolveQuerySet(query, 0, used * 2, resolve, 0); encoder.copyBufferToBuffer(resolve, 0, read, 0, used * 16); }
                const commands = finish(desc);
                if (used) results.set(commands, { query, resolve, read, used });
                else { query.destroy(); resolve.destroy(); read.destroy(); }
                return commands;
            };
            return encoder;
        };
        const submit = device.queue.submit.bind(device.queue);
        device.queue.submit = buffers => {
            const commands = [...buffers];
            let capture;
            if (bench.capture && canvasTexture) {
                bench.capture = false;
                const texture = canvasTexture;
                const stride = Math.ceil(texture.width * 4 / 256) * 256;
                const buffer = device.createBuffer({ size: stride * texture.height, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
                const encoder = create();
                encoder.copyTextureToBuffer({ texture }, { buffer, bytesPerRow: stride }, [texture.width, texture.height, 1]);
                commands.push(encoder.finish());
                capture = { buffer, stride, width: texture.width, height: texture.height, format: texture.format };
            }
            const start = performance.now();
            submit(commands);
            bench.pending++;
            bench.maxPending = Math.max(bench.maxPending, bench.pending);
            device.queue.onSubmittedWorkDone().then(() => { bench.pending--; });
            bench.submits++;
            if (capture) capture.buffer.mapAsync(GPUMapMode.READ).then(() => {
                const bytes = new Uint8Array(capture.buffer.getMappedRange());
                const rgba = new Uint8Array(capture.width * capture.height * 4);
                for (let y = 0; y < capture.height; y++) rgba.set(bytes.subarray(y * capture.stride, y * capture.stride + capture.width * 4), y * capture.width * 4);
                if (capture.format.startsWith('bgra')) for (let i = 0; i < rgba.length; i += 4) { const red = rgba[i]; rgba[i] = rgba[i + 2]; rgba[i + 2] = red; }
                let string = '';
                for (let i = 0; i < rgba.length; i += 16384) string += String.fromCharCode(...rgba.subarray(i, i + 16384));
                bench.image = { width: capture.width, height: capture.height, rgba: btoa(string) };
                capture.buffer.unmap(); capture.buffer.destroy();
            });
            for (const buffer of commands) {
                const entry = results.get(buffer);
                if (!entry) continue;
                const { query, resolve, read, used } = entry;
                read.mapAsync(GPUMapMode.READ).then(() => {
                    const times = new BigUint64Array(read.getMappedRange());
                    let total = 0;
                    for (let pass = 0; pass < used; pass++) total += Number(times[pass * 2 + 1] - times[pass * 2]) / 1e6;
                    bench.gpu.push(total);
                    bench.latency.push(performance.now() - start);
                    read.unmap(); query.destroy(); resolve.destroy(); read.destroy();
                });
            }
        };
        return device;
    };
` });
await call('Page.navigate', { url });
try {
  const startupDeadline = Date.now() + Number(process.env.LARES_BENCH_STARTUP_MS || 60_000);
  for (;;) {
    await new Promise(resolve => setTimeout(resolve, 500));
    const state = await evaluate(`({ ready: document.body?.dataset.ready, error: document.querySelector('#error:not([hidden])')?.textContent })`);
    if (state.error) throw Error(state.error);
    const errors = await evaluate("bench.errors");
    if (errors.length) throw Error(errors.join("\n"));
    if (state.ready) break;
    if (Date.now() >= startupDeadline) throw Error('Scene readiness timed out: ' + await evaluate('document.body.innerText'));
  }
  await new Promise(resolve => setTimeout(resolve, 1000));
  if (process.env.LARES_BENCH_TARGET || process.env.LARES_BENCH_ZOOM) {
    const target = (process.env.LARES_BENCH_TARGET || '0,0').split(',').map(Number);
    const zoom = Number(process.env.LARES_BENCH_ZOOM || 0);
    if (target.length !== 2 || !target.every(Number.isFinite) || !Number.isInteger(zoom) || Math.abs(zoom) > 20) throw Error('Invalid benchmark target/zoom');
    await evaluate(`(() => {
      const canvas = document.querySelector('canvas');
      const target = ${JSON.stringify(target)};
      const units = 36 * Math.tan(.325) / canvas.clientHeight;
      const x = canvas.clientWidth / 2, y = canvas.clientHeight / 2;
      const event = (type, px, py) => canvas.dispatchEvent(new PointerEvent(type, { clientX: px, clientY: py, pointerId: 1, button: 0, buttons: type === 'pointerup' ? 0 : 1, bubbles: true }));
      event('pointerdown', x, y);
      event('pointermove', x - target[0] / units, y - target[1] / units);
      event('pointerup', x - target[0] / units, y - target[1] / units);
      for (let step = 0; step < ${Math.abs(zoom)}; step++) canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: ${zoom >= 0 ? -1 : 1}, bubbles: true, cancelable: true }));
    })()`);
    await new Promise(resolve => setTimeout(resolve, 1000));
    for (let attempt = 0; attempt < 50; attempt++) {
      if (await evaluate("+(document.querySelector('canvas').getAttribute('data-samples') || 0) >= 8")) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
  await evaluate(`(async () => {
    const controls = await import('./web/lares.js');
    if (!controls.set_light_state) return;
    bench.lightUpdates = [];
    for (let step = 0; step < 20; step++) {
      const started = performance.now();
      if (!controls.set_light_state('light.lounge_left_downlights', step % 2)) throw Error('Light control entity missing');
      bench.lightUpdates.push(performance.now() - started);
    }
  })()`);
  // Control checks intentionally reset history; let that settle before
  // measuring spontaneous resets with a running clock.
  await new Promise(resolve => setTimeout(resolve, 200));
  for (let attempt = 0; attempt < 80; attempt++) {
    if (await evaluate(`+(document.querySelector('canvas').getAttribute('data-samples') || 0) >= ${+width * +height > 2500000 ? 8 : 4}`)) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (process.env.LARES_BENCH_MOTION === '1') await evaluate(`
    (() => {
      const canvas = document.querySelector('canvas');
      const x = canvas.clientWidth / 2, y = canvas.clientHeight / 2;
      const event = (type, px) => canvas.dispatchEvent(new PointerEvent(type, { clientX: px, clientY: y, pointerId: 1, button: 0, buttons: type === 'pointerup' ? 0 : 1, bubbles: true }));
      event('pointerdown', x);
      bench.motion = true;
      const started = performance.now();
      const move = () => { if (!bench.motion) return; event('pointermove', x + Math.sin((performance.now() - started) * .001) * 12); requestAnimationFrame(move); };
      requestAnimationFrame(move);
      window.stopBenchMotion = () => { bench.motion = false; event('pointermove', x); event('pointerup', x); };
    })()
  `);
  await evaluate(`
    (() => {
      const canvas = document.querySelector('canvas');
      const count = () => +(canvas.getAttribute('data-samples') || 0);
      bench.minSamples = count(); bench.lastSamples = count(); bench.sampleResets = 0;
      new MutationObserver(() => {
        const current = count();
        if (current < bench.lastSamples) bench.sampleResets++;
        bench.lastSamples = current; bench.minSamples = Math.min(bench.minSamples, current);
      }).observe(canvas, { attributes: true, attributeFilter: ['data-samples'] });
    })()
  `);
  await evaluate('bench.gpu = []; bench.latency = []; bench.submits = 0; bench.maxPending = bench.pending');
  const started = performance.now();
  await new Promise(resolve => setTimeout(resolve, +(process.env.LARES_BENCH_MS || 10000)));
  const result = await evaluate('JSON.parse(JSON.stringify(bench))');
  result.presence = await evaluate(`({
    online: document.querySelector('canvas')?.dataset.presenceConnected,
    ghosts: +(document.querySelector('canvas')?.dataset.presenceCount || 0)
  })`);
  if (process.env.LARES_BENCH_PRESENCE === '1' && (result.presence.online !== 'true' || !result.presence.ghosts)) {
    throw Error('Live world-space presence missing: ' + JSON.stringify(result.presence));
  }
  result.startup = await evaluate(`performance.getEntriesByType('mark').filter(entry => entry.name.startsWith('lares:')).map(entry => ({ stage: entry.name, milliseconds: entry.startTime }))`);
  result.seconds = (performance.now() - started) / 1000;
  result.size = [+width, +height];
  result.motion = process.env.LARES_BENCH_MOTION === '1';
  result.clockRunning = process.env.LARES_BENCH_CLOCK === 'running';
  result.presentedFps = result.submits / result.seconds;
  for (const field of ['gpu', 'latency', ...(result.lightUpdates ? ['lightUpdates'] : [])]) {
    const sorted = result[field].sort((a, b) => a - b);
    result[field + 'Ms'] = { samples: sorted.length, total: sorted.reduce((a, b) => a + b, 0), median: sorted[Math.floor(sorted.length / 2)], p95: sorted[Math.floor(sorted.length * .95)] };
    delete result[field];
  }
  if (result.clockRunning && !result.motion && (result.sampleResets || result.minSamples < (+width * +height > 2500000 ? 8 : 4))) {
    throw Error('Settled accumulation reset or failed to converge with a running clock: ' + JSON.stringify(result));
  }
  console.log(JSON.stringify(result, null, 2));
  await writeFile(prefix + '.json', JSON.stringify(result, null, 2) + '\n');
  await evaluate('window.stopBenchMotion?.()');
  await new Promise(resolve => setTimeout(resolve, 500));
  await evaluate('bench.capture = true');
  let captured;
  for (let i = 0; i < 30; i++) {
    await new Promise(resolve => setTimeout(resolve, 200));
    captured = await evaluate('bench.image');
    if (captured) break;
  }
  if (!captured) throw Error('WebGPU framebuffer capture timed out');
  await writeFile(prefix + '.png', png(captured.width, captured.height, Buffer.from(captured.rgba, 'base64')));
  if (process.env.LARES_BENCH_PRESENCE === '1') {
    const screenshot = await call('Page.captureScreenshot', { format: 'png' });
    await writeFile(prefix + '-page.png', Buffer.from(screenshot.data, 'base64'));
  }
} finally {
  await call('Page.removeScriptToEvaluateOnNewDocument', { identifier: injected.identifier });
  await call('Page.navigate', { url: 'about:blank' });
  socket.close();
}
