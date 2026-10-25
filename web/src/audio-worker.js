let output;
let decoder;
let stopped = false;
let codec = "pcm";
let inFlight = 0;
const configuration = { codec: "opus", sampleRate: 48000, numberOfChannels: 2 };

function samples(left, right) {
  if (inFlight >= 6) return;
  inFlight += 1;
  output.postMessage({ left, right }, [left.buffer, right.buffer]);
}

function fail() {
  if (stopped) return;
  stopped = true;
  if (decoder && decoder.state !== "closed") decoder.close();
  self.postMessage({ type: "error" });
}

function decoded(frame) {
  try {
    if (stopped || frame.numberOfFrames > 5760 || frame.sampleRate !== 48000 || frame.numberOfChannels !== 2) return;
    const left = new Float32Array(frame.numberOfFrames);
    const right = new Float32Array(frame.numberOfFrames);
    frame.copyTo(left, { planeIndex: 0, format: "f32-planar" });
    frame.copyTo(right, { planeIndex: 1, format: "f32-planar" });
    samples(left, right);
  } catch { fail(); }
  finally { frame.close(); }
}

self.onmessage = async ({ data }) => {
  if (stopped) return;
  try {
    if (data.type === "init") {
      if (output) return;
      output = data.port;
      output.onmessage = () => { inFlight = Math.max(0, inFlight - 1); };
      output.start?.();
      if (data.allowOpus && typeof AudioDecoder !== "undefined" && typeof EncodedAudioChunk !== "undefined") {
        try {
          const support = await AudioDecoder.isConfigSupported(configuration);
          if (stopped) return;
          if (support.supported) {
            decoder = new AudioDecoder({ output: decoded, error: fail });
            decoder.configure(configuration);
            codec = "opus";
          }
        } catch {
          // Partial WebCodecs implementations may throw for an unsupported
          // configuration. Negotiate PCM before opening the audio transport.
          if (decoder && decoder.state !== "closed") decoder.close();
          decoder = undefined;
        }
      }
      self.postMessage({ type: "ready", codec });
      return;
    }
    if (data.type !== "packet" || !output || !(data.buffer instanceof ArrayBuffer)) return;
    const buffer = data.buffer;
    const view = new DataView(buffer);
    if (codec === "opus") {
      if (buffer.byteLength < 10 || view.getUint8(0) !== 4 || buffer.byteLength > 1284) return;
      if (decoder.decodeQueueSize >= 6) {
        decoder.reset();
        decoder.configure(configuration);
      }
      const timestamp = view.getUint32(1, true) + view.getUint32(5, true) * 4294967296;
      decoder.decode(new EncodedAudioChunk({ type: "key", timestamp, duration: 20000, data: new Uint8Array(buffer, 9) }));
    } else {
      if (buffer.byteLength < 9 || view.getUint8(0) !== 2 || (buffer.byteLength - 1) % 8 !== 0) return;
      const frames = (buffer.byteLength - 1) / 8;
      if (frames > 5760) return;
      const left = new Float32Array(frames);
      const right = new Float32Array(frames);
      for (let index = 0; index < frames; index += 1) {
        left[index] = view.getFloat32(1 + index * 8, true);
        right[index] = view.getFloat32(5 + index * 8, true);
      }
      samples(left, right);
    }
  } catch { fail(); }
  finally { if (data.type === "packet") self.postMessage({ type: "consumed" }); }
};
