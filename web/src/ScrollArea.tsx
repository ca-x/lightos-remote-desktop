import { useLayoutEffect, useRef, useState, type TextareaHTMLAttributes, type ReactNode, type RefObject } from "react";
import { Textarea } from "@headlessui/react";

type Axis = "x" | "y";
type Metrics = { left: number; top: number; width: number; height: number; x: number; y: number; maxX: number; maxY: number; thumbX: number; thumbY: number; trackX: number; trackY: number };
const empty: Metrics = { left: 0, top: 0, width: 0, height: 0, x: 0, y: 0, maxX: 0, maxY: 0, thumbX: 0, thumbY: 0, trackX: 0, trackY: 0 };

/** Paint scrollbars beside the actual scroller, preserving native scrolling. */
export function Scrollbars({ target, axis = "both" }: { target: RefObject<HTMLElement | null>; axis?: Axis | "both" }) {
  const overlay = useRef<HTMLDivElement>(null);
  const metrics = useRef(empty);
  const refresh = useRef(() => {});
  const drag = useRef<{ id: number; axis: Axis; grab: number } | null>(null);
  const hovering = useRef(false);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [layout, setLayout] = useState(empty);
  const [visible, setVisible] = useState(false);
  const [dragging, setDragging] = useState<Axis | null>(null);
  const reveal = () => {
    clearTimeout(hideTimer.current);
    setVisible(true);
    hideTimer.current = setTimeout(() => { if (!drag.current && !hovering.current) setVisible(false); }, 1000);
  };

  useLayoutEffect(() => {
    const host = target.current, root = overlay.current;
    if (!host || !root) return;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const parent = root.parentElement!;
      const rect = host.getBoundingClientRect(), parentRect = parent.getBoundingClientRect();
      const scaleX = parent.clientWidth ? parentRect.width / parent.clientWidth : 1;
      const scaleY = parent.clientHeight ? parentRect.height / parent.clientHeight : 1;
      const style = getComputedStyle(host);
      const maxX = axis !== "y" && /auto|scroll/.test(style.overflowX) ? Math.max(0, host.scrollWidth - host.clientWidth) : 0;
      const maxY = axis !== "x" && /auto|scroll/.test(style.overflowY) ? Math.max(0, host.scrollHeight - host.clientHeight) : 0;
      const width = host.offsetWidth, height = host.offsetHeight;
      const trackX = Math.max(0, width - 8 - (maxY > 1 ? 12 : 0));
      const trackY = Math.max(0, height - 8 - (maxX > 1 ? 12 : 0));
      const next: Metrics = {
        left: scaleX ? (rect.left - parentRect.left) / scaleX : 0,
        top: scaleY ? (rect.top - parentRect.top) / scaleY : 0,
        width, height, x: host.scrollLeft, y: host.scrollTop, maxX, maxY, trackX, trackY,
        thumbX: Math.min(trackX, Math.max(28, trackX * host.clientWidth / Math.max(1, host.scrollWidth))),
        thumbY: Math.min(trackY, Math.max(28, trackY * host.clientHeight / Math.max(1, host.scrollHeight))),
      };
      metrics.current = next;
      setLayout((current) => (Object.keys(next) as (keyof Metrics)[]).every(key => current[key] === next[key]) ? current : next);
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    refresh.current = schedule;
    const resize = window.ResizeObserver ? new ResizeObserver(schedule) : null;
    const observeChildren = () => { resize?.disconnect(); resize?.observe(host); for (const child of host.children) resize?.observe(child); schedule(); };
    const mutation = new MutationObserver(observeChildren);
    mutation.observe(host, { childList: true });
    observeChildren();
    const scroll = () => { schedule(); reveal(); };
    const pointer = (event: PointerEvent) => {
      const rect = host.getBoundingClientRect(), current = metrics.current;
      if (current.maxY > 1 && event.clientX >= rect.right - 16 || current.maxX > 1 && event.clientY >= rect.bottom - 16) reveal();
    };
    const wheel = (event: WheelEvent) => {
      const track = (event.target as Element).closest<HTMLElement>(".scrollbar-track");
      if (!track) return;
      event.preventDefault(); event.stopPropagation();
      const factor = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? host.clientHeight : 1;
      if (track.dataset.axis === "x") host.scrollLeft += (event.deltaX || event.deltaY) * factor;
      else host.scrollTop += event.deltaY * factor;
      scroll();
    };
    host.addEventListener("scroll", scroll, { passive: true });
    host.addEventListener("input", schedule);
    host.addEventListener("pointermove", pointer, { passive: true });
    root.addEventListener("wheel", wheel, { passive: false });
    window.addEventListener("resize", schedule);
    window.visualViewport?.addEventListener("resize", schedule);
    schedule();
    return () => {
      refresh.current = () => {};
      cancelAnimationFrame(frame); clearTimeout(hideTimer.current);
      resize?.disconnect(); mutation.disconnect();
      host.removeEventListener("scroll", scroll); host.removeEventListener("input", schedule); host.removeEventListener("pointermove", pointer);
      root.removeEventListener("wheel", wheel); window.removeEventListener("resize", schedule); window.visualViewport?.removeEventListener("resize", schedule);
    };
  }, [target, axis]);
  // Native textarea values and client-owned controls may change without a DOM mutation.
  useLayoutEffect(() => { refresh.current(); });

  function move(track: HTMLDivElement, coordinate: number, grab: number, axis: Axis) {
    const host = target.current;
    if (!host) return;
    const rect = track.getBoundingClientRect(), current = metrics.current;
    const physical = axis === "x" ? rect.width : rect.height;
    const logical = axis === "x" ? current.trackX : current.trackY;
    const thumb = axis === "x" ? current.thumbX : current.thumbY;
    const position = coordinate - (axis === "x" ? rect.left : rect.top) - grab;
    const ratio = position / Math.max(1, physical * (1 - thumb / Math.max(1, logical)));
    if (axis === "x") host.scrollLeft = Math.max(0, Math.min(1, ratio)) * current.maxX;
    else host.scrollTop = Math.max(0, Math.min(1, ratio)) * current.maxY;
    refresh.current(); reveal();
  }
  function track(axis: Axis) {
    const horizontal = axis === "x", maximum = horizontal ? layout.maxX : layout.maxY;
    const length = horizontal ? layout.thumbX : layout.thumbY, available = (horizontal ? layout.trackX : layout.trackY) - length;
    const offset = maximum ? Math.max(0, Math.min(maximum, horizontal ? layout.x : layout.y)) / maximum * available : 0;
    return <div key={axis} data-axis={axis} hidden={maximum <= 1 || !layout.width || !layout.height}
      className={`scrollbar-track scrollbar-track-${axis}${visible ? " is-visible" : ""}${dragging === axis ? " is-dragging" : ""}`}
      style={horizontal ? { right: layout.maxY > 1 ? 16 : 4 } : { bottom: layout.maxX > 1 ? 16 : 4 }}
      onPointerEnter={() => { hovering.current = true; reveal(); }} onPointerLeave={() => { hovering.current = false; reveal(); }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault(); event.stopPropagation();
        const thumb = event.currentTarget.firstElementChild!.getBoundingClientRect();
        const coordinate = horizontal ? event.clientX : event.clientY;
        const onThumb = (event.target as Element).closest(".scrollbar-thumb");
        const grab = onThumb ? coordinate - (horizontal ? thumb.left : thumb.top) : (horizontal ? thumb.width : thumb.height) / 2;
        drag.current = { id: event.pointerId, axis, grab }; setDragging(axis);
        event.currentTarget.setPointerCapture(event.pointerId); move(event.currentTarget, coordinate, grab, axis);
      }}
      onPointerMove={(event) => { const current = drag.current; if (current?.id === event.pointerId) move(event.currentTarget, horizontal ? event.clientX : event.clientY, current.grab, axis); }}
      onPointerUp={(event) => { if (drag.current?.id === event.pointerId) { drag.current = null; setDragging(null); event.currentTarget.releasePointerCapture(event.pointerId); reveal(); } }}
      onPointerCancel={() => { drag.current = null; setDragging(null); reveal(); }}
      onLostPointerCapture={() => { drag.current = null; setDragging(null); reveal(); }}>
      <div className="scrollbar-thumb" style={horizontal ? { width: length, transform: `translateX(${offset}px)` } : { height: length, transform: `translateY(${offset}px)` }} />
    </div>;
  }
  return <div ref={overlay} className="custom-scrollbars" aria-hidden="true" style={{ left: layout.left, top: layout.top, width: layout.width, height: layout.height }}>{track("x")}{track("y")}</div>;
}

export function ScrollArea({ children, className = "", contentClassName = "", axis = "y" }: { children: ReactNode; className?: string; contentClassName?: string; axis?: Axis | "both" }) {
  const viewport = useRef<HTMLDivElement>(null);
  return <div className={`scroll-area ${className}`}><div ref={viewport} className="scroll-viewport" data-scroll-axis={axis}><div className={`scroll-content ${contentClassName}`}>{children}</div></div><Scrollbars target={viewport} axis={axis} /></div>;
}

export function ScrollTextarea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const textarea = useRef<HTMLTextAreaElement>(null);
  return <div className="scroll-area textarea-scroll-area"><Textarea {...props} ref={textarea} /><Scrollbars target={textarea} /></div>;
}
