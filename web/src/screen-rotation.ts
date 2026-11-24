interface OrientationControl {
  lock?: (orientation: "portrait" | "landscape") => Promise<void>;
  unlock?: () => void;
}

type OrientationHost = Window & {
  android?: { SetScreenOrientation?: (orientation: number) => void };
  android_window?: { SetScreenOrientation?: (orientation: number) => void };
  webkit?: { messageHandlers?: { SetScreenOrientation?: { postMessage: (message: { params: number[] }) => unknown } } };
  LzcClientBridge?: {
    has?: (method: string) => boolean;
    call?: (method: string, args: number[]) => Promise<unknown>;
  };
};

export function mobileOrientationControl(host: OrientationHost): OrientationControl | undefined {
  const browser = host.screen?.orientation as OrientationControl | undefined;
  const android = host.android_window ?? host.android;
  const bridge = host.LzcClientBridge;
  const legacyIOS = host.webkit?.messageHandlers?.SetScreenOrientation;
  const method = "SetScreenOrientation";
  const native = typeof android?.SetScreenOrientation === "function"
    ? (value: number) => android.SetScreenOrientation!(value)
    : typeof bridge?.call === "function" && (bridge.has?.(method) ?? true)
      ? (value: number) => bridge.call!(method, [value])
      : legacyIOS
        ? (value: number) => legacyIOS.postMessage({ params: [value] })
        : undefined;
  if (!native) return browser;
  let owner: "native" | "browser" | undefined;
  return {
    async lock(orientation) {
      try {
        owner = "native";
        const result: unknown = await native(orientation === "landscape" ? 2 : 3);
        if (result === false) throw new Error("Native rotation denied");
      } catch (error) {
        owner = undefined;
        if (!browser?.lock) throw error;
        owner = "browser";
        await browser.lock(orientation);
      }
    },
    unlock() {
      if (owner === "native") void Promise.resolve(native(1)).catch(() => {});
      else if (owner === "browser") browser?.unlock?.();
      owner = undefined;
    },
  };
}

/** Keep the landscape view usable while the host changes orientation. */
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
