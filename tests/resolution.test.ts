import { expect, it } from "vitest";
import { captureResolution, LEGACY_SCALES } from "../web/src/resolution";

it("matches PC device pixels and preserves the compositor's logical desktop size", () => {
  for (const scale of [1, 1.25, 1.5, 1.75, 2, 2.25, 2.5]) {
    expect(captureResolution(1280, 800, scale)).toEqual({ width: 1280 * scale, height: 800 * scale, scale });
  }
  expect(captureResolution(1280, 768, 2.75)).toEqual({ width: 3520, height: 2112, scale: 2.75 });
});

it("bounds 4K capture and keeps fractional scales divisible into whole logical pixels", () => {
  for (const ratio of [1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 2.75, 3, NaN]) {
    for (const [width, height] of [[1365, 767], [1920, 1080], [1920, 1200], [3840, 2160], [7680, 4320]]) {
      const mode = captureResolution(width, height, ratio);
      expect(mode.width % 2).toBe(0); expect(mode.height % 2).toBe(0);
      expect(mode.width).toBeLessThanOrEqual(3840); expect(mode.height).toBeLessThanOrEqual(2160);
      expect(mode.width / mode.scale % 1).toBe(0); expect(mode.height / mode.scale % 1).toBe(0);
    }
  }
});

it("retains negotiated older scale support", () => {
  expect(captureResolution(1280, 800, 1.25, LEGACY_SCALES)).toEqual({ width: 1280, height: 800, scale: 1 });
  expect(captureResolution(1280, 800, 1.75, LEGACY_SCALES)).toEqual({ width: 1920, height: 1200, scale: 1.5 });
});
