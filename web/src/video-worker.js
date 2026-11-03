// Decoding is isolated from layout, input handlers and toolbar updates. Frames
// cross the worker boundary as transferable VideoFrames, without pixel readback.
let decoder;
const fail = (error) => {
  try { decoder?.close(); } catch {}
  decoder = undefined;
  self.postMessage({ type: "error", message: error?.message || String(error) });
};
self.onmessage = async ({ data }) => {
  try {
    if (data.type === "configure") {
      if (typeof VideoDecoder === "undefined") throw new Error("Worker WebCodecs unavailable");
      const support = await VideoDecoder.isConfigSupported(data.config);
      if (!support.supported) throw new Error("Worker decoder configuration unavailable");
      decoder = new VideoDecoder({
        output(frame) { self.postMessage({ type: "frame", frame }, [frame]); },
        error: fail,
      });
      decoder.configure(data.config);
      self.postMessage({ type: "ready" });
    } else if (data.type === "decode") {
      if (!decoder || decoder.state !== "configured") throw new Error("Decoder not ready");
      decoder.decode(new EncodedVideoChunk({ type: data.key ? "key" : "delta", timestamp: data.timestamp, data: data.data }));
    }
  } catch (error) { fail(error); }
};
