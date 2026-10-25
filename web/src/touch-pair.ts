export interface TouchPairSample { x: number; y: number; distance: number }

/** Atomic two-finger samples, including when a browser cancels Pointer Events
 * after handing a multi-touch gesture to its native recognizer. */
export function createTouchPair(target: HTMLElement, options: {
  enabled: () => boolean;
  start: (sample: TouchPairSample | null, first: boolean) => void;
  move: (sample: TouchPairSample) => void;
  end: (cancelled: boolean) => void;
}) {
  const lifecycle = new AbortController();
  let active = false, count = 0;
  let frame: number | null = null, pending: TouchPairSample | null = null;
  const sample = (touches: TouchList): TouchPairSample | null => {
    if (touches.length !== 2) return null;
    const [a, b] = Array.from(touches);
    return { x: (a.clientX+b.clientX)/2, y: (a.clientY+b.clientY)/2, distance: Math.hypot(a.clientX-b.clientX, a.clientY-b.clientY) };
  };
  function flush() {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    const value = pending; pending = null;
    if (value && active && options.enabled()) options.move(value);
  }
  function reset() {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null; pending = null; active = false; count = 0;
  }
  function finish(cancelled: boolean) {
    if (!active) return;
    if (!cancelled) flush();
    reset(); options.end(cancelled);
  }
  const listen = (name: string, handler: (event: TouchEvent) => void) => target.addEventListener(name, handler as EventListener, { signal: lifecycle.signal, passive: false });
  listen("touchstart", event => {
    if (!options.enabled()) { finish(true); return; }
    if (event.targetTouches.length < 2 && !active) return;
    event.preventDefault(); flush();
    const first = !active; active = true; count = event.targetTouches.length;
    options.start(sample(event.targetTouches), first);
  });
  listen("touchmove", event => {
    if (!active) return;
    event.preventDefault();
    if (!options.enabled()) { finish(true); return; }
    const value = sample(event.targetTouches);
    if (value && count === 2) {
      pending = value;
      if (frame === null) frame = requestAnimationFrame(flush);
    }
  });
  listen("touchend", event => {
    if (!active) return;
    event.preventDefault(); flush();
    const previous = count; count = event.targetTouches.length;
    if (!count) finish(false);
    else if (count === 2 && previous !== 2) options.start(sample(event.targetTouches), false);
  });
  listen("touchcancel", () => finish(true));
  return { active: () => active, reset, dispose() { reset(); lifecycle.abort(); } };
}
