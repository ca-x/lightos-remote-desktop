import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function worker(supported = true) {
  const messages = [];
  const output = [];
  const decoders = [];
  class AudioDecoder {
    state = "unconfigured";
    decodeQueueSize = 0;
    packets = [];
    constructor(callbacks) { this.callbacks = callbacks; decoders.push(this); }
    static async isConfigSupported() { if (supported === "throws") throw new Error("unsupported codec"); return { supported }; }
    configure(config) { this.config = config; this.state = "configured"; }
    decode(packet) { this.packets.push(packet); }
    reset() { this.resets = (this.resets ?? 0) + 1; this.decodeQueueSize = 0; }
    close() { this.state = "closed"; }
  }
  const self = { postMessage: (message) => messages.push(message) };
  vm.runInNewContext(readFileSync(new URL("../web/src/audio-worker.js", import.meta.url), "utf8"), {
    self, AudioDecoder, EncodedAudioChunk: class { constructor(config) { Object.assign(this, config); } },
    ArrayBuffer, Uint8Array, DataView, Float32Array,
  });
  const send = (data) => self.onmessage({ data });
  const port = { postMessage: (data) => output.push(data) };
  return { messages, output, decoders, send, port, init: (allowOpus = true) => send({ type: "init", allowOpus, port }) };
}

test("worker negotiates Opus and transfers decoded stereo out of the UI thread", async () => {
  const app = worker();
  await app.init();
  assert.equal(app.messages[0].codec, "opus");
  const packet = new ArrayBuffer(12);
  const view = new DataView(packet);
  view.setUint8(0, 4);
  view.setUint32(1, 20000, true);
  await app.send({ type: "packet", buffer: packet });
  assert.equal(app.decoders[0].packets[0].timestamp, 20000);
  assert.equal(app.decoders[0].packets[0].duration, 20000);
  let closed = false;
  app.decoders[0].callbacks.output({ numberOfFrames: 960, sampleRate: 48000, numberOfChannels: 2,
    copyTo(target, options) { target.fill(options.planeIndex ? -0.5 : 0.25); },
    close() { closed = true; },
  });
  assert.equal(closed, true);
  assert.equal(app.output[0].left[0], 0.25);
  assert.equal(app.output[0].right[0], -0.5);
});

test("worker supports PCM when the server or browser lacks Opus", async () => {
  for (const [supported, allowOpus] of [[false, true], [true, false], ["throws", true]]) {
    const app = worker(supported);
    await app.init(allowOpus);
    assert.equal(app.messages[0].codec, "pcm");
    const packet = new ArrayBuffer(9);
    const view = new DataView(packet);
    view.setUint8(0, 2);
    view.setFloat32(1, 0.25, true);
    view.setFloat32(5, -0.5, true);
    await app.send({ type: "packet", buffer: packet });
    assert.equal(app.output[0].left[0], 0.25);
    assert.equal(app.output[0].right[0], -0.5);
    assert.equal(app.decoders.length, 0);
  }
});

test("worker bounds decoder backlog and ignores malformed packets", async () => {
  const app = worker();
  await app.init();
  await app.send({ type: "packet", buffer: new ArrayBuffer(3) });
  assert.equal(app.decoders[0].packets.length, 0);
  app.decoders[0].decodeQueueSize = 6;
  const packet = new ArrayBuffer(10);
  new Uint8Array(packet)[0] = 4;
  await app.send({ type: "packet", buffer: packet });
  assert.equal(app.decoders[0].resets, 1);
  assert.equal(app.decoders[0].packets.length, 1);
});

test("worker closes AudioData even on copy errors and reports isolated failure", async () => {
  const app = worker();
  await app.init();
  let closed = false;
  app.decoders[0].callbacks.output({ numberOfFrames: 960, sampleRate: 48000, numberOfChannels: 2,
    copyTo() { throw new Error("decoder failure"); }, close() { closed = true; },
  });
  assert.equal(closed, true);
  assert.equal(app.messages.at(-1).type, "error");
  assert.equal(app.decoders[0].state, "closed");
});

test("worker-to-worklet credits cap transfers and recover when a packet is consumed", async () => {
  const app = worker(false);
  await app.init();
  const packet = new ArrayBuffer(9); new Uint8Array(packet)[0] = 2;
  for (let count = 0; count < 50; count += 1) await app.send({ type: "packet", buffer: packet });
  assert.equal(app.output.length, 6);
  assert.equal(app.messages.filter(message => message.type === "consumed").length, 50);
  app.port.onmessage();
  await app.send({ type: "packet", buffer: packet });
  assert.equal(app.output.length, 7);
});
