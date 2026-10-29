import { pictureScrollDelta } from "./picture-geometry";
import { createTouchPair, type TouchPairSample } from "./touch-pair";
import { captureResolution, LEGACY_SCALES } from "./resolution";
import { isMobilePlatform } from "./platform";

export interface MobileShortcut {
  label: string;
  key?: string;
  modifiers?: string[];
  modifier?: boolean;
  text?: string;
  action?: "menu";
}

export const MOBILE_ROWS: readonly (readonly MobileShortcut[])[] = [
  [
    { label: "Ctrl", key: "ControlLeft", modifier: true },
    { label: "Alt", key: "AltLeft", modifier: true },
    { label: "Shift", key: "ShiftLeft", modifier: true },
    { label: "Super", key: "MetaLeft", modifier: true },
    { label: "Tab", key: "Tab" }, { label: "Esc", key: "Escape" },
    { label: "↑", key: "ArrowUp" }, { label: "↓", key: "ArrowDown" },
    { label: "←", key: "ArrowLeft" }, { label: "→", key: "ArrowRight" },
    { label: "Enter", key: "Enter" },
  ],
  [
    { label: "Ctrl+C", key: "KeyC", modifiers: ["ControlLeft"] },
    { label: "Ctrl+V", key: "KeyV", modifiers: ["ControlLeft"] },
    { label: "Ctrl+A", key: "KeyA", modifiers: ["ControlLeft"] },
    { label: "Home", key: "Home" }, { label: "End", key: "End" },
    { label: "PgUp", key: "PageUp" }, { label: "PgDn", key: "PageDown" },
    { label: "⌫", key: "Backspace" }, { label: "Del", key: "Delete" },
    { label: "/", text: "/" }, { label: "-", text: "-" },
    { label: "~", text: "~" }, { label: "$", text: "$" },
  ],
];

// More capture pixels must be paired with compositor scale: increasing only
// the resolution makes desktop text smaller instead of making it sharper.
export function mobileResolution(cssWidth: number, cssHeight: number, pixelRatio: number) {
  return captureResolution(cssWidth, cssHeight, pixelRatio, LEGACY_SCALES);
}

interface MobileOptions {
  canvas: HTMLCanvasElement;
  viewport: HTMLElement;
  input: HTMLTextAreaElement;
  bar: HTMLElement;
  toggle: HTMLButtonElement;
  fullscreenExit?: HTMLButtonElement;
  fullscreenRotate?: HTMLButtonElement;
  rotateScreen?: () => void;
  toggleFullscreen?: () => void;
  toggleAudio?: () => void;
  audioAvailable?: () => boolean;
  rotated?: () => boolean;
  exitFullscreen?: () => void;
  ready: () => boolean;
  lightBook?: () => boolean;
  send: (command: Record<string, unknown>) => boolean;
  chord: (codes: string[]) => void;
  text: (text: string) => void;
  move: (point: { clientX: number; clientY: number }) => void;
  tools: (open: boolean) => void;
  isToolsOpen: () => boolean;
  layout: () => void;
  translate: (text: string) => string;
  zoom?: {
    active: () => boolean;
    gesture: (from: { x: number; y: number }, to: { x: number; y: number }, ratio: number) => void;
  };
  hardwareKey?: (event: KeyboardEvent) => boolean;
  hardwareInput?: (event: InputEvent) => boolean;
  composition?: () => void;
  paste?: (event: ClipboardEvent) => void;
}

const SENTINEL = "\u200b";
const MODIFIERS = new Set(["ControlLeft", "AltLeft", "ShiftLeft", "MetaLeft"]);
const REPEAT = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Backspace", "Delete", "Enter"]);
const SYMBOL_KEYS: Record<string, [string, boolean]> = {};
for (const [code, plain, shifted] of [
  ["Backquote", "`", "~"], ["Minus", "-", "_"], ["Equal", "=", "+"],
  ["BracketLeft", "[", "{"], ["BracketRight", "]", "}"], ["Backslash", "\\", "|"],
  ["Semicolon", ";", ":"], ["Quote", "'", "\""], ["Comma", ",", "<"],
  ["Period", ".", ">"], ["Slash", "/", "?"], ["Space", " ", " "],
  ...Array.from("1234567890", (digit, index) => [`Digit${digit}`, digit, "!@#$%^&*()"[index]]),
]) {
  SYMBOL_KEYS[plain] = [code, false];
  SYMBOL_KEYS[shifted] = [code, plain !== shifted];
}

export function createMobileControls(options: MobileOptions) {
  const { canvas, viewport, input, bar, toggle } = options;
  const floating = viewport.querySelector?.<HTMLButtonElement>("#tools-toggle");
  const lifecycle = new AbortController();
  const coarse = window.matchMedia?.("(any-pointer: coarse)");
  let touch = isMobilePlatform();
  let disposed = false;
  let fullscreen = false;
  let fullscreenTimer: ReturnType<typeof setTimeout> | null = null;
  let composing = false;
  let committed = "";
  let physicalInput = false;
  let frame: number | null = null;
  let closingTimer: ReturnType<typeof setTimeout> | null = null;
  let keyboardReference = 0;
  let keyboardWasVisible = false;
  const sticky = new Set<string>();
  const keyButtons = Array.from(bar.querySelectorAll?.<HTMLButtonElement>("[data-mobile-key]") ?? []);
  const buttons = Array.from(bar.querySelectorAll?.<HTMLButtonElement>("button") ?? []);
  const keyboardButtons = [toggle, ...Array.from(bar.querySelectorAll?.<HTMLButtonElement>('[data-mobile-action="keyboard"]') ?? [])];
  const activeTouches = new Map<number, { x: number; y: number; startX: number; startY: number }>();
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let dragging = false;
  let rightClicked = false;
  let multiTouch = false;
  let centroid: { x: number; y: number; distance: number } | null = null;
  let gestureFrame: number | null = null;
  let gestureOrigin: { x: number; y: number; distance: number } | null = null;
  let gestureMode: "pending" | "scroll" | "zoom" | "pan" = "pending";
  let shortcutPointer: { id: number; button: HTMLButtonElement; x: number; y: number; moved: boolean; repeated: boolean } | null = null;
  let repeatTimer: ReturnType<typeof setTimeout> | null = null;
  let suppressClick: HTMLButtonElement | null = null;

  const listen = (target: EventTarget | null | undefined, name: string, handler: (event: any) => void, extra = {}) => {
    target?.addEventListener(name, handler, { signal: lifecycle.signal, ...extra });
  };
  const cancelHold = () => { if (holdTimer !== null) clearTimeout(holdTimer); holdTimer = null; };
  const cancelRepeat = () => { if (repeatTimer !== null) clearTimeout(repeatTimer); repeatTimer = null; };
  const focused = () => document.activeElement === input;
  const resetField = () => { input.value = SENTINEL; input.setSelectionRange?.(1, 1); };

  function hideFullscreenExit() {
    if (fullscreenTimer !== null) clearTimeout(fullscreenTimer);
    fullscreenTimer = null;
    if (options.fullscreenExit) options.fullscreenExit.hidden = true;
    if (options.fullscreenRotate) options.fullscreenRotate.hidden = true;
  }
  function revealFullscreenExit() {
    if (!touch || !fullscreen || disposed || !options.fullscreenExit) return;
    hideFullscreenExit();
    options.fullscreenExit.hidden = false;
    if (options.fullscreenRotate) options.fullscreenRotate.hidden = false;
    fullscreenTimer = setTimeout(hideFullscreenExit, 2000);
  }
  function setFullscreen(value: boolean) {
    if (fullscreen === value) return;
    fullscreen = value;
    if (value && touch) {
      reset();
      if (closingTimer !== null) clearTimeout(closingTimer);
      closingTimer = null; keyboardWasVisible = false;
      revealFullscreenExit();
    } else hideFullscreenExit();
    update();
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null; layout();
  }

  function update() {
    bar.hidden = !touch || (fullscreen && !focused() && options.ready());
    viewport.classList.toggle("touch-desktop", touch);
    viewport.classList.toggle("mobile-fullscreen", touch && fullscreen);
    viewport.classList.toggle("software-keyboard-open", focused() || closingTimer !== null);
    // Retain Headless UI's programmatic focus anchor for its popover while
    // removing the invisible floating entry from navigation and accessibility.
    if (floating) {
      floating.tabIndex = touch ? -1 : 0;
      floating.setAttribute("aria-hidden", String(touch));
    }
    for (const button of buttons) {
      const action = button.dataset.mobileAction;
      button.disabled = !["menu", "fullscreen"].includes(action ?? "") && (!options.ready() || (action === "audio" && !options.audioAvailable?.()));
    }
    input.disabled = !options.ready();
    toggle.disabled = !options.ready();
    for (const button of keyboardButtons) {
      button.disabled = !options.ready();
      button.setAttribute("aria-pressed", String(focused()));
      for (const attribute of ["title", "aria-label"]) button.setAttribute(attribute, options.translate(focused() ? "Hide keyboard" : "Show keyboard"));
    }
    for (const button of keyButtons) {
      if (MODIFIERS.has(button.dataset.mobileKey!)) button.setAttribute("aria-pressed", String(sticky.has(button.dataset.mobileKey!)));
    }
  }

  function geometry() {
    const visual = window.visualViewport;
    return {
      width: visual?.width ?? window.innerWidth,
      height: visual?.height ?? window.innerHeight,
      top: visual?.offsetTop ?? 0,
      left: visual?.offsetLeft ?? 0,
    };
  }

  function layout() {
    frame = null;
    if (!touch || disposed) return;
    const bounds = geometry();
    if (focused()) {
      if (bounds.height < keyboardReference - 80) keyboardWasVisible = true;
      else if (keyboardWasVisible && bounds.height >= keyboardReference - 40) {
        hideKeyboard(); // Android Back can dismiss the IME without blurring its input.
        return;
      }
    }
    const inset = Math.max(0, window.innerHeight - bounds.height - bounds.top);
    viewport.style.setProperty("--mobile-height", `${bounds.height}px`);
    viewport.style.setProperty("--mobile-width", `${bounds.width}px`);
    viewport.style.setProperty("--mobile-top", `${bounds.top}px`);
    viewport.style.setProperty("--mobile-left", `${bounds.left}px`);
    viewport.style.setProperty("--dock-width", `${bounds.width}px`);
    viewport.style.setProperty("--dock-left", `${bounds.left}px`);
    viewport.style.setProperty("--dock-top", `${bounds.top}px`);
    viewport.style.setProperty("--visible-height", `${bounds.height}px`);
    viewport.style.setProperty("--keyboard-inset", `${inset}px`);
    viewport.classList.toggle("software-keyboard-open", focused() || closingTimer !== null);
    const keyboardVisible = inset > 80 || ((focused() || closingTimer !== null) && bounds.height < keyboardReference - 80);
    viewport.classList.toggle("software-keyboard-visible", keyboardVisible);
    viewport.style.setProperty("--mobile-bar-height", `${bar.hidden ? 0 : bar.getBoundingClientRect().height}px`);
    options.layout();
  }

  function contentBounds() {
    const bounds = geometry();
    const style = window.getComputedStyle(viewport);
    const top = fullscreen ? 0 : parseFloat(style.paddingTop) || 0;
    const left = fullscreen ? 0 : parseFloat(style.paddingLeft) || 0;
    const right = fullscreen ? 0 : parseFloat(style.paddingRight) || 0;
    return {
      top: bounds.top + top, left: bounds.left + left,
      width: Math.max(0, bounds.width - left - right), height: Math.max(0, bounds.height - top),
      bar: bar.hidden ? 0 : bar.getBoundingClientRect().height,
    };
  }

  function requestLayout() {
    if (touch && frame === null && !disposed) frame = requestAnimationFrame(layout);
  }

  function hideKeyboard() {
    if (focused()) input.blur?.();
    composing = false;
    committed = "";
    resetField();
    update();
    requestLayout();
  }

  function showKeyboard() {
    if (!options.ready()) return;
    touch = isMobilePlatform();
    update();
    options.tools(false);
    if (options.lightBook?.()) options.composition?.();
    resetField();
    input.focus({ preventScroll: true });
    update();
    requestLayout();
  }

  function key(code: string, modifiers: string[] = []) {
    if (!options.ready()) return;
    const chord = [...new Set([...sticky, ...modifiers, code])];
    sticky.clear();
    options.chord(chord);
    update();
  }

  function text(value: string) {
    if (!value || !options.ready()) return;
    const symbol = SYMBOL_KEYS[value];
    if (sticky.size && (/^[a-zA-Z]$/.test(value) || symbol)) {
      key(symbol?.[0] ?? `Key${value.toUpperCase()}`, (symbol ? symbol[1] : /[A-Z]/.test(value)) ? ["ShiftLeft"] : []);
    } else {
      sticky.clear();
      options.text(value);
      update();
    }
  }

  function action(button: HTMLButtonElement) {
    if (button.disabled) return;
    if (button.dataset.mobileAction === "menu") {
      hideKeyboard();
      options.tools(!options.isToolsOpen());
      return;
    }
    if (button.dataset.mobileAction === "fullscreen") { hideKeyboard(); options.toggleFullscreen?.(); return; }
    if (!options.ready()) return;
    if (button.dataset.mobileAction === "audio") { options.toggleAudio?.(); return; }
    if (button === toggle || button.dataset.mobileAction === "keyboard") { if (focused()) hideKeyboard(); else showKeyboard(); return; }
    const code = button.dataset.mobileKey;
    if (code && MODIFIERS.has(code)) {
      if (sticky.has(code)) sticky.delete(code); else sticky.add(code);
      update();
    } else if (code) {
      key(code, (button.dataset.mobileModifiers ?? "").split(" ").filter(Boolean));
    } else if (button.dataset.mobileText) {
      text(button.dataset.mobileText);
    }
  }

  // Reveal controls without consuming the click meant for the remote desktop.
  listen(document, "pointerdown", revealFullscreenExit, { capture: true, passive: true });
  listen(options.fullscreenExit, "pointerdown", (event: PointerEvent) => event.preventDefault());
  listen(options.fullscreenExit, "click", () => {
    options.exitFullscreen?.();
  });

  listen(options.fullscreenRotate, "pointerdown", (event: PointerEvent) => event.preventDefault());
  listen(options.fullscreenRotate, "click", () => { options.rotateScreen?.(); revealFullscreenExit(); });

  listen(bar, "pointerdown", (event: PointerEvent) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
    if (!button || button.disabled) return;
    event.preventDefault(); // Keep the IME input focused while a key is pressed.
    suppressClick = null;
    shortcutPointer = { id: event.pointerId, button, x: event.clientX, y: event.clientY, moved: false, repeated: false };
    button.setPointerCapture?.(event.pointerId);
    if (REPEAT.has(button.dataset.mobileKey ?? "")) {
      const repeat = () => {
        if (!shortcutPointer || shortcutPointer.moved || !options.ready()) return;
        shortcutPointer.repeated = true;
        action(button);
        repeatTimer = setTimeout(repeat, 80);
      };
      repeatTimer = setTimeout(repeat, 320);
    }
  });
  listen(bar, "pointermove", (event: PointerEvent) => {
    if (shortcutPointer?.id === event.pointerId && Math.hypot(event.clientX - shortcutPointer.x, event.clientY - shortcutPointer.y) > 8) {
      shortcutPointer.moved = true;
      cancelRepeat();
    }
  });
  listen(bar, "pointerup", (event: PointerEvent) => {
    if (shortcutPointer?.id !== event.pointerId) return;
    const current = shortcutPointer;
    shortcutPointer = null;
    cancelRepeat();
    if (current.button.hasPointerCapture?.(current.id)) current.button.releasePointerCapture(current.id);
    // Toggle on click so one pointer gesture changes the toolbar state once.
    if (current.button.dataset.mobileAction === "menu") {
      suppressClick = null;
      return;
    }
    suppressClick = current.button;
    if (!current.moved && !current.repeated) action(current.button);
  });
  listen(bar, "pointercancel", () => { shortcutPointer = null; cancelRepeat(); });
  listen(bar, "lostpointercapture", (event: PointerEvent) => { if (shortcutPointer?.id === event.pointerId) { shortcutPointer = null; cancelRepeat(); } });
  listen(bar, "mousedown", (event: MouseEvent) => event.preventDefault());
  listen(bar, "click", (event: MouseEvent) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
    if (!button) return;
    if (suppressClick === button && event.detail !== 0) { suppressClick = null; return; }
    action(button);
  });

  listen(input, "focus", () => {
    keyboardReference = Math.max(window.innerHeight, geometry().height);
    keyboardWasVisible = false;
    if (closingTimer !== null) clearTimeout(closingTimer);
    closingTimer = null; update(); requestLayout();
  });
  listen(input, "blur", () => {
    composing = false;
    sticky.clear();
    if (closingTimer !== null) clearTimeout(closingTimer);
    closingTimer = setTimeout(() => { closingTimer = null; requestLayout(); }, 350);
    update(); requestLayout();
  });
  listen(input, "compositionstart", () => {
    composing = true;
    committed = "";
    physicalInput = false;
    options.composition?.();
  });
  listen(input, "compositionend", (event: CompositionEvent) => {
    composing = false;
    committed = event.data;
    text(event.data);
    resetField();
  });
  listen(input, "beforeinput", (event: InputEvent) => {
    if (composing || event.isComposing || !event.cancelable) return;
    if (options.hardwareInput?.(event)) { event.preventDefault(); resetField(); return; }
    if (physicalInput) { event.preventDefault(); physicalInput = false; resetField(); return; }
    const code = ({ deleteContentBackward: "Backspace", deleteContentForward: "Delete", insertLineBreak: "Enter", insertParagraph: "Enter" } as Record<string, string>)[event.inputType];
    if (code) { event.preventDefault(); key(code); resetField(); }
  });
  listen(input, "input", (event: InputEvent) => {
    if (composing || event.isComposing) return;
    if (options.hardwareInput?.(event)) { resetField(); return; }
    const value = input.value.startsWith(SENTINEL) ? input.value.slice(1) : input.value;
    const code = ({ deleteContentBackward: "Backspace", deleteContentForward: "Delete", insertLineBreak: "Enter", insertParagraph: "Enter" } as Record<string, string>)[event.inputType];
    if (physicalInput) physicalInput = false;
    else if (code) key(code);
    else if (value && (!committed || value !== committed)) text(value);
    committed = "";
    resetField();
  });
  listen(input, "keydown", (event: KeyboardEvent) => {
    if (event.isComposing || composing || event.keyCode === 229) return;
    committed = "";
    if (!touch && options.hardwareKey) { physicalInput = options.hardwareKey(event); return; }
    const code = event.code || ({ Enter: "Enter", Backspace: "Backspace", Delete: "Delete", Tab: "Tab", Escape: "Escape" } as Record<string, string>)[event.key];
    if (!code || MODIFIERS.has(code)) return;
    if (code === "KeyV" && (event.ctrlKey || event.metaKey)) return;
    if (event.ctrlKey || event.altKey || event.metaKey || !event.key || event.key.length !== 1) {
      event.preventDefault();
      key(code, [event.ctrlKey ? "ControlLeft" : "", event.altKey ? "AltLeft" : "", event.shiftKey ? "ShiftLeft" : "", event.metaKey ? "MetaLeft" : ""].filter(Boolean));
    }
  });
  listen(input, "paste", (event: ClipboardEvent) => {
    if (options.paste) options.paste(event);
    else { event.preventDefault(); text(event.clipboardData?.getData("text/plain") ?? ""); }
    physicalInput = false; committed = ""; resetField();
  });

  function cancelTouches() {
    nativePair.reset();
    if (gestureFrame !== null) cancelAnimationFrame(gestureFrame);
    gestureFrame = null;
    cancelHold();
    if (dragging) options.send({ type: "button", button: 0, pressed: false });
    dragging = false;
    rightClicked = false;
    multiTouch = false;
    gestureOrigin = null;
    gestureMode = "pending";
    centroid = null;
    const ids = [...activeTouches.keys()];
    activeTouches.clear();
    for (const id of ids) if (canvas.hasPointerCapture?.(id)) canvas.releasePointerCapture(id);
  }

  function applyGesture(snapshot?: TouchPairSample) {
    if (!options.ready() || !centroid || (!snapshot && activeTouches.size !== 2)) return;
    const [a, b] = [...activeTouches.values()];
    const next = snapshot ?? { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2, distance: Math.hypot(a.x - b.x, a.y - b.y) };
    const origin = gestureOrigin ?? centroid;
    const distanceChanged = Math.abs(next.distance - origin.distance) > Math.max(8, origin.distance * .06);
    const from = gestureMode === "pending" ? origin : centroid;
    if (distanceChanged) gestureMode = "zoom";
    else if (gestureMode === "pending" && Math.hypot(next.x - origin.x, next.y - origin.y) > 8) gestureMode = "scroll";
    if (gestureMode === "zoom" || gestureMode === "pan") {
      options.zoom?.gesture(from, next, gestureMode === "zoom" && from.distance > 0 ? next.distance / from.distance : 1);
    } else if (gestureMode === "scroll") {
      const delta = pictureScrollDelta(from.x-next.x, from.y-next.y, options.rotated?.());
      options.send({ type: "scroll", x: Math.max(-1000, Math.min(1000, delta.x)), y: Math.max(-1000, Math.min(1000, delta.y)) });
    }
    centroid = next;
  }

  const nativePair = createTouchPair(canvas, {
    enabled: options.ready,
    start(sample) {
      cancelHold();
      if (gestureFrame !== null) cancelAnimationFrame(gestureFrame);
      gestureFrame = null;
      if (dragging) options.send({ type: "button", button: 0, pressed: false });
      dragging = false; multiTouch = true;
      centroid = gestureOrigin = sample;
      gestureMode = options.zoom?.active() ? "pan" : "pending";
    },
    move: applyGesture,
    end() { cancelTouches(); },
  });

  function handlePointer(event: PointerEvent, phase: "down" | "move" | "up" | "cancel") {
    if (event.pointerType !== "touch") return false;
    event.preventDefault();
    if (!options.ready()) { cancelTouches(); return true; }
    touch = isMobilePlatform();
    update();
    if (phase === "cancel") {
      if (nativePair.active()) activeTouches.delete(event.pointerId);
      else cancelTouches();
      return true;
    }
    if (phase === "down") {
      options.tools(false);
      activeTouches.set(event.pointerId, { x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY });
      canvas.setPointerCapture?.(event.pointerId);
      if (nativePair.active()) return true;
      if (activeTouches.size === 1 && !multiTouch) {
        rightClicked = false;
        options.move(event);
        holdTimer = setTimeout(() => {
          holdTimer = null;
          if (activeTouches.size !== 1 || dragging || multiTouch || !options.ready()) return;
          rightClicked = true;
          options.send({ type: "button", button: 2, pressed: true });
          options.send({ type: "button", button: 2, pressed: false });
        }, 500);
      } else {
        cancelHold();
        if (dragging) options.send({ type: "button", button: 0, pressed: false });
        dragging = false;
        multiTouch = true;
        const [a, b] = [...activeTouches.values()];
        if (b && activeTouches.size === 2) {
          centroid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2, distance: Math.hypot(a.x - b.x, a.y - b.y) };
          gestureOrigin = centroid;
          gestureMode = options.zoom?.active() ? "pan" : "pending";
        } else { centroid = gestureOrigin = null; }
      }
      requestLayout();
      return true;
    }
    const current = activeTouches.get(event.pointerId);
    if (!current) return true;
    current.x = event.clientX;
    current.y = event.clientY;
    if (nativePair.active()) {
      if (phase === "up") {
        activeTouches.delete(event.pointerId);
        if (canvas.hasPointerCapture?.(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
      }
      return true;
    }
    if (phase === "move") {
      if (multiTouch) {
        if (activeTouches.size !== 2 || !centroid) return true;
        // Both fingers arrive as separate pointer events. Sample them together
        // so a translation cannot transiently pinch against the zoom limit.
        if (gestureFrame === null) gestureFrame = requestAnimationFrame(() => { gestureFrame = null; applyGesture(); });
      } else if (!rightClicked && Math.hypot(current.x - current.startX, current.y - current.startY) > 8) {
        cancelHold();
        if (!dragging) {
          options.move({ clientX: current.startX, clientY: current.startY });
          dragging = options.send({ type: "button", button: 0, pressed: true });
        }
        options.move(event);
      }
    } else {
      if (gestureFrame !== null) {
        cancelAnimationFrame(gestureFrame); gestureFrame = null;
        applyGesture();
      }
      cancelHold();
      if (!multiTouch && !rightClicked) {
        options.move(event);
        if (!dragging) options.send({ type: "button", button: 0, pressed: true });
        options.send({ type: "button", button: 0, pressed: false });
      }
      dragging = false;
      activeTouches.delete(event.pointerId);
      if (canvas.hasPointerCapture?.(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
      if (!activeTouches.size) { multiTouch = false; centroid = gestureOrigin = null; gestureMode = "pending"; }
    }
    return true;
  }

  function reset() {
    cancelTouches();
    cancelRepeat();
    if (shortcutPointer?.button.hasPointerCapture?.(shortcutPointer.id)) shortcutPointer.button.releasePointerCapture(shortcutPointer.id);
    shortcutPointer = null;
    sticky.clear();
    physicalInput = false;
    hideKeyboard();
  }

  listen(window.visualViewport, "resize", requestLayout);
  listen(window.visualViewport, "scroll", requestLayout);
  listen(window, "resize", requestLayout);
  listen(window, "orientationchange", requestLayout);
  listen(window, "blur", () => { cancelTouches(); cancelRepeat(); sticky.clear(); update(); });
  listen(document, "visibilitychange", () => { if (document.hidden) { reset(); hideFullscreenExit(); } });
  listen(coarse, "change", () => { touch = isMobilePlatform(); update(); requestLayout(); });
  const observer = window.ResizeObserver ? new window.ResizeObserver(requestLayout) : null;
  observer?.observe(bar);
  // Host-injected safe areas can change without a window resize. Observing
  // the content box also catches top/side padding and orientation changes.
  observer?.observe(viewport);
  observer?.observe(canvas);
  hideFullscreenExit();
  update();
  requestLayout();
  return {
    handlePointer, reset, update, setFullscreen, hideKeyboard,
    isFullscreen: () => touch && fullscreen,
    isTouch: () => touch,
    refreshLayout: () => { if (frame !== null) cancelAnimationFrame(frame); frame = null; layout(); },
    focusDesktop: () => {
      if (touch || !options.ready()) return false;
      if (!focused()) { resetField(); input.focus({ preventScroll: true }); }
      return true;
    },
    keyboardActive: () => touch && (focused() || closingTimer !== null),
    touching: () => activeTouches.size > 0 || nativePair.active(),
    lostPointer: (id: number) => { if (nativePair.active()) activeTouches.delete(id); else if (activeTouches.has(id)) cancelTouches(); },
    bounds: () => touch ? contentBounds() : null,
    dispose() {
      reset();
      nativePair.dispose();
      disposed = true;
      hideFullscreenExit();
      if (closingTimer !== null) clearTimeout(closingTimer);
      lifecycle.abort();
      observer?.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      viewport.classList.toggle("touch-desktop", false);
      viewport.classList.toggle("mobile-fullscreen", false);
      viewport.classList.toggle("software-keyboard-open", false);
      viewport.classList.toggle("software-keyboard-visible", false);
    },
  };
}
