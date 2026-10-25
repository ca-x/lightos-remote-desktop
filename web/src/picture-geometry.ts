export interface PictureRect { left: number; top: number; width: number; height: number }
export interface PictureCoordinate { x: number; y: number }

/** The bounding rectangle is in screen coordinates; desktop coordinates stay unrotated. */
export function pictureNormalizedPoint(rect: PictureRect, point: PictureCoordinate, rotated = false): PictureCoordinate {
  const x = (point.x - rect.left) / rect.width, y = (point.y - rect.top) / rect.height;
  return rotated ? { x: y, y: 1 - x } : { x, y };
}

export function pictureClientPoint(rect: PictureRect, point: PictureCoordinate, rotated = false): PictureCoordinate {
  return { x: rect.left + (rotated ? 1 - point.y : point.x) * rect.width,
    y: rect.top + (rotated ? point.x : point.y) * rect.height };
}

export function pictureScrollDelta(x: number, y: number, rotated = false): PictureCoordinate {
  return rotated ? { x: y, y: -x } : { x, y };
}

/** Convert clipping at screen edges into insets in the original image axes. */
export function pictureClip(top: number, right: number, bottom: number, left: number, rotated = false): string {
  const insets = rotated ? [right, bottom, left, top] : [top, right, bottom, left];
  return `inset(${insets.map(value => `${Math.max(0, value)}px`).join(" ")})`;
}
