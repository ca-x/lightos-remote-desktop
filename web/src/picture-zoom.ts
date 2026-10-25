import { pictureClientPoint, pictureClip } from "./picture-geometry";

export interface PicturePoint { x: number; y: number }
export interface PictureBounds { left: number; top: number; right: number; bottom: number }

/** Local picture magnification. It never changes the browser or remote resolution. */
export function createPictureZoom(options: {
  canvas: HTMLCanvasElement;
  viewport: HTMLElement;
  enabled: () => boolean;
  bounds: () => PictureBounds;
  touching: () => boolean;
  changed: () => void;
}) {
  const { canvas, viewport } = options;
  const lifecycle = new AbortController();
  let scale = 1, x = 0, y = 0;
  let cover = false, automaticCover = false;
  let rotated = false;
  let safari: { scale: number; point: PicturePoint } | null = null;
  const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
  const listen = (target: EventTarget, name: string, handler: (event: any) => void) => {
    target.addEventListener(name, handler, { signal: lifecycle.signal, capture: true, passive: false });
  };
  function base() {
    const rect = canvas.getBoundingClientRect();
    return { left: rect.left - x, top: rect.top - y, width: rect.width / scale, height: rect.height / scale };
  }
  function transform() {
    if (rotated) {
      const style = window.getComputedStyle(canvas);
      const width = parseFloat(style.width) || canvas.offsetWidth || 0;
      const height = parseFloat(style.height) || canvas.offsetHeight || 0;
      // Rotate about the picture's center, then zoom from its visual top left.
      // Translation remains in screen axes, so pinch/pan math stays unchanged.
      canvas.style.transform = `translate(${x + (width-height)/2 + height*scale}px, ${y + (height-width)/2}px) rotate(90deg) scale(${scale})`;
    } else if (scale === 1 && x === 0 && y === 0) canvas.style.removeProperty("transform");
    else canvas.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
  }
  function reset() {
    scale = 1; x = y = 0; safari = null;
    automaticCover = cover;
    transform();
    canvas.style.removeProperty("clip-path");
    options.changed();
  }
  function render(rect?: ReturnType<typeof base>) {
    if (!options.enabled()) { reset(); return; }
    if (rotated) transform(); // Refresh rotation offsets after responsive layout changes.
    rect ??= base();
    const area = options.bounds();
    if (cover && automaticCover && rect.width > 0 && rect.height > 0) {
      scale = Math.max(1, (area.right-area.left)/rect.width, (area.bottom-area.top)/rect.height);
      x = area.left + (area.right-area.left-rect.width*scale)/2 - rect.left;
      y = area.top + (area.bottom-area.top-rect.height*scale)/2 - rect.top;
    }
    if (scale <= 1) { const automatic = automaticCover; reset(); automaticCover = automatic; return; }
    const width = rect.width * scale, height = rect.height * scale;
    const position = (start: number, size: number, low: number, high: number) => size <= high - low
      ? low + (high - low - size) / 2 : clamp(start, high - size, low);
    const left = position(rect.left + x, width, area.left, area.right);
    const top = position(rect.top + y, height, area.top, area.bottom);
    x = left - rect.left; y = top - rect.top;
    transform();
    // clip-path uses untransformed element coordinates. Limit the picture to
    // the safe desktop area, including the space reserved above the dock.
    canvas.style.clipPath = pictureClip((area.top-top)/scale, (left+width-area.right)/scale, (top+height-area.bottom)/scale, (area.left-left)/scale, rotated);
    options.changed();
  }
  function gesture(from: PicturePoint, to: PicturePoint, ratio: number) {
    if (!options.enabled() || ![from.x, from.y, to.x, to.y, ratio].every(Number.isFinite) || ratio <= 0) return;
    const rect = base();
    if (!rect.width || !rect.height) return;
    const area = options.bounds();
    const maximum = cover ? Math.max(4, (area.right-area.left)/rect.width, (area.bottom-area.top)/rect.height) : 4;
    const next = clamp(scale * ratio, 1, maximum);
    automaticCover = false;
    const imageX = (from.x - rect.left - x) / scale;
    const imageY = (from.y - rect.top - y) / scale;
    x = to.x - rect.left - imageX * next;
    y = to.y - rect.top - imageY * next;
    scale = next;
    render(rect);
  }
  function anchor(event: { clientX?: number; clientY?: number }): PicturePoint {
    if (Number.isFinite(event.clientX) && Number.isFinite(event.clientY)) return { x: event.clientX!, y: event.clientY! };
    const area = options.bounds();
    return { x: (area.left + area.right) / 2, y: (area.top + area.bottom) / 2 };
  }
  listen(document, "wheel", (event: WheelEvent) => {
    if (!event.ctrlKey && !event.metaKey) return;
    event.preventDefault();
    event.stopPropagation();
    // Chromium trackpad pinches arrive as Ctrl+wheel. Other targets only
    // suppress browser zoom; ordinary wheel scrolling still reaches the peer.
    if (event.target !== canvas || safari || options.touching()) return;
    const units = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 100 : 1;
    const point = anchor(event);
    gesture(point, point, Math.exp(-clamp(event.deltaY * units, -100, 100) * .01));
  });
  listen(document, "keydown", (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.isComposing
      && (["Equal", "Minus", "Digit0", "NumpadAdd", "NumpadSubtract", "Numpad0"].includes(event.code)
        || ["+", "-", "=", "0"].includes(event.key))) {
      event.preventDefault(); // Keep propagation so remote applications still receive their shortcuts.
    }
  });
  for (const name of ["touchstart", "touchmove"]) {
    listen(document, name, (event: TouchEvent) => { if (event.touches.length > 1) event.preventDefault(); });
  }
  listen(document, "gesturestart", (event: Event & { scale?: number; clientX?: number; clientY?: number }) => {
    event.preventDefault();
    safari = event.target === canvas && options.enabled() && !options.touching()
      ? { scale: 1, point: anchor(event) } : null;
  });
  listen(document, "gesturechange", (event: Event & { scale?: number; clientX?: number; clientY?: number }) => {
    event.preventDefault();
    if (!safari || options.touching() || !Number.isFinite(event.scale) || event.scale! <= 0) return;
    const point = anchor(event);
    gesture(safari.point, point, event.scale! / safari.scale);
    safari = { scale: event.scale!, point };
  });
  listen(document, "gestureend", (event: Event) => { event.preventDefault(); safari = null; });
  listen(window, "resize", () => render());
  const observer = window.ResizeObserver ? new window.ResizeObserver(() => render()) : null;
  observer?.observe(canvas); observer?.observe(viewport);
  return {
    gesture, reset,
    setRotated(value: boolean) {
      if (rotated === value) return;
      rotated = value;
      viewport.classList.toggle("picture-rotated", value);
      reset(); render();
    },
    setCover(value: boolean) {
      if (cover !== value) { cover = value; reset(); }
      if (cover) render();
    },
    reveal: (point: PicturePoint) => {
      if (!options.enabled() || scale <= 1) return;
      const rect = canvas.getBoundingClientRect(), area = options.bounds();
      const from = pictureClientPoint(rect, point, rotated);
      const margin = Math.min(12, (area.right-area.left)/4, (area.bottom-area.top)/4);
      const to = { x: clamp(from.x, area.left+margin, area.right-margin), y: clamp(from.y, area.top+margin, area.bottom-margin) };
      if (from.x !== to.x || from.y !== to.y) gesture(from,to,1);
    },
    layout: () => render(), active: () => options.enabled() && scale > 1,
    dispose() { lifecycle.abort(); observer?.disconnect(); rotated = false; viewport.classList.toggle("picture-rotated", false); reset(); },
  };
}
