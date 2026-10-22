import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { stripTypeScriptTypes } from "node:module";

const mobileSource = stripTypeScriptTypes(readFileSync(new URL("../web/src/mobile.ts", import.meta.url), "utf8")).replace(/^export /gm, "");
const codecSource = stripTypeScriptTypes(readFileSync(new URL("../web/src/video-codec.ts", import.meta.url), "utf8")).replace(/^export /gm, "");
const preferenceSource = stripTypeScriptTypes(readFileSync(new URL("../web/src/preferences.ts", import.meta.url), "utf8")).replace(/^export /gm, "");
const source = mobileSource + "\n" + preferenceSource + "\n" + codecSource + "\n" + stripTypeScriptTypes(readFileSync(new URL("../web/src/client.ts", import.meta.url), "utf8"))

  .replace(/^import .* from .*;$/gm, "")
  .replace(/^export /gm, "") + '\nconst audioWorkerUrl = "./audio-worker.js"; const audioWorkletUrl = "./audio-worklet.js"; const client = createClient(clientOptions);';
const html = readFileSync(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const builtHtml = readFileSync(new URL("../web/dist/index.html", import.meta.url), "utf8");
const flush = () => new Promise((resolve) => setImmediate(resolve));

test("toolbar displays the server version without accepting malformed values", () => {
  const app = harness();
  const socket = app.sockets.at(-1);
  socket.open();
  socket.json({ type: "ready", width: 1280, height: 720, version: "0.1.2" });
  assert.equal(app.nodes["software-version"].textContent, "v0.1.2");
  socket.json({ type: "ready", width: 1280, height: 720, version: "<script>" });
  assert.equal(app.nodes["software-version"].textContent, "v0.1.2");
  socket.json({ type: "ready", width: 1280, height: 720 });
  assert.equal(app.nodes["software-version"].textContent, "v0.1.2");
});

function harness(pathname = "/", storage = new Map(), clientOptions = {}) {
  class Target {
    handlers = new Map();
    disabled = false;
    value = "";
    textContent = "";
    captures = new Set();
    attributes = {};
    style = { values: {}, setProperty(name, value) { this.values[name] = value; } };
    classList = {
      values: new Set(),
      toggle(name, enabled) { if (enabled) this.values.add(name); else this.values.delete(name); },
      contains(name) { return this.values.has(name); },
    };
    addEventListener(name, callback, options) {
      if (!this.handlers.has(name)) this.handlers.set(name, new Set());
      this.handlers.get(name).add(callback);
      options?.signal?.cleanups.push(() => this.handlers.get(name).delete(callback));
    }
    fire(name, properties = {}) {
      const event = { target: this, type: name, preventDefault() { this.defaultPrevented = true; }, stopImmediatePropagation() {}, ...properties };
      for (const callback of this.handlers.get(name) || []) callback(event);
      return event;
    }
    setAttribute(name, value) { this.attributes[name] = value; }
    appendChild(child) { (this.children ??= []).push(child); }
    replaceChildren(...children) { this.children = children; }
    getBoundingClientRect() { return { left: 10, top: 20, width: 200, height: 100 }; }
    focus() { document.activeElement = this; }
    setPointerCapture(id) { this.captures.add(id); }
    hasPointerCapture(id) { return this.captures.has(id); }
    releasePointerCapture(id) { this.captures.delete(id); this.fire("lostpointercapture", { pointerId: id }); }
    clientHeight = 100;
  }
  class AbortController {
    signal = { cleanups: [] };
    abort() { this.signal.cleanups.splice(0).forEach((cleanup) => cleanup()); }
  }
  const nodes = Object.fromEntries([...html.matchAll(/id="([^"]+)"/g)].map((match) => [match[1], new Target()]));
  const document = new Target();
  document.hidden = false;
  document.focused = true;
  document.hasFocus = () => document.focused;
  document.getElementById = (id) => {
    assert.ok(nodes[id], `Missing element ${id}`);
    return nodes[id];
  };
  document.createElement = () => new Target();
  const draws = [];
  nodes.viewport.classList.toggle("fit", true);
  nodes.viewport.requestFullscreen = async () => { document.fullscreenElement = nodes.viewport; document.fire("fullscreenchange"); };
  document.exitFullscreen = async () => { document.fullscreenElement = null; document.fire("fullscreenchange"); };
  nodes.desktop.getContext = () => ({ drawImage: (...args) => draws.push(args) });
  nodes.width.value = "1280";
  nodes.height.value = "720";
  const window = new Target();
  window.innerWidth = 1280;
  window.innerHeight = 720;
  const sockets = [];
  class WebSocket {
    static OPEN = 1;
    readyState = 0;
    bufferedAmount = 0;
    sent = [];
    constructor(url) { this.url = String(url); sockets.push(this); }
    send(data) { assert.equal(this.readyState, 1); this.sent.push(JSON.parse(data)); }
    close(code) {
      assert.ok(code === undefined || code === 1000 || (code >= 3000 && code <= 4999), "Browser-legal close code");
      this.readyState = 2;
    }
    finishClose() { this.readyState = 3; this.onclose?.(); }
    open() { this.readyState = 1; this.onopen?.(); }
    json(value) { this.onmessage?.({ data: JSON.stringify(value) }); }
    binary(data) { this.onmessage?.({ data }); }
  }
  const timers = new Map();
  const animationFrames = new Map();
  let timerId = 0;
  const decodes = [];
  const contexts = [];
  let now = 0;
  class AudioContext {
    state = "running";
    currentTime = 1;
    destination = {};
    buffers = [];
    sources = [];
    constructor(options) { this.options = options; contexts.push(this); }
    resume() { return Promise.resolve(); }
    close() { this.state = "closed"; return Promise.resolve(); }
    createBuffer(channels, frames, rate) {
      const data = Array.from({ length: channels }, () => new Float32Array(frames));
      const buffer = { data, duration: frames / rate, getChannelData: (channel) => data[channel] };
      this.buffers.push(buffer);
      return buffer;
    }
    createBufferSource() {
      const audioSource = { connect() {}, disconnect() { this.disconnected = true; },
        start(time) { this.time = time; }, stop() { this.stopped = true; } };
      this.sources.push(audioSource);
      return audioSource;
    }
  }
  window.AudioContext = AudioContext;
  const navigator = { clipboard: { readText: async () => "local text" } };
  const context = vm.createContext({ document, window, navigator, AbortController, WebSocket, clientOptions,
    URL, location: { href: `https://desktop.test${pathname}`, pathname, protocol: "https:" },
    ArrayBuffer, Uint8Array, DataView, TextEncoder, Blob, performance: { now: () => now },
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    setTimeout(callback, delay) { timers.set(++timerId, { callback, delay }); return timerId; },
    clearTimeout(id) { timers.delete(id); },
    requestAnimationFrame(callback) { animationFrames.set(++timerId, callback); return timerId; },
    cancelAnimationFrame(id) { animationFrames.delete(id); },
    createImageBitmap(blob) { return new Promise((resolve, reject) => decodes.push({ blob, resolve, reject })); },
  });
  vm.runInContext("Math.random = () => 0.5", context);
  vm.runInContext(source, context);
  const run = (code) => vm.runInContext(code, context);
  const ready = (socket = sockets.at(-1), audio = true) => {
    socket.open();
    socket.json({ type: "ready", width: 1280, height: 720, audio });
    return socket;
  };
  const fireKey = (code, properties = {}) => nodes.desktop.fire("keydown", {
    code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...properties,
  });
  const tick = () => {
    assert.equal(timers.size, 1);
    const [id, timer] = [...timers][0];
    timers.delete(id);
    timer.callback();
    return timer.delay;
  };
  const paint = () => {
    const callbacks = [...animationFrames.values()];
    animationFrames.clear();
    for (const callback of callbacks) callback();
  };
  return { nodes, document, window, sockets, ready, fireKey, run, timers, tick, decodes, draws, contexts, navigator, AudioContext, paint, animationFrames, advance: (elapsed) => { now += elapsed; } };
}

const desktopChoices = [
  { mode: "virtual", label: "虚拟桌面", available: true },
  { mode: "physical", label: "物理桌面", available: true },
];

test("React receives control snapshots without owning frame presentation", () => {
  const states = [];
  const app = harness("/", new Map(), { onControls: (state) => states.push(state) });
  app.ready();
  assert.equal(states.at(-1).ready, true);
  app.nodes.connection.fire("click");
  assert.equal(states.at(-1).ready, false);
  assert.equal(states.at(-1).wanted, false);
});

test("changing UI language refreshes text and desktop choices without reconnecting", () => {
  let prefix = "";
  const app = harness("/", new Map(), { translate: (value) => prefix + value });
  const socket = app.sockets[0];
  socket.open();
  desktopReady(socket);
  prefix = "translated:";
  app.run("client.refreshLanguage()");
  assert.equal(app.nodes.state.textContent, "translated:Connected");
  assert.equal(app.nodes["desktop-source"].children[0].textContent, "translated:虚拟桌面");
  assert.equal(app.sockets.length, 1);
  assert.equal(socket.readyState, 1);
});

test("toolbar requests delegate to the Headless UI owner", () => {
  let open = false;
  const requests = [];
  const app = harness("/", new Map(), {
    onToolsOpen: (wanted, focus) => { open = wanted; requests.push({ wanted, focus }); },
    isToolsOpen: () => open,
  });
  app.ready();
  app.fireKey("Escape", { ctrlKey: true, altKey: true, shiftKey: true });
  assert.equal(open, true);
  assert.equal(requests.at(-1).focus, false);
  app.nodes["tools-close"].fire("click");
  assert.equal(open, false);
  app.run("client.destroy()");
  app.paint();
  assert.equal(app.sockets[0].readyState, 2);
});
const physicalOutputs = [
  { name: "DP-1", description: "主显示器", width: 1920, height: 1080, x: 0, y: 0, scale: 1 },
  { name: "HDMI A&2", description: "副显示器", width: 2560, height: 1440, x: 1920, y: 0, scale: 1.5 },
];
function desktopReady(socket, mode = "virtual", extra = {}) {
  socket.json({ type: "ready", width: 1280, height: 720, audio: false,
    desktop: mode, resize: mode === "virtual", desktops: desktopChoices,
    outputs: mode === "physical" ? physicalOutputs : [], output: mode === "physical" ? "DP-1" : null, ...extra });
}

test("desktop selector is capability-gated, rejects unavailable modes and reload defaults virtual", () => {
  const app = harness();
  const socket = app.ready();
  assert.equal(app.nodes["desktop-controls"].hidden, true);
  desktopReady(socket, "virtual", { desktops: [desktopChoices[0],
    { mode: "physical", label: "物理桌面", available: false, reason: "未找到物理会话" }] });
  assert.equal(app.nodes["desktop-controls"].hidden, false);
  const physical = app.nodes["desktop-source"].children[1];
  assert.equal(physical.disabled, true);
  assert.match(physical.attributes.title, /未找到物理会话/);
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  assert.equal(app.sockets.length, 1);
  assert.equal(socket.readyState, 1);
  assert.equal(app.nodes["desktop-source"].value, "virtual");
  assert.match(app.nodes["desktop-status"].textContent, /未找到物理会话/);
  assert.equal(new URL(harness().sockets[0].url).searchParams.get("desktop"), "virtual");
});

test("desktop switch releases input, ignores old readiness and connects only after old close", async () => {
  const app = harness("/gateway/opaque/");
  const socket = app.ready();
  desktopReady(socket);
  app.fireKey("ControlLeft", { ctrlKey: true });
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  assert.equal(socket.sent.at(-1).type, "release");
  assert.equal(socket.readyState, 2);
  assert.equal(app.sockets.length, 1);
  assert.equal(app.nodes.desktop.hidden, true);
  assert.match(app.nodes["desktop-status"].textContent, /尚未确认/);
  desktopReady(socket);
  assert.equal(app.nodes["send-text"].disabled, true);
  socket.finishClose();
  assert.equal(app.sockets.length, 2);
  const physical = app.sockets[1];
  assert.equal(physical.url, "wss://desktop.test/gateway/opaque/ws?desktop=physical");
  physical.open();
  desktopReady(physical, "physical");
  assert.match(app.nodes["desktop-status"].textContent, /已连接.*物理桌面/);
  assert.equal(app.nodes["monitor-source"].value, "DP-1");
  assert.match(app.nodes["monitor-source"].attributes.title, /DP-1.*1920 × 1080.*缩放 1×.*位置 \(0, 0\)/);
  assert.match(app.nodes["desktop-status"].textContent, /DP-1.*1920 × 1080.*缩放 1×/);
  assert.match(app.nodes["monitor-source"].children[1].textContent, /2560 × 1440/);
  physical.binary(jpeg(1));
  app.decodes[0].resolve(bitmap());
  await flush();
  assert.equal(app.nodes.desktop.hidden, false);
  assert.equal(app.nodes["clipboard-get"].disabled, false);
});

test("stalled close reports failure within three seconds and ignores late callbacks", () => {
  const app = harness();
  const old = app.ready();
  desktopReady(old);
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  assert.equal(app.sockets.length, 1);
  assert.equal(app.tick(), 3000);
  assert.match(app.nodes["desktop-status"].textContent, /did not close within 3 seconds/);
  assert.equal(app.nodes["send-text"].disabled, true);
  assert.equal(app.timers.size, 0);
  app.nodes.connection.fire("click");
  assert.equal(app.sockets.length, 2);
  const current = app.sockets.at(-1);
  current.open();
  desktopReady(current, "physical");
  old.finishClose();
  old.json({type:"ready",width:800,height:600,desktop:"virtual"});
  assert.match(app.nodes["desktop-status"].textContent, /已连接.*物理桌面/);
  assert.equal(app.sockets.length, 2);
});

test("negotiated control release switches immediately without waiting for transport close", () => {
  const app = harness();
  const old = app.ready();
  desktopReady(old, "virtual", {control_release:true});
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  assert.equal(old.sent.at(-1).type, "disconnect");
  assert.equal(old.readyState, 1);
  assert.equal(app.sockets.length, 1);
  assert.equal(app.nodes["send-text"].disabled, true);
  old.json({type:"released"});
  assert.equal(old.readyState, 2);
  assert.equal(app.sockets.length, 2);
  const next = app.sockets.at(-1);
  next.open();
  desktopReady(next, "physical", {control_release:true});
  old.finishClose();
  old.json({type:"released"});
  assert.equal(app.sockets.length, 2);
  assert.match(app.nodes["desktop-status"].textContent, /已连接.*物理桌面/);
});

test("rapid source changes keep the latest target and issue a single release request", () => {
  const app = harness();
  const old = app.ready();
  desktopReady(old, "virtual", {control_release:true});
  old.json({type:"released"});
  assert.equal(app.sockets.length, 1, "unsolicited release is ignored");
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  app.nodes["desktop-source"].value = "virtual";
  app.nodes["desktop-source"].fire("change");
  assert.equal(old.sent.filter(v=>v.type==="disconnect").length, 1);
  old.json({type:"released"});
  assert.equal(app.sockets.length, 2);
  assert.equal(new URL(app.sockets.at(-1).url).searchParams.get("desktop"), "virtual");
});

test("a missing release acknowledgement is bounded and manual disconnect cancels it", () => {
  const app = harness();
  const old = app.ready();
  desktopReady(old, "virtual", {control_release:true});
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  assert.equal(app.tick(), 3000);
  old.json({type:"released"});
  assert.equal(old.readyState, 2, "timed-out release closes its abandoned transport");
  assert.equal(app.sockets.length, 1);
  assert.equal(app.timers.size, 0);
  const cancelled = harness();
  const active = cancelled.ready();
  desktopReady(active, "virtual", {control_release:true});
  cancelled.nodes["desktop-source"].value = "physical";
  cancelled.nodes["desktop-source"].fire("change");
  cancelled.nodes.connection.fire("click");
  active.json({type:"released"});
  active.finishClose();
  assert.equal(cancelled.sockets.length, 1);
  assert.equal(cancelled.timers.size, 0);
});

test("a source selection recovers from a stalled close without reusing old input", () => {
  const app = harness();
  desktopReady(app.ready());
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  app.tick();
  app.nodes["desktop-source"].value = "virtual";
  app.nodes["desktop-source"].fire("change");
  assert.equal(app.sockets.length, 2);
  assert.equal(new URL(app.sockets.at(-1).url).searchParams.get("desktop"), "virtual");
  assert.equal(app.nodes["send-text"].disabled, true);
});

test("readiness timeout followed by a stalled close retries without concurrent sockets", () => {
  const app=harness();app.sockets[0].open();
  assert.equal(app.tick(),15000);assert.equal(app.tick(),3000);
  assert.equal(app.sockets.length,1);
  assert.equal(app.timers.size,1);
  app.tick();assert.equal(app.sockets.length,2);
});

test("initialization failures preserve server reasons while automatic retries continue", () => {
  const app=harness();
  for(let attempt=0;attempt<4;attempt++){
    const socket=app.sockets.at(-1);socket.open();
    socket.json({type:"error",message:"capture permission denied"});socket.finishClose();
    if(attempt<3)app.tick();
  }
  assert.equal(app.timers.size,1);
  assert.match(app.nodes["desktop-status"].textContent,/capture permission denied/);
  assert.match(app.nodes.message.textContent,/capture permission denied/);
});

test("physical desktop cannot resize, monitor switches are encoded and virtual restores resize", async () => {
  const app = harness();
  const socket = app.ready();
  app.nodes["auto-resolution"].checked = true;
  desktopReady(socket, "physical");
  for (const id of ["width", "height", "apply-size", "auto-resolution"]) assert.equal(app.nodes[id].disabled, true);
  app.nodes.viewport.clientWidth = 1600;
  app.nodes.viewport.clientHeight = 900;
  app.window.fire("resize");
  app.nodes["auto-resolution"].fire("change");
  app.nodes.resize.fire("submit");
  assert.equal(socket.sent.filter((command) => command.type === "resize").length, 0);
  app.nodes["mode-fit"].fire("click");
  app.nodes.fullscreen.fire("click");
  await flush();
  assert.equal(app.document.fullscreenElement, app.nodes.viewport);
  app.nodes["monitor-source"].value = "HDMI A&2";
  app.nodes["monitor-source"].fire("change");
  socket.finishClose();
  const selected = app.sockets.at(-1);
  assert.equal(new URL(selected.url).searchParams.get("output"), "HDMI A&2");
  assert.equal(new URL(selected.url).searchParams.get("desktop"), "physical");
  selected.open();
  desktopReady(selected, "physical", { output: "HDMI A&2" });
  assert.match(app.nodes["monitor-source"].attributes.title, /HDMI A&2.*2560 × 1440.*缩放 1.5×.*位置 \(1920, 0\)/);
  assert.match(app.nodes["desktop-status"].textContent, /HDMI A&2.*2560 × 1440.*缩放 1.5×/);
  app.nodes["desktop-source"].value = "virtual";
  app.nodes["desktop-source"].fire("change");
  selected.finishClose();
  const virtual = app.sockets.at(-1);
  assert.equal(new URL(virtual.url).searchParams.has("output"), false);
  virtual.open();
  desktopReady(virtual);
  assert.equal(app.nodes["monitor-controls"].hidden, true);
  for (const id of ["width", "height", "apply-size", "auto-resolution"]) assert.equal(app.nodes[id].disabled, false);
});

test("failed physical connection can return virtual without enabling unconfirmed input", () => {
  const app = harness();
  const socket = app.ready();
  desktopReady(socket);
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  socket.finishClose();
  const failed = app.sockets.at(-1);
  failed.onerror();
  assert.match(app.nodes["desktop-status"].textContent, /失败/);
  assert.equal(app.nodes["send-shortcut"].disabled, true);
  app.nodes["desktop-source"].value = "virtual";
  app.nodes["desktop-source"].fire("change");
  assert.equal(app.sockets.length, 2);
  failed.finishClose();
  assert.equal(app.sockets.length, 3);
  assert.equal(new URL(app.sockets.at(-1).url).searchParams.get("desktop"), "virtual");
});

test("desktop discovery refresh stays prefix-relative, reports errors and ignores stale responses", async () => {
  const app = harness("/gateway/opaque/");
  const socket = app.ready();
  desktopReady(socket, "virtual", { desktops: [desktopChoices[0],
    { mode: "physical", label: "物理桌面", available: false, reason: "没有可用会话" }] });
  app.run("globalThis.discoveryRequests = []; globalThis.fetch = (url, options) => new Promise(resolve => discoveryRequests.push({url: String(url), options, resolve}))");
  app.nodes["refresh-desktops"].fire("click");
  const request = app.run("discoveryRequests[0]");
  assert.equal(request.url, "https://desktop.test/gateway/opaque/desktops");
  assert.equal(request.options.credentials, "same-origin");
  request.resolve({ ok: true, json: async () => ({ desktops: desktopChoices, outputs: physicalOutputs }) });
  await flush();
  assert.equal(app.nodes["desktop-source"].children[1].disabled, false);
  assert.equal(app.nodes["monitor-controls"].hidden, true);
  app.nodes["refresh-desktops"].fire("click");
  const stale = app.run("discoveryRequests[1]");
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  stale.resolve({ ok: true, json: async () => ({ desktops: [] }) });
  await flush();
  assert.equal(app.nodes["desktop-controls"].hidden, false);
  socket.finishClose();
  const physical = app.sockets.at(-1);
  physical.open();
  desktopReady(physical, "physical");
  app.nodes["refresh-desktops"].fire("click");
  app.run("discoveryRequests[2]").resolve({ ok: false, status: 503 });
  await flush();
  assert.match(app.nodes["desktop-status"].textContent, /刷新失败.*503/);
  assert.equal(app.nodes["desktop-source"].disabled, false);
});

test("desktop switching preserves manual disconnect and stale audio cannot enable replacement session", async () => {
  const app = harness();
  const socket = app.ready();
  desktopReady(socket, "virtual", { audio: true });
  let resume;
  app.AudioContext.prototype.resume = () => new Promise((resolve) => { resume = resolve; });
  app.nodes.audio.fire("click");
  app.nodes["desktop-source"].value = "physical";
  app.nodes["desktop-source"].fire("change");
  app.nodes.connection.fire("click");
  socket.finishClose();
  resume();
  await flush();
  assert.equal(app.sockets.length, 1);
  assert.equal(app.timers.size, 0);
  app.nodes.connection.fire("click");
  const physical = app.sockets.at(-1);
  physical.open();
  desktopReady(physical, "physical", { audio: false });
  assert.equal(app.nodes.audio.disabled, true);
  assert.equal(physical.sent.some((command) => command.type === "audio" && command.enabled), false);
});

test("remote cursor is shown once and legacy connections retain a local pointer", () => {
  const app = harness();
  const socket = app.ready();
  assert.equal(app.nodes.desktop.style.values.cursor, "default");
  socket.json({ type: "ready", width: 1280, height: 720, cursor: "embedded" });
  assert.equal(app.nodes.desktop.style.values.cursor, "none");
  socket.finishClose();
  assert.equal(app.nodes.desktop.style.values.cursor, "default");
  app.tick();
  app.ready();
  assert.equal(app.nodes.desktop.style.values.cursor, "default");
});

test("toolbar follows the pointer through deferred React layouts and closes only at drag start", () => {
  const changes = [];
  const app = harness("/", new Map(), { onToolsOpen: (open) => changes.push(open), isToolsOpen: () => false });
  const handle = app.nodes["tools-toggle"];
  changes.length = 0;
  handle.fire("pointerdown", { button: 0, pointerId: 1, clientX: 20, clientY: 30 });
  handle.fire("pointermove", { pointerId: 1, clientX: 500, clientY: 300 });
  assert.equal(app.nodes.tools.style.values["--tool-x"], "490px");
  assert.equal(app.nodes.tools.style.values["--tool-y"], "290px");
  app.paint();
  app.run("client.layoutTools()");
  assert.equal(app.nodes.tools.style.values["--tool-x"], "490px");
  handle.fire("pointermove", { pointerId: 1, clientX: 550, clientY: 320 });
  app.paint();
  app.window.fire("resize");
  assert.equal(app.nodes.tools.style.values["--tool-x"], "540px");
  assert.equal(app.nodes.tools.style.values["--tool-y"], "310px");
  assert.deepEqual(changes, [false]);
  handle.fire("pointerup", { pointerId: 1, clientX: 550, clientY: 320 });
  app.paint();
  const docked = app.nodes.tools.style.values["--tool-x"];
  app.run("client.layoutTools()");
  assert.equal(app.nodes.tools.style.values["--tool-x"], docked);
  assert.equal(handle.hasPointerCapture(1), false);
});

test("toolbar drags to all four edges, suppresses drag click and restores persisted position", () => {
  const storage = new Map();
  const app = harness("/", storage);
  const handle = app.nodes["tools-toggle"];
  for (const [edge, clientX, clientY] of [["left", 0, 350], ["right", 1270, 350], ["top", 600, 0], ["bottom", 600, 710]]) {
    handle.fire("pointerdown", { button: 0, pointerId: 1, clientX: 20, clientY: 30 });
    handle.fire("pointermove", { pointerId: 1, clientX, clientY });
    handle.fire("pointerup", { pointerId: 1, clientX, clientY });
    handle.fire("click", { detail: 1 });
    assert.equal(app.nodes["tool-panel"].hidden, true);
    assert.equal(handle.hasPointerCapture(1), false);
    assert.equal(JSON.parse(storage.get("lightos-desktop-toolbar-position")).edge, edge);
  }
  const restored = harness("/", storage);
  assert.equal(restored.nodes.tools.style.values["--tool-y"], "660px");
  handle.fire("keydown", { altKey: true, code: "ArrowLeft" });
  handle.fire("click", { detail: 0 });
  assert.equal(app.nodes["tool-panel"].hidden, false);
  assert.equal(app.nodes.tools.style.values["--tool-x"], "12px");
  assert.equal(app.nodes.tools.style.values["--panel-x"], "60px");
  app.window.innerWidth = 360;
  app.window.innerHeight = 240;
  app.window.fire("resize");
  assert.equal(app.nodes.tools.style.values["--tool-y"], "96px");
  assert.ok(parseFloat(app.nodes.tools.style.values["--panel-y"]) >= 12);
});

test("toolbar tap opens without moving, cancelled drag restores location and releases capture", () => {
  const app = harness();
  const handle = app.nodes["tools-toggle"];
  const original = app.nodes.tools.style.values["--tool-x"];
  handle.fire("pointerdown", { button: 0, pointerId: 1, clientX: 20, clientY: 30 });
  handle.fire("pointermove", { pointerId: 1, clientX: 22, clientY: 31 });
  handle.fire("pointerup", { pointerId: 1, clientX: 22, clientY: 31 });
  handle.fire("click", { detail: 1 });
  assert.equal(app.nodes["tool-panel"].hidden, false);
  handle.fire("pointerdown", { button: 0, pointerId: 2, clientX: 20, clientY: 30 });
  handle.fire("pointermove", { pointerId: 2, clientX: 600, clientY: 300 });
  handle.fire("pointercancel", { pointerId: 2 });
  assert.equal(app.nodes.tools.style.values["--tool-x"], original);
  assert.equal(handle.hasPointerCapture(2), false);
});

test("fullscreen requests all-key capture, emergency escape unlocks and disables recapture", async () => {
  const app = harness();
  const requests = [];
  let unlocks = 0;
  app.navigator.keyboard = { lock: async (...args) => requests.push(args), unlock: () => { unlocks += 1; } };
  app.ready();
  app.nodes.fullscreen.fire("click");
  await flush();
  assert.deepEqual(requests, [[]]);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  app.fireKey("Escape", { ctrlKey: true, altKey: true, shiftKey: true });
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  assert.equal(app.nodes["tool-panel"].hidden, false);
  app.nodes.desktop.focus();
  app.nodes.desktop.fire("focus");
  await flush();
  assert.equal(requests.length, 1);
  assert.ok(unlocks >= 1);
});

test("manual fullscreen changes capture automatically, including a fullscreen page ancestor", async () => {
  const app = harness();
  let requests = 0;
  app.navigator.keyboard = { lock: async () => { requests += 1; }, unlock() {} };
  app.ready();
  app.nodes.desktop.fire("pointerdown", { button: 0, pointerId: 1, clientX: 30, clientY: 30 });
  await flush();
  assert.equal(app.document.fullscreenElement, undefined, "windowed desktop must stay windowed");
  assert.equal(requests, 0);
  app.document.fullscreenElement = { contains: (node) => node === app.nodes.desktop };
  app.document.fire("fullscreenchange");
  await flush();
  assert.equal(requests, 1);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  app.fireKey("Escape", { ctrlKey: true, altKey: true, shiftKey: true });
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  await app.document.exitFullscreen();
  await app.nodes.viewport.requestFullscreen();
  await flush();
  assert.equal(requests, 2, "a fresh manual fullscreen entry restores automatic capture");
  await app.document.exitFullscreen();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
});

test("clicking a focused fullscreen desktop retries a denied capture request", async () => {
  const app = harness();
  let requests = 0;
  app.navigator.keyboard = { lock: async () => { if (++requests === 1) throw Error("NotAllowedError"); }, unlock() {} };
  app.ready();
  await app.nodes.viewport.requestFullscreen();
  await flush();
  assert.notEqual(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  app.nodes.desktop.fire("pointerdown", { button: 0, pointerId: 1, clientX: 30, clientY: 30 });
  await flush();
  assert.equal(requests, 2);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
});

test("late keyboard permission cannot capture after disconnect, denial and unsupported remain usable", async () => {
  const app = harness();
  let resolveLock;
  let unlocks = 0;
  app.navigator.keyboard = { lock: () => new Promise((resolve) => { resolveLock = resolve; }), unlock: () => { unlocks += 1; } };
  app.ready();
  app.nodes.fullscreen.fire("click");
  await flush();
  app.nodes.connection.fire("click");
  resolveLock();
  await flush();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  assert.ok(unlocks >= 2);
  const denied = harness();
  denied.navigator.keyboard = { lock: async () => { throw Error("NotAllowedError"); }, unlock() {} };
  denied.ready();
  denied.nodes.fullscreen.fire("click");
  await flush();
  assert.match(denied.nodes["keyboard-state"].textContent, /被拒绝/);
  const unsupported = harness();
  unsupported.ready();
  unsupported.nodes.fullscreen.fire("click");
  await flush();
  assert.match(unsupported.nodes["keyboard-state"].textContent, /不支持/);
});

test("captured Super+Enter and Ctrl+V are sent as remote chords and released on focus loss", async () => {
  const app = harness();
  app.navigator.keyboard = { lock: async () => {}, unlock() {} };
  const socket = app.ready();
  app.nodes["keyboard-capture"].fire("click");
  await flush();
  socket.sent.length = 0;
  const enter = app.fireKey("Enter", { metaKey: true });
  assert.equal(enter.defaultPrevented, true);
  assert.deepEqual(socket.sent, [
    { type: "key", code: 125, pressed: true },
    { type: "key", code: 28, pressed: true },
  ]);
  app.window.fire("keyup", { code: "Enter", metaKey: true });
  app.window.fire("keyup", { code: "MetaLeft" });
  socket.sent.length = 0;
  const paste = app.fireKey("KeyV", { ctrlKey: true });
  assert.equal(paste.defaultPrevented, true);
  assert.deepEqual(socket.sent, [
    { type: "key", code: 29, pressed: true },
    { type: "key", code: 47, pressed: true },
  ]);
  app.nodes.desktop.fire("blur");
  assert.equal(socket.sent.at(-1).type, "release");
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  app.nodes.desktop.focus();
  const localPaste = app.fireKey("KeyV", { ctrlKey: true });
  assert.equal(localPaste.defaultPrevented, undefined);
});

test("keyboard capture resumes after pending permission loses focus and after automatic reconnect", async () => {
  const app = harness();
  let resolveLock;
  let requests = 0;
  app.navigator.keyboard = {
    lock: () => ++requests === 1 ? new Promise((resolve) => { resolveLock = resolve; }) : Promise.resolve(),
    unlock() {},
  };
  const socket = app.ready();
  app.nodes.fullscreen.fire("click");
  await flush();
  app.nodes.desktop.fire("blur");
  app.nodes.desktop.focus();
  app.nodes.desktop.fire("focus");
  resolveLock();
  await flush();
  assert.equal(requests, 2);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  socket.finishClose();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  app.tick();
  app.ready();
  await flush();
  assert.equal(requests, 3);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
});

test("pending keyboard permission does not recapture while window remains unfocused", async () => {
  const app = harness();
  let resolveLock;
  let requests = 0;
  app.navigator.keyboard = {
    lock: () => ++requests === 1 ? new Promise((resolve) => { resolveLock = resolve; }) : Promise.resolve(),
    unlock() {},
  };
  app.ready();
  app.nodes.fullscreen.fire("click");
  await flush();
  app.document.focused = false;
  app.window.fire("blur");
  resolveLock();
  await flush();
  assert.equal(requests, 1);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  app.document.focused = true;
  app.window.fire("focus");
  await flush();
  assert.equal(requests, 2);
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
});

test("capture releases on focus loss and fullscreen exit; composer sends arbitrary chord in reverse release order", async () => {
  const app = harness();
  app.navigator.keyboard = { lock: async () => {}, unlock() {} };
  const socket = app.ready();
  app.nodes["keyboard-capture"].fire("click");
  await flush();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  app.nodes.desktop.fire("blur");
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  app.nodes.desktop.focus();
  app.nodes.desktop.fire("focus");
  await flush();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "true");
  await app.document.exitFullscreen();
  assert.equal(app.nodes["keyboard-capture"].attributes["aria-pressed"], "false");
  app.nodes["shortcut-super"].checked = true;
  app.nodes["shortcut-alt"].checked = true;
  app.nodes["shortcut-key"].value = "F7";
  socket.sent.length = 0;
  app.nodes["send-shortcut"].fire("click");
  assert.deepEqual(socket.sent, [
    { type: "release" },
    { type: "key", code: 56, pressed: true }, { type: "key", code: 125, pressed: true }, { type: "key", code: 65, pressed: true },
    { type: "key", code: 65, pressed: false }, { type: "key", code: 125, pressed: false }, { type: "key", code: 56, pressed: false },
  ]);
  assert.equal(app.nodes["shortcut-key"].children.length, Object.keys(app.run("KEY_CODES")).length);
  socket.sent.length = 0;
  app.nodes["shortcut-key"].value = "invalid";
  app.nodes["send-shortcut"].fire("click");
  assert.equal(socket.sent.length, 0);
});

test("toolbar counts drawn frames and matched monotonic RTT, clears metrics on disconnect", async () => {
  const app = harness();
  const socket = app.sockets[0];
  socket.open();
  socket.json({ type: "ready", width: 1280, height: 720, metrics: true });
  const ping = socket.sent.find((value) => value.type === "ping");
  app.advance(80);
  socket.json({ type: "pong", id: ping.id + 1 });
  socket.json({ type: "pong", id: ping.id });
  socket.binary(new Uint8Array([1, 0xff, 0xd8]).buffer);
  app.decodes[0].resolve(bitmap());
  await flush();
  app.advance(920);
  const timer = [...app.timers].find(([, value]) => value.delay === 1000);
  app.timers.delete(timer[0]);
  timer[1].callback();
  assert.equal(app.nodes.performance.textContent, "1.0 FPS · RTT 80 ms · P95 80 ms");
  assert.equal(app.nodes["video-details"].textContent, "JPEG · 1280 × 720");
  app.advance(5000);
  const next = [...app.timers].find(([, value]) => value.delay === 1000);
  app.timers.delete(next[0]);
  next[1].callback();
  assert.match(app.nodes.performance.textContent, /0.0 FPS · RTT 超时/);
  app.nodes.connection.fire("click");
  assert.equal(app.nodes.performance.textContent, "-- FPS · RTT -- ms");
  assert.equal(app.timers.size, 0);
});

test("floating tools start collapsed, close on desktop click and remain keyboard accessible", () => {
  const app = harness();
  app.ready();
  assert.equal(app.nodes["tool-panel"].hidden, true);
  app.nodes["tools-toggle"].fire("click");
  assert.equal(app.nodes["tool-panel"].hidden, false);
  assert.equal(app.nodes["tools-toggle"].attributes["aria-expanded"], "true");
  app.nodes.desktop.fire("pointerdown", { button: 0, pointerId: 1, clientX: 50, clientY: 50 });
  assert.equal(app.nodes["tool-panel"].hidden, true);
  app.fireKey("Escape", { ctrlKey: true, altKey: true, shiftKey: true });
  assert.equal(app.nodes["tool-panel"].hidden, false);
  app.nodes["tools-close"].fire("click");
  assert.equal(app.nodes["tool-panel"].hidden, true);
  assert.equal(app.document.activeElement, app.nodes.desktop);
});

test("pointer bursts coalesce and the latest position precedes a click", () => {
  const app = harness();
  const socket = app.ready();
  for (let index = 0; index < 100; index++) app.nodes.desktop.fire("pointermove", { clientX: 10 + index, clientY: 45 });
  assert.equal(socket.sent.length, 0);
  assert.equal(app.animationFrames.size, 1);
  app.paint();
  assert.equal(socket.sent.length, 1);
  app.nodes.desktop.fire("pointermove", { clientX: 40, clientY: 40 });
  app.nodes.desktop.fire("pointerdown", { button: 0, pointerId: 1, clientX: 110, clientY: 70 });
  assert.deepEqual(socket.sent.slice(-2), [{ type: "move", x: 0.5, y: 0.5 }, { type: "button", button: 0, pressed: true }]);
  assert.equal(app.animationFrames.size, 0);
  app.nodes.desktop.fire("pointermove", { clientX: 160, clientY: 70 });
  app.nodes.desktop.fire("pointerup", { button: 0, pointerId: 1 });
  assert.equal(socket.sent.at(-2).type, "move");
  assert.equal(socket.sent.at(-1).pressed, false);
});

test("video credit returns after decode, never before, and is session-scoped", async () => {
  const app = harness();
  const socket = app.ready();
  socket.json({ type: "ready", width: 1280, height: 720, audio: false, frame_ack: true });
  assert.equal(socket.sent.at(-1).type, "video_flow");
  socket.binary(jpeg(1));
  assert.equal(socket.sent.filter((command) => command.type === "frame_ack").length, 0);
  app.decodes[0].resolve(bitmap());
  await flush();
  assert.equal(socket.sent.at(-1).type, "frame_ack");
  socket.binary(jpeg(2));
  socket.finishClose();
  app.decodes[1].resolve(bitmap());
  await flush();
  assert.equal(socket.sent.filter((command) => command.type === "frame_ack").length, 1);
});

test("display mode icons switch fit, 1:1 and fullscreen, restoring the prior window mode", async () => {
  const app = harness();
  const socket = app.ready();
  app.nodes["mode-actual"].fire("click");
  await flush();
  assert.equal(app.nodes.viewport.classList.contains("fit"), false);
  assert.equal(app.nodes["mode-actual"].attributes["aria-pressed"], "true");
  app.nodes.fullscreen.fire("click");
  await flush();
  assert.equal(app.document.fullscreenElement, app.nodes.viewport);
  assert.equal(app.nodes.viewport.classList.contains("fit"), true);
  assert.equal(app.nodes.fullscreen.attributes["aria-pressed"], "true");
  await app.document.exitFullscreen();
  assert.equal(app.nodes.viewport.classList.contains("fit"), false);
  app.nodes["mode-fit"].fire("click");
  await flush();
  assert.equal(app.nodes.viewport.classList.contains("fit"), true);
  assert.equal(app.nodes["mode-fit"].attributes["aria-pressed"], "true");
  assert.equal(socket.sent.filter((command) => command.type === "resize").length, 0);
  app.nodes.viewport.requestFullscreen = async () => { throw new Error("denied"); };
  app.nodes.fullscreen.fire("click");
  await flush();
  assert.equal(app.nodes["mode-fit"].attributes["aria-pressed"], "true");
  assert.match(app.nodes.message.textContent, /denied/);
});

test("fullscreen temporarily follows browser resolution after actual-size mode, including Escape exit", async () => {
  const app = harness();
  const socket = app.ready();
  app.nodes["auto-resolution"].checked = true;
  app.nodes["mode-actual"].fire("click");
  await flush();
  app.nodes.viewport.clientWidth = 1920;
  app.nodes.viewport.clientHeight = 1080;
  app.nodes.fullscreen.fire("click");
  await flush();
  assert.equal(app.nodes["auto-resolution"].checked, true);
  app.tick();
  assert.deepEqual(socket.sent.at(-1), { type: "resize", width: 1920, height: 1080 });
  await app.document.exitFullscreen();
  assert.equal(app.nodes["auto-resolution"].checked, false);
  assert.equal(app.nodes.viewport.classList.contains("fit"), false);
  assert.equal(app.timers.size, 0);
});

test("H.264 waits for codec setup, acknowledges rendered frames and closes stale decoders", async () => {
  const app = harness();
  const decoders = [];
  app.window.VideoDecoder = class {
    state = "unconfigured";
    decodeQueueSize = 0;
    chunks = [];
    static async isConfigSupported() { return { supported: true }; }
    constructor(callbacks) { this.callbacks = callbacks; decoders.push(this); }
    configure(config) { this.config = config; this.state = "configured"; }
    decode(chunk) { this.chunks.push(chunk); }
    close() { this.state = "closed"; }
  };
  app.window.EncodedVideoChunk = class { constructor(value) { Object.assign(this, value); } };
  const socket = app.ready();
  socket.json({ type: "ready", width: 1280, height: 720, audio: false, frame_ack: true, frame_ids: true, video_codecs: ["h264", "jpeg"] });
  assert.equal(socket.sent.at(-1).codec, "h264");
  socket.json({ type: "video_config", codec: "avc1.42c01f", width: 1280, height: 720 });
  socket.json({ type: "frame_id", id: 22 });
  const packet = new Uint8Array([3, 1, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x65]).buffer;
  socket.binary(packet);
  await flush();
  assert.equal(decoders[0].chunks.length, 1);
  assert.equal(decoders[0].chunks[0].timestamp, 42);
  const frame = { displayWidth: 1280, displayHeight: 720, timestamp: 42, close() { this.closed = true; } };
  decoders[0].callbacks.output(frame);
  assert.equal(frame.closed, true);
  assert.equal(socket.sent.at(-1).type, "frame_ack");
  assert.equal(socket.sent.at(-1).id, 22);
  socket.json({ type: "video_config", codec: "avc1.42c01f", width: 800, height: 600 });
  await flush();
  assert.equal(decoders[0].state, "closed");
  const acknowledgements = socket.sent.length;
  decoders[0].callbacks.output(frame);
  assert.equal(socket.sent.length, acknowledgements);
  socket.binary(packet);
  app.tick();
  assert.equal(socket.sent.at(-1).codec, "jpeg");
  assert.equal(decoders[1].state, "closed");
});

test("pipelined H.264 shares one watchdog until all rendered frames are acknowledged", async () => {
  const app = harness();
  let decoder;
  app.window.VideoDecoder = class {
    state = "unconfigured";
    decodeQueueSize = 0;
    chunks = [];
    static async isConfigSupported() { return { supported: true }; }
    constructor(callbacks) { this.callbacks = callbacks; decoder = this; }
    configure() { this.state = "configured"; }
    decode(chunk) { this.chunks.push(chunk); }
    close() { this.state = "closed"; }
  };
  app.window.EncodedVideoChunk = class { constructor(value) { Object.assign(this, value); } };
  const socket = app.ready();
  socket.json({ type: "ready", width: 1280, height: 720, audio: false, frame_ack: true, frame_ids: true, video_codecs: ["h264", "jpeg"] });
  socket.json({ type: "video_config", codec: "avc1.42c01f", width: 1280, height: 720 });
  await flush();
  for (let id = 1; id <= 3; id++) {
    socket.json({ type: "frame_id", id });
    socket.binary(new Uint8Array([3, id === 1 ? 1 : 0, id, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x65]).buffer);
  }
  const watchdogs = () => [...app.timers.values()].filter((timer) => timer.delay === 2000);
  assert.equal(decoder.chunks.length, 3);
  assert.equal(watchdogs().length, 1);
  for (let id = 1; id <= 3; id++) {
    decoder.callbacks.output({ displayWidth: 1280, displayHeight: 720, timestamp: id, close() {} });
    assert.equal(socket.sent.at(-1).id, id);
    assert.equal(watchdogs().length, id === 3 ? 0 : 1);
  }
  assert.equal(decoder.state, "configured");
});

test("automatic resolution follows viewport CSS pixels with debounce, even dimensions and manual opt-out", () => {
  const app = harness();
  const socket = app.ready();
  app.nodes.viewport.clientWidth = 1919;
  app.nodes.viewport.clientHeight = 1079;
  app.nodes["auto-resolution"].checked = true;
  for (let count = 0; count < 10; count++) app.window.fire("resize");
  assert.equal(app.timers.size, 1);
  assert.equal(app.tick(), 250);
  assert.deepEqual(socket.sent.at(-1), { type: "resize", width: 1918, height: 1078 });
  app.window.fire("resize");
  app.tick();
  assert.equal(socket.sent.filter((command) => command.type === "resize").length, 1);
  app.nodes["mode-actual"].fire("click");
  assert.equal(app.nodes["auto-resolution"].checked, false);
  app.window.fire("resize");
  assert.equal(app.timers.size, 0);
  app.nodes["auto-resolution"].checked = true;
  app.window.fire("resize");
  app.nodes.connection.fire("click");
  assert.equal(app.timers.size, 0);
});

for (const acknowledgeInitialResize of [false, true]) {
  test(`fullscreen restores resolution after a manual downsize (${acknowledgeInitialResize ? "completed" : "pending"} earlier request)`, async () => {
    const app = harness();
    const socket = app.ready();
    app.nodes.viewport.clientWidth = 640;
    app.nodes.viewport.clientHeight = 360;
    app.nodes["auto-resolution"].checked = true;
    app.window.fire("resize");
    app.tick();
    if (acknowledgeInitialResize) {
      socket.binary(jpeg(1));
      app.decodes.at(-1).resolve(bitmap(640, 360));
      await flush();
    }
    app.nodes.width.value = "320";
    app.nodes.height.value = "200";
    app.nodes.resize.fire("submit");
    socket.binary(jpeg(2));
    app.decodes.at(-1).resolve(bitmap(320, 200));
    await flush();
    assert.equal(app.nodes.desktop.width, 320);
    const beforeFullscreen = socket.sent.length;
    app.nodes.fullscreen.fire("click");
    await flush();
    app.tick();
    assert.deepEqual(socket.sent.slice(beforeFullscreen).filter((command) => command.type === "resize"), [
      { type: "resize", width: 640, height: 360 },
    ]);
  });
}

function jpeg(marker) { return new Uint8Array([1, marker]).buffer; }
function bitmap(width = 1280, height = 720) { return { width, height, closed: false, close() { this.closed = true; } }; }
function pcm(frames = 1) {
  const data = new ArrayBuffer(1 + frames * 8);
  const view = new DataView(data);
  view.setUint8(0, 2);
  for (let index = 0; index < frames; index += 1) {
    view.setFloat32(1 + index * 8, 0.25, true);
    view.setFloat32(5 + index * 8, -0.5, true);
  }
  return data;
}

test("same-origin socket, readiness gates input, evdev mappings have no XKB offset", () => {
  const app = harness();
  assert.equal(app.sockets[0].url, "wss://desktop.test/ws?desktop=virtual");
  app.fireKey("KeyA");
  assert.deepEqual(app.sockets[0].sent, []);
  const socket = app.ready();
  const expected = { ControlLeft: 29, ControlRight: 97, ShiftLeft: 42, ShiftRight: 54,
    AltLeft: 56, AltRight: 100, MetaLeft: 125, MetaRight: 126,
    ArrowUp: 103, ArrowDown: 108, ArrowLeft: 105, ArrowRight: 106, NumpadEnter: 96 };
  for (const [code, value] of Object.entries(expected)) assert.equal(app.run(`KEY_CODES[${JSON.stringify(code)}]`), value);
  app.fireKey("ArrowLeft");
  app.fireKey("ArrowLeft", { repeat: true });
  app.fireKey("ArrowLeft");
  app.window.fire("keyup", { code: "ArrowLeft" });
  app.fireKey("toString");
  assert.deepEqual(socket.sent, [
    { type: "key", code: 105, pressed: true }, { type: "key", code: 105, pressed: false },
  ]);
});

test("nested gateway prefix is preserved for WebSocket and asset URLs", () => {
  const pathname = "/remote-desktop/test-opaque-path/";
  const app = harness(pathname);
  assert.equal(app.sockets[0].url, "wss://desktop.test/remote-desktop/test-opaque-path/ws?desktop=virtual");
  assert.match(builtHtml, /href="\.\/style\.css"/);
  assert.match(builtHtml, /src="\.\/app\.js"/);
  for (const asset of ["style.css", "app.js"]) {
    assert.equal(new URL(asset, `https://desktop.test${pathname}`).pathname, `${pathname}${asset}`);
  }
  app.sockets[0].finishClose();
  app.tick();
  assert.equal(app.sockets[1].url, "wss://desktop.test/remote-desktop/test-opaque-path/ws?desktop=virtual");
});

test("modifiers pressed before focus are synchronized and right modifiers stay distinct", () => {
  const app = harness();
  const socket = app.ready();
  app.fireKey("KeyA", { ctrlKey: true });
  app.fireKey("KeyB");
  app.fireKey("ControlRight", { ctrlKey: true });
  app.fireKey("KeyC", { ctrlKey: true });
  assert.deepEqual(socket.sent.filter((command) => command.code === 29 || command.code === 97), [
    { type: "key", code: 29, pressed: true }, { type: "key", code: 29, pressed: false },
    { type: "key", code: 97, pressed: true },
  ]);
  app.window.fire("blur");
  assert.equal(socket.sent.at(-1).type, "release");
});

test("pointer normalization, buttons, wheel units and capture cleanup", () => {
  const app = harness();
  const socket = app.ready();
  app.nodes.desktop.fire("pointerdown", { button: 2, pointerId: 7, clientX: 110, clientY: 45 });
  assert.deepEqual(socket.sent.slice(-2), [{ type: "move", x: 0.5, y: 0.25 }, { type: "button", button: 2, pressed: true }]);
  app.nodes.desktop.fire("pointermove", { clientX: 900, clientY: -10 });
  app.paint();
  assert.deepEqual(socket.sent.at(-1), { type: "move", x: 1, y: 0 });
  app.nodes.desktop.fire("wheel", { deltaX: 2, deltaY: -3, deltaMode: 1 });
  assert.deepEqual(socket.sent.at(-1), { type: "scroll", x: 32, y: -48 });
  app.nodes.desktop.fire("blur");
  assert.equal(app.nodes.desktop.captures.size, 0);
  assert.equal(socket.sent.at(-1).type, "release");
});

test("normal pointer release keeps held modifiers; keyboard escape returns focus", () => {
  const app = harness();
  const socket = app.ready();
  app.fireKey("ShiftLeft", { shiftKey: true });
  app.nodes.desktop.fire("pointerdown", { button: 0, pointerId: 7, clientX: 10, clientY: 20, shiftKey: true });
  app.nodes.desktop.fire("pointerup", { button: 0, pointerId: 7 });
  assert.deepEqual(socket.sent.at(-1), { type: "button", button: 0, pressed: false });
  assert.equal(socket.sent.some((command) => command.type === "release"), false);
  app.fireKey("Escape", { ctrlKey: true, altKey: true, shiftKey: true });
  assert.equal(socket.sent.at(-1).type, "release");
  assert.equal(app.document.activeElement, app.nodes.connection);
});

test("missing ready message times out without opening a concurrent socket", () => {
  const app = harness();
  app.sockets[0].open();
  assert.equal(app.tick(), 15000);
  assert.equal(app.sockets[0].readyState, 2);
  assert.equal(app.sockets.length, 1);
  assert.equal(app.timers.size, 1);
  app.sockets[0].finishClose();
  assert.equal(app.tick(), 500);
  assert.equal(app.sockets.length, 2);
});

test("composition and explicit Unicode text do not duplicate physical keys", () => {
  const app = harness();
  const socket = app.ready();
  app.nodes.desktop.fire("compositionstart");
  app.fireKey("KeyA", { isComposing: true });
  app.nodes.desktop.fire("compositionend", { data: "你好" });
  app.nodes.text.value = "日本語 🎉";
  app.nodes.text.fire("compositionstart");
  app.nodes["send-text"].fire("click");
  app.nodes.text.fire("compositionend");
  app.nodes["send-text"].fire("click");
  assert.deepEqual(socket.sent.filter((command) => command.type === "text"), [
    { type: "text", text: "你好" }, { type: "text", text: "日本語 🎉" },
  ]);
  assert.equal(socket.sent.some((command) => command.type === "key"), false);
  app.nodes.text.value = "界".repeat(6000);
  app.nodes["send-text"].fire("click");
  assert.equal(socket.sent.filter((command) => command.type === "text").length, 2);
});

test("clipboard is explicit, paste releases modifiers, and stale async reads are ignored", async () => {
  const app = harness();
  const socket = app.ready();
  socket.json({ type: "clipboard", text: "unsolicited" });
  assert.equal(app.nodes.text.value, "");
  app.nodes["clipboard-get"].fire("click");
  socket.json({ type: "clipboard", text: "<script>not markup</script>" });
  assert.equal(app.nodes.text.value, "<script>not markup</script>");
  app.nodes["clipboard-set"].fire("click");
  assert.equal(socket.sent.at(-1).type, "clipboard_set");
  app.fireKey("ControlLeft", { ctrlKey: true });
  app.fireKey("KeyV", { ctrlKey: true });
  app.nodes.desktop.fire("paste", { clipboardData: { getData: () => "pasted" } });
  assert.deepEqual(socket.sent.slice(-2), [{ type: "release" }, { type: "text", text: "pasted" }]);
  let resolveRead;
  app.navigator.clipboard.readText = () => new Promise((resolve) => { resolveRead = resolve; });
  app.nodes["paste-local"].fire("click");
  socket.finishClose();
  app.tick();
  const next = app.ready();
  resolveRead("stale");
  await flush();
  assert.deepEqual(next.sent, []);
});

test("resolution is explicit and validated", () => {
  const app = harness();
  const socket = app.ready();
  assert.deepEqual(socket.sent, []);
  app.nodes.width.value = "3841";
  app.nodes.resize.fire("submit");
  assert.deepEqual(socket.sent, []);
  app.nodes.width.value = "1920";
  app.nodes.height.value = "1080";
  app.nodes.resize.fire("submit");
  assert.deepEqual(socket.sent.at(-1), { type: "resize", width: 1920, height: 1080 });
  socket.json({ type: "ready", width: -1, height: 720 });
  assert.equal(app.nodes.desktop.width, 1280);
  socket.json({ type: "error", message: "resize unsupported" });
  assert.equal(app.nodes.message.textContent, "Server: resize unsupported");
});

test("weak-network reconnect continues with bounded backoff and manual disconnect cancels it", () => {
  const app = harness();
  app.sockets[0].finishClose();
  const delays = [];
  for (let index = 0; index < 10; index += 1) {
    delays.push(app.tick());
    app.sockets.at(-1).finishClose();
  }
  assert.deepEqual(delays.slice(0,5),[500,1000,2000,4000,8000]);
  assert.ok(delays.every(delay=>delay<=30000));
  assert.equal(app.sockets.length,11);
  assert.equal(app.timers.size,1);
  app.nodes.connection.fire("click");
  assert.equal(app.timers.size,0);
});

test("offline waits for online without spending reconnect attempts", () => {
  const app=harness();
  app.navigator.onLine=false;
  app.sockets[0].finishClose();
  assert.equal(app.timers.size,0);
  assert.match(app.nodes.state.textContent,/network/);
  app.navigator.onLine=true;
  app.window.fire("online");
  assert.equal(app.sockets.length,2);
  app.nodes.connection.fire("click");
  app.sockets.at(-1).finishClose();
  app.window.fire("online");
  assert.equal(app.sockets.length,2,"manual disconnect must not reconnect online");
});

test("silent visible transport is replaced after a bounded close without reviving stale callbacks", () => {
  const app=harness();
  const old=app.sockets[0];
  old.open();old.json({type:"ready",width:1280,height:720,metrics:true});
  app.advance(31000);
  assert.equal(app.tick(),1000);
  assert.equal(old.readyState,2);
  assert.equal(app.tick(),3000);
  app.tick();
  assert.equal(app.sockets.length,2);
  app.ready();
  old.json({type:"ready",width:800,height:600});
  old.finishClose();
  assert.equal(app.nodes.width.value,"1280");
});

test("recent server replies and a newly visible tab keep a static desktop connected", () => {
  const app=harness();const socket=app.sockets[0];
  socket.open();socket.json({type:"ready",width:1280,height:720,metrics:true});
  app.advance(29000);socket.json({type:"pong",id:1});
  app.advance(2000);app.tick();
  assert.equal(socket.readyState,1);
  app.document.hidden=true;app.advance(60000);app.tick();
  assert.equal(socket.readyState,1);
  app.document.hidden=false;app.document.fire("visibilitychange");app.tick();
  assert.equal(socket.readyState,1);
});

test("JPEG decode is single-flight with one latest pending frame; stale results are closed", async () => {
  const app = harness();
  const socket = app.ready();
  socket.binary(jpeg(10));
  socket.binary(jpeg(11));
  socket.binary(jpeg(12));
  assert.equal(app.decodes.length, 1);
  const first = bitmap();
  app.decodes[0].resolve(first);
  await flush();
  assert.equal(first.closed, true);
  assert.equal(app.decodes.length, 2);
  assert.deepEqual([...new Uint8Array(await app.decodes[1].blob.arrayBuffer())], [12]);
  socket.finishClose();
  app.tick();
  const next = app.ready();
  next.binary(jpeg(13));
  assert.equal(app.decodes.length, 2);
  const stale = bitmap();
  app.decodes[1].resolve(stale);
  await flush();
  assert.equal(stale.closed, true);
  assert.equal(app.draws.length, 1);
  assert.equal(app.decodes.length, 3);
  app.decodes[2].reject(new Error("bad JPEG"));
  await flush();
  assert.match(app.nodes.message.textContent, /decode/);
  next.binary(jpeg(14));
  assert.equal(app.decodes.length, 4);
});

test("resized JPEG updates canvas before drawing, preserves edited resolution, and avoids redundant resets", async () => {
  const app = harness();
  const socket = app.ready();
  const canvas = app.nodes.desktop;
  const dimensions = { width: canvas.width, height: canvas.height };
  const changes = [];
  for (const name of ["width", "height"]) {
    Object.defineProperty(canvas, name, {
      get: () => dimensions[name],
      set(value) { dimensions[name] = value; changes.push([name, value]); },
    });
  }
  app.nodes.width.value = "1600";
  app.nodes.height.value = "900";
  app.nodes.width.focus();
  socket.binary(jpeg(1));
  app.decodes[0].resolve(bitmap(1024, 768));
  await flush();
  assert.deepEqual(dimensions, { width: 1024, height: 768 });
  assert.equal(canvas.style.values["--desktop-aspect"], String(1024 / 768));
  assert.deepEqual(app.draws[0].slice(1), [0, 0, 1024, 768]);
  assert.deepEqual(changes, [["width", 1024], ["height", 768]]);
  assert.equal(app.nodes.width.value, "1600");
  assert.equal(app.nodes.height.value, "900");
  socket.binary(jpeg(2));
  app.decodes[1].resolve(bitmap(1024, 768));
  await flush();
  assert.equal(changes.length, 2);
  canvas.fire("pointermove", { clientX: 110, clientY: 45 });
  app.paint();
  assert.deepEqual(socket.sent.at(-1), { type: "move", x: 0.5, y: 0.25 });
  socket.json({ type: "ready", width: 1024, height: 768, audio: true });
  assert.equal(changes.length, 2);
  assert.equal(app.nodes.width.value, "1600");
  assert.equal(app.nodes.height.value, "900");
  socket.binary(jpeg(3));
  socket.finishClose();
  app.tick();
  const next = app.ready();
  const stale = bitmap(640, 480);
  app.decodes[2].resolve(stale);
  await flush();
  assert.equal(stale.closed, true);
  assert.deepEqual(dimensions, { width: 1280, height: 720 });
  assert.equal(app.draws.length, 2);
  next.binary(jpeg(4));
  const invalid = bitmap(8193, 720);
  app.decodes[3].resolve(invalid);
  await flush();
  assert.equal(invalid.closed, true);
  assert.deepEqual(dimensions, { width: 1280, height: 720 });
  assert.equal(app.draws.length, 2);
});

test("PCM reads unaligned little-endian stereo and bounds scheduled audio", async () => {
  const app = harness();
  const socket = app.ready();
  socket.binary(pcm());
  assert.equal(app.contexts.length, 0);
  app.nodes.audio.fire("click");
  await flush();
  const context = app.contexts[0];
  assert.equal(context.options.sampleRate, 48000);
  assert.deepEqual(socket.sent.at(-1), { type: "audio", enabled: true });
  socket.binary(pcm());
  assert.equal(context.buffers[0].data[0][0], 0.25);
  assert.equal(context.buffers[0].data[1][0], -0.5);
  for (let index = 0; index < 100; index += 1) socket.binary(pcm(480));
  assert.ok(context.sources.length < 32);
  app.document.hidden = true;
  app.document.fire("visibilitychange");
  assert.equal(context.state, "closed");
  assert.ok(context.sources.every((item) => item.stopped && item.disconnected));
  assert.deepEqual(socket.sent.at(-1), { type: "audio", enabled: false });
});

test("dedicated audio preserves gateway prefix and never enables audio on the video socket", async () => {
  const app = harness("/gateway/desktop/");
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 17 });
  app.nodes.audio.fire("click");
  await flush();
  assert.equal(app.sockets.length, 2);
  const audio = app.sockets[1];
  assert.equal(audio.url, "wss://desktop.test/gateway/desktop/audio?session=17");
  assert.equal(video.sent.some((command) => command.type === "audio"), false);
  audio.open();
  audio.json({ type: "audio_ready" });
  audio.binary(pcm());
  assert.equal(app.contexts[0].buffers.length, 1);
  video.binary(pcm());
  assert.equal(app.contexts[0].buffers.length, 1);
  app.nodes.audio.fire("click");
  assert.equal(audio.readyState, 2);
  assert.equal(video.readyState, 1);
  assert.equal(app.contexts[0].state, "closed");
});

function enableModernAudio(app) {
  app.AudioContext.prototype.audioWorklet = { async addModule(url) { app.moduleUrl = url; } };
  app.run(`
    globalThis.audioWorkers = [];
    globalThis.audioNodes = [];
    globalThis.MessageChannel = class { port1 = {}; port2 = {}; };
    globalThis.AudioWorkletNode = class {
      port = { postMessage() {}, close() { this.closed = true; } };
      constructor() { audioNodes.push(this); }
      connect() {}
      disconnect() { this.disconnected = true; }
    };
    globalThis.Worker = class {
      messages = [];
      constructor(url) { this.url = String(url); audioWorkers.push(this); }
      postMessage(data, transfers) { this.messages.push({ data, transfers }); }
      terminate() { this.terminated = true; }
    };
  `);
}

test("modern audio negotiates Opus and transfers packets without creating main-thread playback nodes", async () => {
  const app = harness("/gateway/desktop/");
  enableModernAudio(app);
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 29, audio_codecs: ["opus", "pcm"] });
  app.nodes.audio.fire("click");
  await flush();
  assert.equal(app.moduleUrl, "https://desktop.test/gateway/desktop/audio-worklet.js");
  const worker = app.run("audioWorkers[0]");
  assert.equal(worker.url, "https://desktop.test/gateway/desktop/audio-worker.js");
  assert.equal(worker.messages[0].data.allowOpus, true);
  worker.onmessage({ data: { type: "ready", codec: "opus" } });
  await flush();
  const audio = app.sockets[1];
  assert.equal(audio.url, "wss://desktop.test/gateway/desktop/audio?session=29&codec=opus");
  audio.open();
  audio.json({ type: "audio_ready", codec: "opus" });
  const packet = new Uint8Array(12); packet[0] = 4;
  audio.binary(packet.buffer);
  assert.equal(worker.messages.at(-1).data.buffer, packet.buffer);
  assert.equal(worker.messages.at(-1).transfers[0], packet.buffer);
  for (let count = 0; count < 50; count += 1) audio.binary(new Uint8Array([4, 0, 0, 0, 0, 0, 0, 0, 0, 0]).buffer);
  assert.equal(worker.messages.filter(message => message.data.type === "packet").length, 6);
  worker.onmessage({ data: { type: "consumed" } });
  audio.binary(new Uint8Array([4, 0, 0, 0, 0, 0, 0, 0, 0, 0]).buffer);
  assert.equal(worker.messages.filter(message => message.data.type === "packet").length, 7);
  assert.equal(app.contexts[0].sources.length, 0);
  assert.equal(app.contexts[0].buffers.length, 0);
  app.nodes.audio.fire("click");
  assert.equal(worker.terminated, true);
  assert.equal(app.run("audioNodes[0].disconnected"), true);
  assert.equal(app.run("audioNodes[0].port.closed"), true);
  assert.equal(video.readyState, 1);
});

test("modern PCM compatibility, decoder failure and late worker errors do not disconnect video", async () => {
  const app = harness();
  enableModernAudio(app);
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 31 });
  app.nodes.audio.fire("click");
  await flush();
  const worker = app.run("audioWorkers[0]");
  assert.equal(worker.messages[0].data.allowOpus, false);
  worker.onmessage({ data: { type: "ready", codec: "pcm" } });
  await flush();
  const audio = app.sockets[1];
  audio.open(); audio.json({ type: "audio_ready" }); audio.binary(pcm());
  assert.equal(worker.messages.at(-1).data.type, "packet");
  worker.onmessage({ data: { type: "error" } });
  assert.equal(worker.terminated, true);
  assert.equal(audio.readyState, 2);
  assert.equal(video.readyState, 1);
  app.nodes.audio.fire("click");
  await flush();
  worker.onerror();
  assert.equal(app.contexts[1].state, "running");
  app.run("client.destroy()");
  assert.equal(app.run("audioWorkers[1].terminated"), true);
  assert.equal(app.timers.size, 0);
});

test("modern audio initialization timeout releases workers and preserves the controller", async () => {
  const app = harness();
  enableModernAudio(app);
  const video = app.ready();
  app.nodes.audio.fire("click");
  await flush();
  app.tick();
  await flush();
  assert.equal(app.run("audioWorkers[0].terminated"), true);
  assert.equal(app.contexts[0].state, "closed");
  assert.equal(video.readyState, 1);
});

test("audio failure is isolated and stale audio events cannot stop its replacement", async () => {
  const app = harness();
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 19 });
  app.nodes.audio.fire("click");
  await flush();
  const first = app.sockets[1];
  first.finishClose();
  assert.equal(app.contexts[0].state, "closed");
  assert.equal(video.readyState, 1);
  app.nodes.audio.fire("click");
  await flush();
  const second = app.sockets[2];
  second.open();
  second.json({ type: "audio_ready" });
  first.finishClose();
  assert.equal(app.contexts[1].state, "running");
  video.finishClose();
  assert.equal(second.readyState, 2);
  assert.equal(app.contexts[1].state, "closed");
});

test("dedicated audio handshake timeout cleans up without reconnecting video", async () => {
  const app = harness();
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 21 });
  app.nodes.audio.fire("click");
  await flush();
  app.tick();
  assert.equal(app.sockets[1].readyState, 2);
  assert.equal(video.readyState, 1);
  assert.equal(app.contexts[0].state, "closed");
});

test("dedicated audio closes on page hiding and controller teardown without stale recapture", async () => {
  const app = harness();
  const video = app.ready();
  video.json({ type: "ready", width: 1280, height: 720, audio: true, audio_session: 23 });
  app.nodes.audio.fire("click");
  await flush();
  const audio = app.sockets[1];
  app.document.hidden = true;
  app.document.fire("visibilitychange");
  assert.equal(audio.readyState, 2);
  audio.json({ type: "audio_ready" });
  assert.equal(app.nodes.audio.attributes["aria-pressed"], "false");
  assert.equal(video.readyState, 1);
  app.run("client.destroy()");
  assert.equal(app.timers.size, 0);
});

test("late audio permission cannot enable audio on a replacement socket", async () => {
  const app = harness();
  const socket = app.ready();
  let resume;
  app.AudioContext.prototype.resume = () => new Promise((resolve) => { resume = resolve; });
  app.nodes.audio.fire("click");
  socket.finishClose();
  app.tick();
  const next = app.ready();
  resume();
  await flush();
  assert.equal(app.contexts[0].state, "closed");
  assert.deepEqual(next.sent, []);
});

test("backpressure closes instead of queueing input; destroy removes listeners and timers", () => {
  const app = harness();
  const socket = app.ready();
  socket.bufferedAmount = 300000;
  app.fireKey("KeyA");
  assert.equal(socket.readyState, 2);
  assert.deepEqual(socket.sent, []);
  socket.finishClose();
  assert.equal(app.timers.size, 1);
  app.run("client.destroy()");
  assert.equal(app.timers.size, 0);
  for (const target of [app.window, app.document, ...Object.values(app.nodes)]) {
    for (const handlers of target.handlers.values()) assert.equal(handlers.size, 0);
  }
});

test("manual disconnect releases held input and BFCache restore reconnects once", () => {
  const app = harness();
  const socket = app.ready();
  app.fireKey("ShiftLeft", { shiftKey: true });
  app.window.fire("pagehide", { persisted: true });
  assert.ok(socket.sent.some((command) => command.type === "release"));
  app.window.fire("pageshow", { persisted: true });
  assert.equal(app.sockets.length, 1);
  socket.finishClose();
  app.tick();
  assert.equal(app.sockets.length, 2);
  app.window.fire("pagehide", { persisted: false });
  assert.equal(app.sockets[1].onmessage, null);
  assert.equal(app.timers.size, 0);
});

test("codec and streaming strategy default to automatic and invalid saved values are rejected", () => {
  for (const storage of [new Map(), new Map([["lightos-desktop-video-codec", "invalid"], ["lightos-desktop-stream-strategy", "invalid"]])]) {
    const app = harness("/", storage);
    assert.equal(app.nodes["video-codec"].value,"auto");
    assert.equal(app.nodes["stream-strategy"].value,"auto");
  }
});

test("manual JPEG and scheduling preferences persist and are sent on reconnect", () => {
  const storage = new Map();
  const app = harness("/", storage);
  app.nodes["video-codec"].value = "jpeg";
  app.nodes["video-codec"].fire("change");
  app.nodes["stream-strategy"].value = "low_latency";
  app.nodes["stream-strategy"].fire("change");
  assert.equal(storage.get("lightos-desktop-video-codec"),"jpeg");
  assert.equal(storage.get("lightos-desktop-stream-strategy"),"low_latency");
  const next = harness("/", storage);
  const socket = next.sockets.at(-1);
  socket.open();
  socket.json({type:"ready",width:1280,height:720,frame_ack:true,frame_ids:true,video_codecs:["h264","jpeg"]});
  assert.ok(socket.sent.some(packet => packet.type === "video" && packet.codec === "jpeg"));
  assert.ok(socket.sent.some(packet => packet.type === "video_flow" && packet.strategy === "low_latency"));
});

test("manual unsupported H264 reports JPEG fallback while preserving the preference", () => {
  const storage = new Map([["lightos-desktop-video-codec","h264"]]);
  const app = harness("/",storage);
  const socket = app.sockets.at(-1);
  socket.open();
  socket.json({type:"ready",width:1280,height:720,frame_ack:true,frame_ids:true,video_codecs:["jpeg"]});
  assert.ok(socket.sent.some(packet => packet.type === "video" && packet.codec === "jpeg"));
  assert.equal(app.nodes["video-codec"].value,"h264");
  assert.match(app.nodes.message.textContent,/H\.264.*unavailable.*JPEG/);
  assert.equal(storage.get("lightos-desktop-video-codec"),"h264");
});

 test("quality-first preference has a real option and survives reconnect", () => {
  assert.match(html,/option value="quality_first"/);
  const storage=new Map();const app=harness("/",storage);
  app.nodes["stream-strategy"].value="quality_first";app.nodes["stream-strategy"].fire("change");
  assert.equal(storage.get("lightos-desktop-stream-strategy"),"quality_first");
  const next=harness("/",storage);const socket=next.sockets.at(-1);socket.open();
  socket.json({type:"ready",width:1280,height:720,frame_ack:true,frame_ids:true});
  assert.ok(socket.sent.some(packet=>packet.type==="video_flow"&&packet.strategy==="quality_first"));
});

test("hardware backend defaults to auto, persists and is negotiated independently", () => {
  const storage=new Map(); const app=harness("/",storage);
  assert.equal(app.nodes["encoder-backend"].value,"auto");
  app.nodes["encoder-backend"].value="nvenc"; app.nodes["encoder-backend"].fire("change");
  assert.equal(storage.get("lightos-desktop-encoder-backend"),"nvenc");
  const next=harness("/",storage); next.window.VideoDecoder = class {}; next.window.EncodedVideoChunk = class {}; const socket=next.sockets.at(-1); socket.open();
  socket.json({type:"ready",width:1280,height:720,frame_ack:true,frame_ids:true,video_codecs:["h264","jpeg"],backend_selection:true});
  assert.ok(socket.sent.some(p=>p.type==="video"&&p.codec==="h264"&&p.backend==="nvenc"));
});

test("browser rejects the active hardware profile and requests another H264 backend before JPEG", async () => {
  const app=harness();const socket=app.sockets.at(-1);socket.open();
  socket.json({type:"ready",width:1280,height:720,frame_ack:true,frame_ids:true,video_codecs:["h264","jpeg"],backend_selection:true});
  socket.sent.length=0;
  socket.json({type:"video_config",codec:"avc1.invalid",width:1280,height:720,backend:"nvenc"});
  await flush();
  assert.ok(socket.sent.some(p=>p.type==="video"&&p.codec==="h264"&&p.rejected_backend==="nvenc"));
  socket.binary(new Uint8Array([3,1,0,0,0,0,0,0,0,0,0,0,1,0x65]).buffer);
  await flush();
  assert.equal(socket.sent.some(p=>p.type==="video"&&p.codec==="jpeg"),false);
});
