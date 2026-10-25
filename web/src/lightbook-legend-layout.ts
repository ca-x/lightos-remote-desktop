type Bounds = { left: number; top: number; right: number; bottom: number };
type TextInk = { actualBoundingBoxLeft: number; actualBoundingBoxRight: number; actualBoundingBoxAscent: number; actualBoundingBoxDescent: number };

// Font line boxes include unequal ascent/descent space. Center the visible ink
// instead, combining both legends before moving anything. Recompute only when
// the keyboard size or loaded fonts change, never on the input/media path.
export function createLightBookLegendLayout(root: HTMLElement) {
  const keyboard = root.querySelector<HTMLElement>("#lightbook-keys");
  if (!keyboard) return () => {};
  const lifecycle = new AbortController();
  const offsets = new WeakMap<HTMLElement, { x: number; y: number }>();
  const metrics = new Map<string, TextInk>();
  let context: CanvasRenderingContext2D | null | undefined;
  let frame: number | null = null, disposed = false;

  function measure() {
    if (disposed || !keyboard!.getBoundingClientRect().height) return;
    if (context === undefined) {
      try { context = document.createElement("canvas").getContext("2d", { willReadFrequently: true }); }
      catch { context = null; }
    }
    const density = Math.max(1, Math.min(4, window.devicePixelRatio || 1));
    const updates: { element: HTMLElement; x: number; y: number }[] = [];
    for (const key of keyboard!.querySelectorAll<HTMLElement>("[data-lightbook-key]")) {
      const box = key.getBoundingClientRect();
      const element = key.querySelector<HTMLElement>(".lightbook-legend");
      if (!element || !box.width || !box.height) continue;
      const parts: Bounds[] = [];
      for (const text of element.querySelectorAll<HTMLElement>(".lightbook-legend-text")) {
        const value = text.textContent ?? "";
        const rect = text.getBoundingClientRect();
        if (!value.trim() || !rect.height) continue;
        const baseline = text.querySelector<HTMLElement>(".lightbook-baseline")?.getBoundingClientRect();
        if (context && baseline) {
          const style = getComputedStyle(text);
          const font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
          // Match the actual font size and physical-pixel phase. TextMetrics
          // alone rounds small glyphs differently from their visible pixels.
          const phaseX = ((rect.left * density) % 1) / density;
          const phaseY = ((baseline.top * density) % 1) / density;
          const id = `${density}\0${font}\0${value}\0${phaseX.toFixed(4)},${phaseY.toFixed(4)}`;
          let metric = metrics.get(id);
          if (!metric) {
            context.font = font;
            context.textAlign = "left";
            context.textBaseline = "alphabetic";
            const fallback = context.measureText(value), size = parseFloat(style.fontSize);
            const padding = Math.ceil(size), originX = Math.ceil(padding * density) / density + phaseX;
            const originY = Math.ceil((padding + size * 1.5) * density) / density + phaseY;
            metric = fallback;
            try {
              context.canvas.width = Math.ceil((fallback.width + padding * 2) * density);
              context.canvas.height = Math.ceil((size * 3 + padding * 2) * density);
              context.setTransform(density, 0, 0, density, 0, 0);
              context.font = font; context.textAlign = "left"; context.textBaseline = "alphabetic"; context.fillStyle = "#fff";
              context.fillText(value, originX, originY);
              const { data, width, height } = context.getImageData(0, 0, context.canvas.width, context.canvas.height);
              let left = width, top = height, right = 0, bottom = 0;
              for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
                if (data[(y * width + x) * 4 + 3] < 112) continue;
                left = Math.min(left, x); right = Math.max(right, x + 1);
                top = Math.min(top, y); bottom = Math.max(bottom, y + 1);
              }
              if (right > left) metric = {
                actualBoundingBoxLeft: originX - left / density, actualBoundingBoxRight: right / density - originX,
                actualBoundingBoxAscent: originY - top / density, actualBoundingBoxDescent: bottom / density - originY,
              };
            } catch {
              // Keep the metrics fallback if pixel readback is unavailable.
            }
            if (metrics.size >= 512) metrics.clear();
            metrics.set(id, metric);
          }
          const bounds = {
            left: rect.left - metric.actualBoundingBoxLeft,
            right: rect.left + metric.actualBoundingBoxRight,
            top: baseline.top - metric.actualBoundingBoxAscent,
            bottom: baseline.top + metric.actualBoundingBoxDescent,
          };
          if (Object.values(bounds).every(Number.isFinite)) { parts.push(bounds); continue; }
        }
        parts.push(rect);
      }
      for (const svg of element.querySelectorAll<SVGSVGElement>("svg")) {
        if (!svg.getBoundingClientRect().height) continue;
        for (const shape of svg.querySelectorAll<SVGGraphicsElement>("path, circle, line, polyline, polygon, rect, ellipse")) {
          const matrix = shape.getScreenCTM();
          if (!matrix) continue;
          const bounds = shape.getBBox();
          const points = [
            new DOMPoint(bounds.x, bounds.y), new DOMPoint(bounds.x + bounds.width, bounds.y),
            new DOMPoint(bounds.x, bounds.y + bounds.height), new DOMPoint(bounds.x + bounds.width, bounds.y + bounds.height),
          ].map(point => point.matrixTransform(matrix));
          const style = getComputedStyle(shape);
          const stroke = style.stroke === "none" ? 0 : (parseFloat(style.strokeWidth) || 0) / 2;
          const dx = stroke * Math.hypot(matrix.a, matrix.c), dy = stroke * Math.hypot(matrix.b, matrix.d);
          parts.push({ left: Math.min(...points.map(p => p.x)) - dx, right: Math.max(...points.map(p => p.x)) + dx,
            top: Math.min(...points.map(p => p.y)) - dy, bottom: Math.max(...points.map(p => p.y)) + dy });
        }
      }
      if (!parts.length) continue;
      const old = offsets.get(element) ?? { x: 0, y: 0 };
      // Whole physical-pixel translations keep the measured raster phase.
      const x = old.x + Math.round((box.left + box.right - Math.min(...parts.map(p => p.left)) - Math.max(...parts.map(p => p.right))) / 2 * density) / density;
      const y = old.y + Math.round((box.top + box.bottom - Math.min(...parts.map(p => p.top)) - Math.max(...parts.map(p => p.bottom))) / 2 * density) / density;
      if (Math.abs(x - old.x) > .01 || Math.abs(y - old.y) > .01) updates.push({ element, x, y });
    }
    // Batch writes after all geometry reads; transforms do not resize the keys.
    for (const { element, x, y } of updates) {
      offsets.set(element, { x, y });
      element.style.setProperty("--lightbook-legend-x", `${x}px`);
      element.style.setProperty("--lightbook-legend-y", `${y}px`);
    }
  }
  function schedule() {
    if (!disposed && frame === null) frame = requestAnimationFrame(() => { frame = null; measure(); });
  }
  function fontsChanged() { metrics.clear(); schedule(); }
  const observer = window.ResizeObserver ? new window.ResizeObserver(measure) : null;
  observer?.observe(keyboard);
  window.addEventListener("resize", schedule, { signal: lifecycle.signal });
  document.fonts?.addEventListener("loadingdone", fontsChanged, { signal: lifecycle.signal });
  void document.fonts?.ready.then(fontsChanged);
  measure();
  return () => {
    disposed = true;
    observer?.disconnect(); lifecycle.abort();
    if (frame !== null) cancelAnimationFrame(frame);
    for (const element of keyboard.querySelectorAll<HTMLElement>(".lightbook-legend")) {
      element.style.removeProperty("--lightbook-legend-x"); element.style.removeProperty("--lightbook-legend-y");
    }
  };
}
