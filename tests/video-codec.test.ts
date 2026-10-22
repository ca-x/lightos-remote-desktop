import { describe, expect, it } from "vitest";
import { supportedVideoConfig } from "../web/src/video-codec";

describe("browser video codec adapter", () => {
  it("prefers a proven hardware decode configuration", async () => {
    const requests: VideoDecoderConfig[] = [];
    const decoder = { isConfigSupported: async (config: VideoDecoderConfig) => {
      requests.push(config); return { supported: true };
    }} as unknown as typeof VideoDecoder;
    const config = await supportedVideoConfig(decoder, "avc1.42c01f", 800, 600);
    expect(config?.hardwareAcceleration).toBe("prefer-hardware");
    expect(requests).toHaveLength(1);
  });
  it("probes the general decoder when the hardware query rejects or throws", async () => {
    for (const fail of [false, true]) {
      const requests: VideoDecoderConfig[] = [];
      const decoder = { isConfigSupported: async (config: VideoDecoderConfig) => {
        requests.push(config);
        if (requests.length === 1) {
          if (fail) throw new Error("hardware unavailable");
          return {supported:false};
        }
        return {supported:true};
      }} as unknown as typeof VideoDecoder;
      const config = await supportedVideoConfig(decoder, "avc1.42c01f", 800, 600);
      expect(config?.hardwareAcceleration).toBe("no-preference");
      expect(requests).toHaveLength(2);
    }
  });
  it("returns unavailable when every decoder probe fails", async () => {
    const decoder = {isConfigSupported: async () => {throw new Error("unsupported");}} as unknown as typeof VideoDecoder;
    expect(await supportedVideoConfig(decoder,"avc1.42c01f",800,600)).toBeNull();
  });
});
