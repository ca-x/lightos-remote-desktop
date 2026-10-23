export const LEGACY_SCALES = [3, 2.5, 2, 1.5, 1] as const;
export const CAPTURE_SCALES = [3, 2.75, 2.5, 2.25, 2, 1.75, 1.5, 1.25, 1] as const;

// Pair capture pixels with compositor scale so sharper text keeps its logical
// size. Quarter scales need eight logical pixels for even encoder dimensions.
export function captureResolution(cssWidth: number, cssHeight: number, pixelRatio: number, scales: readonly number[] = CAPTURE_SCALES) {
  const ratio = Number.isFinite(pixelRatio) ? Math.max(1, pixelRatio) : 1;
  for (const scale of CAPTURE_SCALES) {
    if (scale > ratio || !scales.includes(scale)) continue;
    const step = Number.isInteger(scale) ? 2 : Number.isInteger(scale * 2) ? 4 : 8;
    const logicalWidth = Math.max(320, Math.floor(cssWidth / step) * step);
    const logicalHeight = Math.max(200, Math.floor(cssHeight / step) * step);
    const width = logicalWidth * scale, height = logicalHeight * scale;
    if (width <= 3840 && height <= 2160) return { width, height, scale };
  }
  return { width: Math.min(3840, Math.max(320, Math.floor(cssWidth / 2) * 2)), height: Math.min(2160, Math.max(200, Math.floor(cssHeight / 2) * 2)), scale: 1 };
}
