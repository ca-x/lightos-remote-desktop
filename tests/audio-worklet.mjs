import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function player(rate = 48000) {
  let Processor;
  const messages = [];
  class AudioWorkletProcessor {
    port = { postMessage: (message) => messages.push(message) };
  }
  vm.runInNewContext(readFileSync(new URL("../web/src/audio-worklet.js", import.meta.url), "utf8"), {
    AudioWorkletProcessor, sampleRate: rate, Float32Array, Math,
    registerProcessor: (name, implementation) => { assert.equal(name, "remote-audio"); Processor = implementation; },
  });
  const processor = new Processor();
  const channel = { start() {}, postMessage() {} };
  processor.port.onmessage({ data: { port: channel } });
  const push = (frames, value = 0.25) => channel.onmessage({ data: {
    left: new Float32Array(frames).fill(value), right: new Float32Array(frames).fill(-value),
  } });
  const render = (frames = 128) => {
    const output = [new Float32Array(frames), new Float32Array(frames)];
    assert.equal(processor.process([], [output]), true);
    return output;
  };
  return { processor, push, render, messages };
}

test("worklet prebuffers stereo, renders samples and fills underruns with silence", () => {
  const app = player();
  app.push(960);
  assert.equal(app.render()[0][0], 0);
  app.push(960);
  assert.equal(app.render()[0][0], 0.25);
  assert.equal(app.render()[1][0], -0.25);
  for (let count = 0; count < 20; count += 1) app.render();
  assert.equal(app.render()[0][0], 0);
});

test("worklet discards old backlog instead of increasing playback latency", () => {
  const app = player();
  for (let count = 0; count < 100; count += 1) app.push(960, count / 100);
  assert.ok(app.processor.available <= 5760);
  assert.ok(app.render()[0][0] > 0.9);
  assert.ok(app.processor.dropped > 0);
});

test("worklet resamples when the device cannot use a 48kHz context", () => {
  const app = player(44100);
  app.push(1920);
  const before = app.processor.available;
  const output = app.render(441);
  assert.equal(output[0][0], 0.25);
  assert.ok(Math.abs(before - app.processor.available - 480) < 2);
});

test("worklet rejects malformed and oversized packets", () => {
  const app = player();
  app.processor.port.onmessage({ data: {} });
  app.push(20000);
  assert.equal(app.processor.available, 0);
});


test("interruption clears queued playback and re-prebuffers fresh samples", () => {
  const app = player();
  app.push(1920, 0.4);
  app.processor.port.onmessage({ data: { type: "reset" } });
  assert.equal(app.render()[0][0], 0);
  app.push(1920, 0.25);
  assert.equal(app.render()[0][0], 0.25);
});
