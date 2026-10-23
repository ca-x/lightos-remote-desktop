export interface DesktopTile { x: number; y: number; width: number; height: number; png: Uint8Array<ArrayBuffer> }

export function parseDesktopUpdate(data: ArrayBuffer) {
  if (data.byteLength < 13 || data.byteLength > 2 * 1024 * 1024) throw Error("Invalid desktop update size");
  const view = new DataView(data);
  const width = view.getUint32(1, true), height = view.getUint32(5, true), count = view.getUint32(9, true);
  if (view.getUint8(0) !== 4 || !width || !height || width > 8192 || height > 8192 || width * height > 33554432 || count < 1 || count > 32) throw Error("Invalid desktop update dimensions");
  let offset = 13;
  const tiles: DesktopTile[] = [];
  for (let index = 0; index < count; index++) {
    if (offset + 20 > data.byteLength) throw Error("Incomplete desktop update");
    const x = view.getUint32(offset, true), y = view.getUint32(offset + 4, true);
    const w = view.getUint32(offset + 8, true), h = view.getUint32(offset + 12, true), length = view.getUint32(offset + 16, true);
    offset += 20;
    if (!w || !h || w > 64 || h > 64 || x + w > width || y + h > height || length < 8 || length > 32768 || offset + length > data.byteLength) throw Error("Invalid desktop update region");
    const png = new Uint8Array(data, offset, length);
    if (png[0] !== 137 || png[1] !== 80 || png[2] !== 78 || png[3] !== 71) throw Error("Invalid desktop update image");
    tiles.push({ x, y, width: w, height: h, png }); offset += length;
  }
  if (offset !== data.byteLength) throw Error("Trailing desktop update data");
  return { width, height, tiles };
}

export async function decodeDesktopTiles(tiles: DesktopTile[]) {
  const images: ImageBitmap[] = [];
  try {
    const results = await Promise.allSettled(tiles.map(async tile => {
      const image = await createImageBitmap(new Blob([tile.png], { type: "image/png" }));
      images.push(image);
      if (image.width !== tile.width || image.height !== tile.height) throw Error("Invalid decoded desktop tile");
      return { ...tile, image };
    }));
    const failure = results.find(result => result.status === "rejected");
    if (failure?.status === "rejected") throw failure.reason;
    return results.map(result => {
      if (result.status !== "fulfilled") throw Error("Desktop tile decode failed");
      return result.value;
    });
  } catch (error) {
    // Wait for all parallel decoders before releasing their bitmaps.
    for (const image of images) image.close();
    throw error;
  }
}
