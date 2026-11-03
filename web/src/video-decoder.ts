import workerUrl from "./video-worker.js?url";

/** A bounded decoder. VideoFrame transfers retain GPU storage across threads. */
export interface DesktopDecoder {
  readonly state: CodecState;
  readonly decodeQueueSize: number;
  decode(chunk: EncodedVideoChunk): void;
  close(): void;
  addEventListener(type: "dequeue", listener: () => void): void;
}

export async function createDesktopDecoder(config: VideoDecoderConfig, init: VideoDecoderInit): Promise<DesktopDecoder> {
  if (typeof Worker !== "undefined") {
    try { return await workerDecoder(config, init); } catch { /* Retain browsers without worker WebCodecs. */ }
  }
  const decoder = new VideoDecoder(init);
  decoder.configure(config);
  return decoder;
}

function workerDecoder(config: VideoDecoderConfig, init: VideoDecoderInit): Promise<DesktopDecoder> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(workerUrl, { name: "desktop-video" });
    const events = new EventTarget();
    let state: CodecState = "unconfigured";
    let queued = 0;
    let settled = false;
    const timer = setTimeout(() => fail("Video worker initialization timed out"), 1500);
    const decoder: DesktopDecoder = {
      get state() { return state; },
      get decodeQueueSize() { return queued; },
      decode(chunk) {
        if (state !== "configured") throw new Error("Video decoder is not configured");
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        queued += 1;
        worker.postMessage({ type: "decode", data: data.buffer, timestamp: chunk.timestamp, key: chunk.type === "key" }, [data.buffer]);
      },
      close() { state = "closed"; queued = 0; clearTimeout(timer); worker.terminate(); },
      addEventListener(type, listener) { events.addEventListener(type, listener); },
    };
    function fail(message: string) {
      decoder.close();
      if (!settled) { settled = true; reject(new Error(message)); }
      else init.error(new DOMException(message, "EncodingError"));
    }
    worker.onerror = () => fail("Video worker failed");
    worker.onmessage = ({ data }) => {
      if (data.type === "frame") {
        const frame = data.frame as VideoFrame;
        if (state !== "configured") { frame.close(); return; }
        queued = Math.max(0, queued - 1);
        init.output(frame);
        events.dispatchEvent(new Event("dequeue"));
      } else if (data.type === "ready" && !settled) {
        clearTimeout(timer); settled = true; state = "configured"; resolve(decoder);
      } else if (data.type === "error") fail(String(data.message));
    };
    worker.postMessage({ type: "configure", config });
  });
}
