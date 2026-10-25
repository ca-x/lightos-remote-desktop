interface CursorState {
  type: "cursor"; visible: boolean; id: string; x: number; y: number;
  width: number; height: number; display_width: number; display_height: number;
  hotspot_x: number; hotspot_y: number; units_width: number; units_height: number;
  png?: string;
}
export function createCursorLayer(canvas: HTMLCanvasElement, overlay: HTMLImageElement, rtt: () => number = () => 0, clipBounds?: () => { left: number; top: number; right: number; bottom: number }) {
  let socket: WebSocket | null = null, generation = 0, imageRevision = 0;
  let embedded = true, shape: CursorState | null = null, latest: CursorState | null = null;
  let url: string | null = null, inside = false, touch = false, corrected = false;
  let point: { x: number; y: number } | null = null, localAt = -Infinity;
  let bounds = canvas.getBoundingClientRect();
  const listeners = new AbortController();
  function render() {
    overlay.hidden = true;
    if (embedded) { canvas.style.setProperty("cursor", "none"); return; }
    if (!latest || !shape || !url) { canvas.style.setProperty("cursor", latest?.visible === false ? "none" : "default"); return; }
    if (!latest.visible) { canvas.style.setProperty("cursor", "none"); return; }
    const sx = bounds.width / shape.units_width, sy = bounds.height / shape.units_height;
    const width = shape.display_width * sx, height = shape.display_height * sy;
    const hx = shape.hotspot_x * sx, hy = shape.hotspot_y * sy;
    const density = shape.width / width;
    const cursor = `image-set(url("${url}") ${density.toFixed(5)}x) ${hx.toFixed(2)} ${hy.toFixed(2)}, default`;
    const native = !touch && !corrected && width > 0 && height > 0 && width <= 128 && height <= 128
      && shape.width <= 128 && shape.height <= 128 && hx >= 0 && hy >= 0 && hx < width && hy < height
      && Math.abs(density - shape.height / height) < 0.01 && window.CSS?.supports("cursor", cursor);
    canvas.style.setProperty("cursor", native ? cursor : "none");
    if (native && inside) return;
    const position = point && (touch ? performance.now() - localAt < Math.max(150, rtt() * 2) : inside && !corrected) ? point : latest;
    overlay.src = url;
    overlay.style.width = `${width}px`; overlay.style.height = `${height}px`;
    const left = bounds.left + position.x * bounds.width - hx, top = bounds.top + position.y * bounds.height - hy;
    overlay.style.transform = `translate3d(${left}px, ${top}px, 0)`;
    const clip = clipBounds?.();
    overlay.style.clipPath = clip ? `inset(${Math.max(0, clip.top - top)}px ${Math.max(0, left + width - clip.right)}px ${Math.max(0, top + height - clip.bottom)}px ${Math.max(0, clip.left - left)}px)` : "";
    overlay.hidden = false;
  }
  function layout() { bounds = canvas.getBoundingClientRect(); render(); }
  function move(x: number, y: number, pointerType = "mouse") {
    localAt = performance.now(); touch = pointerType === "touch"; corrected = false;
    inside = x >= bounds.left && y >= bounds.top && x <= bounds.right && y <= bounds.bottom;
    point = { x: Math.max(0, Math.min(1, (x - bounds.left) / bounds.width)), y: Math.max(0, Math.min(1, (y - bounds.top) / bounds.height)) };
    render();
  }
  function valid(value: CursorState) {
    return typeof value.id === "string" && value.id.length <= 32 && typeof value.visible === "boolean"
      && [value.x,value.y,value.display_width,value.display_height,value.hotspot_x,value.hotspot_y,value.units_width,value.units_height].every(Number.isFinite)
      && value.units_width > 0 && value.units_height > 0 && value.units_width <= 100000 && value.units_height <= 100000
      && Math.abs(value.x) <= 100 && Math.abs(value.y) <= 100 && Math.abs(value.hotspot_x) <= 4096 && Math.abs(value.hotspot_y) <= 4096
      && value.display_width >= 0 && value.display_height >= 0 && value.display_width <= 4096 && value.display_height <= 4096
      && Number.isInteger(value.width) && Number.isInteger(value.height) && value.width >= 0 && value.height >= 0 && value.width <= 512 && value.height <= 512;
  }
  async function receive(value: CursorState, session: number) {
    if (session !== generation || !valid(value)) return;
    latest = value;
    if (point && !touch && performance.now() - localAt > Math.max(150, Math.min(1500, rtt() * 2))) {
      corrected = Math.abs((value.x - point.x) * bounds.width) > 3 || Math.abs((value.y - point.y) * bounds.height) > 3;
    }
    if (value.png !== undefined) {
      if (typeof value.png !== "string" || value.png.length > 1500000 || !/^[A-Za-z0-9+/]*={0,2}$/.test(value.png) || !value.width || !value.height) return;
      const revision = ++imageRevision;
      let candidate: string | null = null;
      try {
        const bytes = Uint8Array.from(atob(value.png), character => character.charCodeAt(0));
        if (bytes[0] !== 137 || bytes[1] !== 80 || bytes[2] !== 78 || bytes[3] !== 71) return;
        candidate = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
        const image = new Image(); image.src = candidate;
        await image.decode();
        if (session !== generation || revision !== imageRevision || image.naturalWidth !== value.width || image.naturalHeight !== value.height) return;
        const previous = url;
        url = candidate; candidate = null;
        shape = latest?.id === value.id ? latest : value;
        if (previous) URL.revokeObjectURL(previous);
      } finally { if (candidate) URL.revokeObjectURL(candidate); }
    } else if (shape?.id === value.id) shape = value;
    render();
  }
  function reset() {
    generation++; imageRevision++;
    if (socket) { socket.onmessage = socket.onclose = socket.onerror = null; socket.close(); socket = null; }
    if (url) URL.revokeObjectURL(url);
    url = null; shape = latest = null; embedded = true; inside = false; touch = false; corrected = false; point = null;
    overlay.hidden = true; overlay.removeAttribute("src"); canvas.style.setProperty("cursor", "none");
  }
  function connect(token: string) {
    reset();
    const session = generation;
    const endpoint = new URL("./cursor", location.href); endpoint.protocol = location.protocol === "https:" ? "wss:" : "ws:"; endpoint.searchParams.set("session", token);
    const current = new WebSocket(endpoint); socket = current;
    current.onmessage = event => {
      if (session !== generation || typeof event.data !== "string" || event.data.length > 1600000) return;
      try { const value = JSON.parse(event.data); if (value.type === "cursor") void receive(value, session).catch(() => { if (socket === current) current.close(); }); } catch {}
    };
    current.onerror = () => { if (socket === current) current.close(); };
    current.onclose = () => { if (socket === current) socket = null; };
  }
  canvas.addEventListener("pointerenter", event => { layout(); move(event.clientX,event.clientY,event.pointerType); }, { signal: listeners.signal });
  canvas.addEventListener("pointerleave", () => { inside = false; render(); }, { signal: listeners.signal });
  window.addEventListener("resize", layout, { signal: listeners.signal });
  window.visualViewport?.addEventListener("resize", layout, { signal: listeners.signal });
  window.visualViewport?.addEventListener("scroll", layout, { signal: listeners.signal });
  const observer = window.ResizeObserver ? new window.ResizeObserver(layout) : null; observer?.observe(canvas);
  return {
    connect, reset, layout, move,
    position: () => {
      const recent = point && performance.now() - localAt < Math.max(150, rtt() * 2);
      const current = recent ? point : latest ?? point;
      return { x: Math.max(0, Math.min(1, current?.x ?? .5)), y: Math.max(0, Math.min(1, current?.y ?? .5)) };
    },
    moveNormalized: (x: number, y: number) => move(bounds.left + x * bounds.width, bounds.top + y * bounds.height, "touch"),
    present: (value: boolean) => { embedded = value; if (!embedded && !touch) inside = canvas.matches?.(":hover") ?? inside; render(); },
    dispose: () => { reset(); listeners.abort(); observer?.disconnect(); },
  };
}
