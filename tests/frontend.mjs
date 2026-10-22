import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../web/app.js", import.meta.url), "utf8");
const html = readFileSync(new URL("../web/index.html", import.meta.url), "utf8");
const flush = () => new Promise((resolve) => setImmediate(resolve));

function harness(pathname = "/") {
  class Target {
    handlers = new Map();
    disabled = false;
    value = "";
    textContent = "";
    captures = new Set();
    attributes = {};
    classList = { toggle() {} };
    addEventListener(name, callback, options) {
      if (!this.handlers.has(name)) this.handlers.set(name, new Set());
      this.handlers.get(name).add(callback);
      options?.signal?.cleanups.push(() => this.handlers.get(name).delete(callback));
    }
    fire(name, properties = {}) {
      const event = { target: this, preventDefault() { this.defaultPrevented = true; }, ...properties };
      for (const callback of this.handlers.get(name) || []) callback(event);
      return event;
    }
    setAttribute(name, value) { this.attributes[name] = value; }
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
  document.getElementById = (id) => {
    assert.ok(nodes[id], `Missing element ${id}`);
    return nodes[id];
  };
  const draws = [];
  nodes.desktop.getContext = () => ({ drawImage: (...args) => draws.push(args) });
  nodes.width.value = "1280";
  nodes.height.value = "720";
  const window = new Target();
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
  let timerId = 0;
  const decodes = [];
  const contexts = [];
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
  const context = vm.createContext({ document, window, navigator, AbortController, WebSocket,
    URL, location: { href: `https://desktop.test${pathname}`, pathname, protocol: "https:" },
    ArrayBuffer, Uint8Array, DataView, TextEncoder, Blob,
    setTimeout(callback, delay) { timers.set(++timerId, { callback, delay }); return timerId; },
    clearTimeout(id) { timers.delete(id); },
    createImageBitmap(blob) { return new Promise((resolve, reject) => decodes.push({ blob, resolve, reject })); },
  });
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
  return { nodes, document, window, sockets, ready, fireKey, run, timers, tick, decodes, draws, contexts, navigator, AudioContext };
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
  assert.equal(app.sockets[0].url, "wss://desktop.test/ws");
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
  assert.equal(app.sockets[0].url, "wss://desktop.test/remote-desktop/test-opaque-path/ws");
  assert.match(html, /href="style\.css"/);
  assert.match(html, /src="app\.js"/);
  for (const asset of ["style.css", "app.js"]) {
    assert.equal(new URL(asset, `https://desktop.test${pathname}`).pathname, `${pathname}${asset}`);
  }
  app.sockets[0].finishClose();
  app.tick();
  assert.equal(app.sockets[1].url, "wss://desktop.test/remote-desktop/test-opaque-path/ws");
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
  assert.equal(app.timers.size, 0);
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

test("reconnect has bounded backoff and never overlaps a closing socket", () => {
  const app = harness();
  app.nodes.connection.fire("click");
  app.nodes.connection.fire("click");
  assert.equal(app.sockets.length, 1);
  app.sockets[0].finishClose();
  const delays = [];
  for (let index = 0; index < 9; index += 1) {
    delays.push(app.tick());
    app.sockets.at(-1).finishClose();
  }
  assert.deepEqual(delays, [500, 1000, 2000, 4000, 8000, 10000, 10000, 10000, 10000]);
  app.nodes.connection.fire("click");
  assert.equal(app.timers.size, 0);
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
  assert.deepEqual(app.draws[0].slice(1), [0, 0, 1024, 768]);
  assert.deepEqual(changes, [["width", 1024], ["height", 768]]);
  assert.equal(app.nodes.width.value, "1600");
  assert.equal(app.nodes.height.value, "900");
  socket.binary(jpeg(2));
  app.decodes[1].resolve(bitmap(1024, 768));
  await flush();
  assert.equal(changes.length, 2);
  canvas.fire("pointermove", { clientX: 110, clientY: 45 });
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
