/** Browser decode availability is independent of the server's encoder availability. */
export async function supportedVideoConfig(
  decoder: typeof VideoDecoder,
  codec: string,
  width: number,
  height: number,
  preferSoftware = false,
): Promise<VideoDecoderConfig | null> {
  const choices: HardwareAcceleration[] = preferSoftware ? ["prefer-software", "no-preference"] : ["prefer-hardware", "no-preference"];
  for (const hardwareAcceleration of choices) {
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
