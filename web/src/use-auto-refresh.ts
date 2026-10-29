import { useEffect, useLayoutEffect, useRef } from "react";

/** Refresh only visible settings, without restarting the timer on each render. */
export function useAutoRefresh(active: boolean, refresh: () => Promise<unknown>) {
  const current = useRef(refresh);
  useLayoutEffect(() => { current.current = refresh; });
  useEffect(() => {
    if (!active) return;
    const update = () => { if (!document.hidden) void current.current(); };
    update();
    const timer = window.setInterval(update, 15000);
    document.addEventListener("visibilitychange", update);
    window.addEventListener("focus", update);
    window.addEventListener("online", update);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", update);
      window.removeEventListener("focus", update);
      window.removeEventListener("online", update);
    };
  }, [active]);
}
