interface OrientationControl {
  lock?: (orientation: "portrait" | "landscape") => Promise<void>;
  unlock?: () => void;
}

/** Prefer native orientation; embedded browsers can rotate just the desktop. */
export function createScreenRotation(options: {
  active: () => boolean;
  landscape: () => boolean;
  orientation: () => OrientationControl | undefined;
  rotated: (value: boolean) => void;
  pending: (value: boolean) => void;
  release: () => void;
}) {
  let generation = 0, fallback = false, pending = false;
  let owner: OrientationControl | undefined;
  let timer: ReturnType<typeof setTimeout> | null = null;
  function unlock() { try { owner?.unlock?.(); } catch {} owner = undefined; }
  function clearTimer() { if (timer !== null) clearTimeout(timer); timer = null; }
  function reset() {
    generation++; clearTimer(); unlock();
    pending = fallback = false; options.pending(false); options.rotated(false);
  }
  function toggle() {
    if (!options.active() || pending) return;
    options.release();
    if (fallback) { reset(); return; }
    const orientation = options.orientation();
    const token = ++generation;
    const finish = (success: boolean) => {
      if (token !== generation) return;
      generation++; clearTimer(); pending = false; options.pending(false);
      if (!options.active()) { unlock(); return; }
      if (!success) { unlock(); fallback = true; options.rotated(true); }
    };
    if (!orientation?.lock) { finish(false); return; }
    owner = orientation; pending = true; options.pending(true);
    timer = setTimeout(() => finish(false), 1500);
    try { void orientation.lock(options.landscape() ? "portrait" : "landscape").then(() => finish(true), () => finish(false)); }
    catch { finish(false); }
  }
  return { toggle, reset, deviceChanged() { if (fallback) { options.release(); reset(); } }, dispose: reset };
}
