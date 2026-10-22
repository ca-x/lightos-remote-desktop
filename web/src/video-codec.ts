/** Browser decode availability is independent of the server's encoder availability. */
export async function supportedVideoConfig(
  decoder: typeof VideoDecoder,
  codec: string,
  width: number,
  height: number,
): Promise<VideoDecoderConfig | null> {
  for (const hardwareAcceleration of ["prefer-hardware", "no-preference"] as const) {
    const config: VideoDecoderConfig = {
      codec,
      codedWidth: width,
      codedHeight: height,
      optimizeForLatency: true,
      hardwareAcceleration,
    };
    try {
      const support = await decoder.isConfigSupported(config);
      if (support.supported) return config;
    } catch {
      // An unavailable hardware path must still permit a software decoder probe.
    }
  }
  return null;
}
