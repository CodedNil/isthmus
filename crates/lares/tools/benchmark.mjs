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
  for (let i = 0; i < 120; i++) {
    await new Promise(resolve => setTimeout(resolve, 500));
    const state = await evaluate(`({ ready: document.body?.dataset.ready, error: document.querySelector('#error:not([hidden])')?.textContent })`);
    if (state.error) throw Error(state.error);
    if (state.ready) break;
    if (i === 119) throw Error('Scene readiness timed out: ' + await evaluate('document.body.innerText'));
  }
  await new Promise(resolve => setTimeout(resolve, 1000));
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
  result.seconds = (performance.now() - started) / 1000;
  result.size = [+width, +height];
  result.motion = process.env.LARES_BENCH_MOTION === '1';
  result.clockRunning = process.env.LARES_BENCH_CLOCK === 'running';
  result.presentedFps = result.submits / result.seconds;
  for (const field of ['gpu', 'latency']) {
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
} finally {
  await call('Page.removeScriptToEvaluateOnNewDocument', { identifier: injected.identifier });
  await call('Page.navigate', { url: 'about:blank' });
  socket.close();
}
