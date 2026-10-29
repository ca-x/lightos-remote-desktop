import { pictureNormalizedPoint, pictureScrollDelta } from "./picture-geometry";
import { createScreenRotation } from "./screen-rotation";
import { createLightBookInput, supportsLightBook } from "./lightbook-input";
import { createPictureZoom } from "./picture-zoom";
import { encoderBackendKey, encoderBackends, readPreference, savePreference, videoCodecKey, videoCodecs, streamStrategyKey, streamStrategies } from "./preferences";
import { supportedVideoConfig } from "./video-codec";
import type { ClientOptions, DesktopChoice, DesktopOutput, Elements, SessionView, ToolbarEdge } from "./types";
import { createMobileControls, mobileResolution } from "./mobile";
import { createCursorLayer } from "./cursor";
import { createClipboard } from "./clipboard";
import { captureResolution, encodingBounds, CAPTURE_SCALES, LEGACY_SCALES } from "./resolution";
import { decodeDesktopTiles, parseDesktopUpdate } from "./desktop-updates";
import audioWorkerUrl from "./audio-worker.js?url";
import audioWorkletUrl from "./audio-worklet.js?url";

export const KEY_CODES: Readonly<Record<string, number>> = Object.freeze({
  Escape: 1, Digit1: 2, Digit2: 3, Digit3: 4, Digit4: 5, Digit5: 6,
  Digit6: 7, Digit7: 8, Digit8: 9, Digit9: 10, Digit0: 11, Minus: 12,
  Equal: 13, Backspace: 14, Tab: 15, KeyQ: 16, KeyW: 17, KeyE: 18,
  KeyR: 19, KeyT: 20, KeyY: 21, KeyU: 22, KeyI: 23, KeyO: 24,
  KeyP: 25, BracketLeft: 26, BracketRight: 27, Enter: 28, ControlLeft: 29,
  KeyA: 30, KeyS: 31, KeyD: 32, KeyF: 33, KeyG: 34, KeyH: 35,
  KeyJ: 36, KeyK: 37, KeyL: 38, Semicolon: 39, Quote: 40, Backquote: 41,
  ShiftLeft: 42, Backslash: 43, KeyZ: 44, KeyX: 45, KeyC: 46, KeyV: 47,
  KeyB: 48, KeyN: 49, KeyM: 50, Comma: 51, Period: 52, Slash: 53,
  ShiftRight: 54, NumpadMultiply: 55, AltLeft: 56, Space: 57, CapsLock: 58,
  F1: 59, F2: 60, F3: 61, F4: 62, F5: 63, F6: 64, F7: 65, F8: 66,
  F9: 67, F10: 68, NumLock: 69, ScrollLock: 70, Numpad7: 71, Numpad8: 72,
  Numpad9: 73, NumpadSubtract: 74, Numpad4: 75, Numpad5: 76, Numpad6: 77,
  NumpadAdd: 78, Numpad1: 79, Numpad2: 80, Numpad3: 81, Numpad0: 82,
  NumpadDecimal: 83, IntlBackslash: 86, F11: 87, F12: 88, IntlRo: 89,
  Convert: 92, KanaMode: 93, NonConvert: 94, NumpadEnter: 96,
  ControlRight: 97, NumpadDivide: 98, PrintScreen: 99, AltRight: 100,
  Home: 102, ArrowUp: 103, PageUp: 104, ArrowLeft: 105, ArrowRight: 106,
  End: 107, ArrowDown: 108, PageDown: 109, Insert: 110, Delete: 111,
  AudioVolumeMute: 113, AudioVolumeDown: 114, AudioVolumeUp: 115,
  NumpadEqual: 117, Pause: 119, NumpadComma: 121, IntlYen: 124,
  MetaLeft: 125, MetaRight: 126, ContextMenu: 127,
  F13: 183, F14: 184, F15: 185, F16: 186, F17: 187, F18: 188,
  F19: 189, F20: 190, F21: 191, F22: 192, F23: 193, F24: 194,
});

export function createClient(options: ClientOptions = {}) {
  const element = <Name extends keyof Elements>(id: Name): Elements[Name] => {
    const node = document.getElementById(id);
    if (!node) throw new Error(`Missing client element: ${id}`);
    return node as unknown as Elements[Name];
  };
  const texts = new Map<keyof Elements, string>();
  const translate = (value: string) => options.translate?.(value) ?? value;
  const setText = (id: keyof Elements, value: string) => {
    if (texts.get(id) === value) return;
    texts.set(id, value);
    element(id).textContent = translate(value);
    if (id === "state" || id === "performance" || id === "video-details" || id === "message" || id === "desktop-status") options.onStatus?.(id, translate(value));
  };
  const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);
  type Timer = ReturnType<typeof setTimeout>;
  const cancelTimer = (timer: Timer | null) => { if (timer !== null) clearTimeout(timer); };
  const canvas = element("desktop");
  const drawing = canvas.getContext("2d", { alpha: false });
  const viewport = element("viewport");
  const textBox = element("text");
  const controls = ["apply-size", "send-text", "clipboard-get", "clipboard-set", "paste-local", "keyboard-capture", "send-shortcut", "switch-input-method"] as const;
  const listeners = new AbortController();
  const keys = new Set<number>();
  const deferredMeta = new Set<number>();
  let localMeta = false;
  let clipboardKeyEvent: number | null = null;
  const appleClipboard = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);
  const buttons = new Set<number>();
  const sources = new Set<AudioBufferSourceNode>();
  let socket: WebSocket | null = null;
  let wanted = false;
  let ready = false;
  let desktopPaused = false;
  let cursorSession: string | null = null;
  let cursorEmbedded = true;
  let desktopInitializing = false;
  let destroyed = false;
  let generation = 0;
  let retryTimer: Timer | null = null;
  let connectionTimer: Timer | null = null;
  let closeTimer: Timer | null = null;
  let retries = 0;
  let lastServerActivity = 0;
  let connectedAt: number | null = null;
  let connectionFailure = "";
  let pendingFrame: { data: ArrayBuffer; generation: number; videoToken: number; id: number | null } | null = null;
  let decoding = false;
  let audioAvailable = false;
  let audioContext: AudioContext | null = null;
  let releaseAudioSession: (() => void) | null = null;
  let cancelAudioResume: (() => void) | null = null;
  let audioEnabled = false;
  let audioSession: number | null = null;
  let audioSocket: WebSocket | null = null;
  let audioConnectionTimer: Timer | null = null;
  let audioWorker: Worker | null = null;
  let audioNode: AudioWorkletNode | null = null;
  let cancelAudioPreparation: (() => void) | null = null;
  let opusAvailable = false;
  let audioCodec = "pcm";
  let audioPlayback: { bufferMs: number; underruns: number; dropped: number } | null = null;
  let pendingAudioPackets = 0;
  let nextAudioTime = 0;
  let composing = false;
  let clipboardPending = false;
  let resumeWanted = false;
  let moveFrame: number | null = null;
  let pendingMove: Pick<PointerEvent, "clientX" | "clientY"> | null = null;
  let backendPreference = readPreference(encoderBackendKey, encoderBackends, "auto");
  let backendSelection = false;
  let videoRequestIds = false;
  let videoScaling = false;
  let sourceWidth = canvas.width;
  let sourceHeight = canvas.height;
  let lastVideoBounds: string | null = null;
  let videoScaleTimer: Timer | null = null;
  const frameSourceSizes = new Map<number, { width: number; height: number }>();
  let videoRequestId = 0;
  let activeBackend: string | null = null;
  let waitingForBackend = false;
  const rejectedBackends = new Set<string>();
  element("encoder-backend").value = backendPreference;
  let videoPreference = readPreference(videoCodecKey, videoCodecs, "auto");
  let strategyPreference = readPreference(streamStrategyKey, streamStrategies, "auto");
  let serverVideoCodecs: string[] = [];
  element("video-codec").value = videoPreference;
  element("stream-strategy").value = strategyPreference;
  let frameAcknowledgements = false;
  let losslessAvailable = false;
  let losslessChain = Promise.resolve();
  let windowMode = "fit";
  let fullscreenAutoResolution: boolean | null = null;
  let desktopFullscreen = false;
  let fullscreenFallback = false;
  let pictureRotated = false;
  let videoDecoder: VideoDecoder | null = null;
  let videoToken = 0;
  let configuringVideo = false;
  let pendingVideo: { data: ArrayBuffer; id: number | null } | null = null;
  let videoTimer: Timer | null = null;
  let sequencedFrames = false;
  let pendingFrameId: number | null = null;
  const videoFrameIds = new Map<number, number | null>();
  const videoDecodeStarted = new Map<number, number>();
  let autoResizeTimer: Timer | null = null;
  let resizeScaleAvailable = false;
  let resizeScales: readonly number[] = LEGACY_SCALES;
  let lastAutoResolution: string | null = null;
  let lastRequestedSize: string | null = null;
  let keyboardCaptured = false;
  let keyboardCaptureWanted = true;
  let keyboardRequest = false;
  let keyboardToken = 0;
  let desktopMode = "virtual";
  let desktopConfirmed = false;
  let selectedOutput: string | null = null;
  let desktopChoices: DesktopChoice[] = [];
  let desktopOutputs: DesktopOutput[] = [];
  let resizeAvailable = true;
  let switchingDesktop = false;
  let controlRelease = false;
  let releasingSocket: WebSocket | null = null;
  let discoveryRequest = 0;
  let refreshingDesktops = false;
  let discoveryController: AbortController | null = null;
  let mobile: ReturnType<typeof createMobileControls> | null = null;
  let lightBook: ReturnType<typeof createLightBookInput> = null;
  let lightBookEnabled = false;
  let discoveryReason = "";
  let publishedView: SessionView | undefined;
  let keyChords = false;
  let imageClipboard = false;
  let textClipboard = false;
  let clipboardTextInput = false;
  let clipboardSession:string|null=null;
  let clipboardSync=false;
  let pendingHardwareKey: KeyboardEvent | null = null;
  let localPasteShift = false;
  const keyboardInput = element("mobile-input");
  const keyboardFocused = () => document.activeElement === canvas || document.activeElement === keyboardInput;
  const clipboard = createClipboard({ send, ready: () => ready && !desktopPaused && !destroyed, session: () => generation,
    images: () => imageClipboard, release, text: pasteText, message, signal: listeners.signal, shift: () => localPasteShift });

  function desktopLabel() {
    return desktopMode === "physical" ? "物理桌面" : "虚拟桌面";
  }

  function desktopStatus(text: string) {
    setText("desktop-status", text);
    element("desktop-status").setAttribute("title", translate(text));
  }

  function outputSummary(output: DesktopOutput) {
    return `${output.name} · ${output.width} × ${output.height} · 缩放 ${output.scale ?? 1}×`;
  }

  function outputTitle(output: DesktopOutput) {
    return `${output.description ? `${output.description} · ` : ""}${outputSummary(output)} · 位置 (${output.x ?? "?"}, ${output.y ?? "?"})`;
  }

  function populateDesktops(value: {desktops?: DesktopChoice[]; outputs?: DesktopOutput[]; reason?: string}) {
    discoveryReason = value.reason ?? "";
    desktopChoices = Array.isArray(value.desktops) ? value.desktops.filter((desktop) =>
      desktop && ["virtual", "physical"].includes(desktop.mode) && typeof desktop.label === "string") : [];
    const select = element("desktop-source");
    select.replaceChildren();
    for (const desktop of desktopChoices) {
      const option = document.createElement("option");
      option.value = desktop.mode;
      option.textContent = translate(desktop.label);
      option.disabled = desktop.available !== true;
      option.setAttribute("title", translate(desktop.available === true ? desktop.label : desktop.reason || "桌面不可用"));
      select.appendChild(option);
    }
    select.value = desktopMode;
    const unavailable = desktopChoices.filter((desktop) => desktop.available !== true)
      .map((desktop) => `${desktop.label}：${desktop.reason || "不可用"}`);
    select.setAttribute("title", translate(["切换桌面来源", ...unavailable].join("；")));
    desktopOutputs = Array.isArray(value.outputs) ? value.outputs.filter((output) =>
      output && typeof output.name === "string" && output.name && validDimensions(output.width, output.height)) : [];
    const monitors = element("monitor-source");
    monitors.replaceChildren();
    for (const output of desktopOutputs) {
      const option = document.createElement("option");
      option.value = output.name;
      option.textContent = `${output.name} · ${output.width} × ${output.height}`;
      option.setAttribute("title", translate(outputTitle(output)));
      monitors.appendChild(option);
    }
    monitors.value = selectedOutput || "";
    const selected = desktopOutputs.find((output) => output.name === selectedOutput);
    monitors.setAttribute("title", translate(selected ? outputTitle(selected) : "选择一块物理显示器"));
    const connected = ready ? `已连接${desktopLabel()}${selected ? ` · ${outputSummary(selected)}` : selectedOutput ? ` · ${selectedOutput}` : ""}。` : "";
    const reason = value.reason || unavailable.join("；");
    if (connected || reason) desktopStatus([connected, reason].filter(Boolean).join("；"));
    updateControls();
    layoutTools();
  }

  function switchDesktop(mode: string, output: string | null = null) {
    const choice = desktopChoices.find((desktop) => desktop.mode === mode);
    if (!choice || choice.available !== true) {
      element("desktop-source").value = desktopMode;
      desktopStatus(choice?.reason || "桌面不可用");
      return;
    }
    if (mode === desktopMode && output === selectedOutput) return;
    const canRelease = ready && controlRelease && socket?.readyState === WebSocket.OPEN;
    const releasePending = socket !== null && releasingSocket === socket;
    release();
    stopAudio();
    if (connectionFailure) wanted = true;
    connectionFailure = "";
    desktopMode = mode;
    desktopConfirmed = true;
    selectedOutput = mode === "physical" ? output : null;
    switchingDesktop = socket !== null;
    cancelTimer(retryTimer);
    retryTimer = null;
    cancelTimer(connectionTimer);
    connectionTimer = null;
    retries = 0;
    resetSession();
    desktopStatus(`${wanted ? "正在连接" : "已选择"}${desktopLabel()}，${wanted ? "尚未确认" : "尚未连接"}${selectedOutput ? ` · ${selectedOutput}` : ""}。`);
    setText("state", wanted ? "Connecting" : "Disconnected");
    if (socket && canRelease) {
      releasingSocket = socket;
      waitForClose(socket);
      socket.send(JSON.stringify({ type: "disconnect" }));
    } else if (socket && !releasePending) closeSocket(socket, 1000, "Switch desktop");
    else connect();
  }

  function updateKeyboardCapture() {
    publishSessionView();
    element("keyboard-capture").setAttribute("aria-pressed", String(keyboardCaptured));
    setText("keyboard-capture", keyboardCaptured ? "释放键盘捕获" : "全屏捕获键盘");
  }

  function nativeDesktopFullscreen() {
    const fullscreen = document.fullscreenElement;
    return fullscreen === viewport || fullscreen === canvas || fullscreen?.contains(canvas) === true;
  }
  function isDesktopFullscreen() { return fullscreenFallback || nativeDesktopFullscreen(); }

  function unlockKeyboard() {
    keyboardToken += 1;
    try { navigator.keyboard?.unlock(); } catch {}
    keyboardCaptured = false;
    setText("keyboard-state", "未捕获键盘。全屏可尝试捕获；系统拦截的按键可通过通用组合键发送器发送。");
    updateKeyboardCapture();
  }

  async function captureKeyboard() {
    if (!ready || destroyed || keyboardRequest || keyboardCaptured || !keyboardCaptureWanted || document.hidden || !document.hasFocus() || !isDesktopFullscreen() || !keyboardFocused()) return;
    if (!navigator.keyboard?.lock) {
      setText("keyboard-state", "浏览器不支持键盘捕获。系统截走的组合键可用下方通用发送器发送。");
      return;
    }
    const token = keyboardToken;
    keyboardRequest = true;
    try {
      // Keep native paste available. Locking KeyV suppresses the browser's
      // trusted paste event on Chromium/Electron even when keydown is allowed.
      await navigator.keyboard.lock(Object.keys(KEY_CODES).filter(code => code !== "KeyV"));
      if (token !== keyboardToken || !ready || destroyed || document.hidden || !document.hasFocus() || !isDesktopFullscreen() || !keyboardFocused()) {
        navigator.keyboard.unlock();
        return;
      }
      keyboardCaptured = true;
      updateKeyboardCapture();
      setText("keyboard-state", "已请求捕获所有可用按键。部分系统快捷键仍可能被本机拦截。Ctrl+Alt+Shift+Esc 释放捕获并打开工具栏。");
    } catch (error) {
      if (token === keyboardToken) {
        setText("keyboard-state", "键盘捕获被拒绝，请检查浏览器权限或使用下方通用发送器。");
        message(`Keyboard capture unavailable: ${errorMessage(error)}`);
      }
    } finally {
      keyboardRequest = false;
      if (token !== keyboardToken) void captureKeyboard();
    }
  }
  const toolHandle = element("tools-toggle");
  const tools = element("tools");
  const toolPositionKey = "lightos-desktop-toolbar-position";
  let toolPosition = { edge: "right", ratio: 0.5 };
  let toolDrag: { id: number; startX: number; startY: number; offsetX: number; offsetY: number; x: number; y: number; moved: boolean } | null = null;
  let suppressToolClick = false;
  try {
    const saved = JSON.parse(localStorage.getItem(toolPositionKey) ?? "null");
    if (["left", "right", "top", "bottom"].includes(saved?.edge) && Number.isFinite(saved.ratio)) {
      toolPosition = { edge: saved.edge, ratio: Math.max(0, Math.min(1, saved.ratio)) };
    }
  } catch {}

  function nearestToolEdge(centerX: number, centerY: number, width: number, height: number): ToolbarEdge {
    const distances = { left: centerX, right: width - centerX, top: centerY, bottom: height - centerY };
    return (Object.keys(distances) as ToolbarEdge[]).reduce((nearest, candidate) => distances[candidate] < distances[nearest] ? candidate : nearest);
  }

  function layoutTools(point: {x: number; y: number} | null = toolDrag?.moved ? toolDrag : null) {
    const bounds = mobile?.bounds();
    const width = bounds?.width || window.innerWidth || 1280;
    const height = bounds ? bounds.height - bounds.bar : window.innerHeight || 720;
    const originX = bounds?.left ?? 0;
    const originY = bounds?.top ?? 0;
    const handleWidth = toolHandle.offsetWidth || 40;
    const handleHeight = toolHandle.offsetHeight || 40;
    const maxX = Math.max(12, width - handleWidth - 12);
    const maxY = Math.max(12, height - handleHeight - 12);
    const horizontal = toolPosition.edge === "top" || toolPosition.edge === "bottom";
    const left = point ? Math.max(12, Math.min(maxX, point.x - originX)) :
      horizontal ? 12 + toolPosition.ratio * (maxX - 12) : toolPosition.edge === "left" ? 12 : maxX;
    const top = point ? Math.max(12, Math.min(maxY, point.y - originY)) :
      horizontal ? toolPosition.edge === "top" ? 12 : maxY : 12 + toolPosition.ratio * (maxY - 12);
    tools.style.setProperty("--tool-x", `${left + originX}px`);
    tools.style.setProperty("--tool-y", `${top + originY}px`);
    const leftSpace = Math.max(0, left - 12);
    const rightSpace = Math.max(0, width - 12 - left - handleWidth);
    const directionEdge = point ? nearestToolEdge(left + handleWidth / 2, top + handleHeight / 2, width, height) : toolPosition.edge;
    tools.setAttribute("data-edge", directionEdge);
    options.onToolbarDock?.(directionEdge as ToolbarEdge);
    const expandLeft = directionEdge === "right" || (directionEdge === "top" || directionEdge === "bottom") && leftSpace > rightSpace;
    tools.style.setProperty("--panel-max-width", `${Math.max(40, expandLeft ? leftSpace : rightSpace)}px`);
    tools.setAttribute("data-expand", expandLeft ? "left" : "right");
    // Keep the horizontal expansion direction separate from the dock edge:
    // top/bottom show vertical arrows until the toolbar opens.
    options.onToolbarDirection?.(expandLeft ? "left" : "right");
    // Keep the full layout mounted. Only its clip changes during expansion, so
    // closing and rapid reversals never remeasure an intermediate animation.
    const panel = element("tool-panel");
    const panelWidth = panel.offsetWidth || panel.getBoundingClientRect().width;
    const panelHeight = panel.offsetHeight || panel.getBoundingClientRect().height;
    const panelX = expandLeft ? -panelWidth : handleWidth;
    const desiredY = (handleHeight - panelHeight) / 2;
    const panelY = Math.max(12 - top, Math.min(height - 12 - top - panelHeight, desiredY));
    const shellX = Math.min(0, panelX), shellY = Math.min(0, panelY);
    const shellWidth = Math.max(handleWidth, panelX + panelWidth) - shellX;
    const shellHeight = Math.max(handleHeight, panelY + panelHeight) - shellY;
    for (const [name, value] of Object.entries({
      "shell-x": shellX, "shell-y": shellY, "shell-width": shellWidth, "shell-height": shellHeight,
      "panel-x": panelX - shellX, "panel-y": panelY - shellY,
      "clip-top": -shellY, "clip-right": shellWidth + shellX - handleWidth,
      "clip-bottom": shellHeight + shellY - handleHeight, "clip-left": -shellX,
    })) tools.style.setProperty(`--${name}`, `${value}px`);
  }

  function saveToolPosition() {
    try { localStorage.setItem(toolPositionKey, JSON.stringify(toolPosition)); } catch {}
    layoutTools();
  }
  let metricsTimer: Timer | null = null;
  let metricsStarted = 0;
  let renderedFrames = 0;
  let pingSequence = 0;
  let pendingPing: {id: number; started: number} | null = null;
  let latestRtt: number | null = null;
  function pictureBounds() {
    const mobileBounds = mobile?.bounds();
    if (mobileBounds) return { left: mobileBounds.left, top: mobileBounds.top,
      right: mobileBounds.left + mobileBounds.width, bottom: mobileBounds.top + Math.max(0, mobileBounds.height - mobileBounds.bar) };
    const rect = viewport.getBoundingClientRect();
    return { left: rect.left, top: rect.top, right: rect.left + (viewport.clientWidth || rect.width), bottom: rect.top + (viewport.clientHeight || rect.height) };
  }
  const cursor = createCursorLayer(canvas, element("remote-cursor"), () => latestRtt ?? 0, pictureBounds, () => pictureRotated);
  const pictureZoom = createPictureZoom({ canvas, viewport, bounds: pictureBounds,
    enabled: () => ready && !desktopPaused && desktopMode === "physical",
    touching: () => mobile?.touching() ?? false, changed: () => { cursor.layout(); scheduleVideoScale(); } });
  const screenRotation = createScreenRotation({
    active: () => !!mobile?.isFullscreen() && !destroyed,
    landscape: () => (window.visualViewport?.width ?? window.innerWidth) > (window.visualViewport?.height ?? window.innerHeight),
    orientation: () => window.screen?.orientation,
    pending: (value) => { element("mobile-fullscreen-rotate").disabled = value; },
    release,
    rotated(value) {
      if (pictureRotated === value) return;
      pictureRotated = value; pictureZoom.setRotated(value);
      lastAutoResolution = null; mobile?.refreshLayout(); scheduleAutoResize();
    },
  });
  listen(window, "orientationchange", () => screenRotation.deviceChanged());
  if (window.screen?.orientation) listen(window.screen.orientation, "change", () => screenRotation.deviceChanged());
  const frameCursorModes = new Map<number, boolean>();
  const rttSamples: number[] = [];
  const renderSamples: number[] = [];
  let pipelineMetrics: Record<string, { count: number; p50_ms: number | null; p95_ms: number | null }> = {};
  function cursorFreeFrame(id?:number|null) {
    const embedded=typeof id==="number"?(frameCursorModes.get(id)??cursorEmbedded):cursorEmbedded;
    if(!embedded)return true;
    if(typeof id==="number") {frameCursorModes.delete(id);frameSourceSizes.delete(id);}
    return false;
  }

  function recordFrame(codec: string, started?: number, id?: number | null) {
    if (typeof id === "number") {
      const embedded = frameCursorModes.get(id);
      if (typeof embedded === "boolean") cursor.present(embedded);
      for (const key of frameCursorModes.keys()) if (key <= id) frameCursorModes.delete(key);
      for (const key of frameSourceSizes.keys()) if (key <= id) frameSourceSizes.delete(key);
    }
    renderedFrames += 1;
    if (started !== undefined) {
      renderSamples.push(Math.max(0, performance.now() - started));
      if (renderSamples.length > 120) renderSamples.shift();
    }
    setText("video-details", `${codec} · ${canvas.width} × ${canvas.height}`);
  }

  function stopMetrics() {
    cancelTimer(metricsTimer);
    metricsTimer = null;
    pendingPing = null;
    latestRtt = null;
    rttSamples.length = 0;
    renderSamples.length = 0;
    pipelineMetrics = {};
    renderedFrames = 0;
    setText("performance", "-- FPS · RTT -- ms");
    setText("video-details", "等待画面");
  }

  function sampleMetrics() {
    metricsTimer = null;
    if (!ready || destroyed) return;
    const now = performance.now();
    if (!document.hidden && now - lastServerActivity >= 30_000 && socket) {
      connectionFailure = "Connection stopped responding; reconnecting.";
      message(connectionFailure);
      closeSocket(socket, 4000, "Liveness timeout");
      resetSession();
      return;
    }
    const fps = renderedFrames * 1000 / Math.max(1, now - metricsStarted);
    renderedFrames = 0;
    metricsStarted = now;
    const sorted = [...rttSamples].sort((first, second) => first - second);
    const p95 = sorted.length ? Math.round(sorted[Math.ceil(sorted.length * 0.95) - 1]) : "--";
    const rtt = latestRtt === null ? "--" : Math.round(latestRtt);
    const sortedRender = [...renderSamples].sort((first, second) => first - second);
    const renderP95 = sortedRender.length ? sortedRender[Math.ceil(sortedRender.length * 0.95) - 1].toFixed(1) : "--";
    const serverP95 = (stage: string) => {
      const value = pipelineMetrics[stage]?.p95_ms;
      return typeof value === "number" && Number.isFinite(value) ? value.toFixed(1) : "--";
    };
    const details = [
      `${translate("Encode")} P95 ${serverP95("encode")} ms`,
      `${translate("Socket send")} P95 ${serverP95("send")} ms`,
      `ACK P95 ${serverP95("ack")} ms`,
      `${translate("Decode / draw")} P95 ${renderP95} ms`,
    ];
    if (audioEnabled) details.push(`${translate("Audio")} ${audioCodec}${audioPlayback ? ` · ${translate("Buffer")} ${Math.round(audioPlayback.bufferMs)} ms · ${translate("Underruns")} ${audioPlayback.underruns} · ${translate("Dropped samples")} ${audioPlayback.dropped}` : ""}`);
    element("performance").setAttribute("title", details.join("\n"));
    setText("performance", `${fps.toFixed(1)} FPS · RTT ${rtt} ms · P95 ${p95} ms`);
    if (pendingPing && now - pendingPing.started >= 5000) {
      pendingPing = null;
      latestRtt = null;
      setText("performance", `${fps.toFixed(1)} FPS · RTT 超时 · P95 ${p95} ms`);
    }
    if (!pendingPing && !document.hidden) {
      const id = ++pingSequence;
      pendingPing = { id, started: now };
      if (!send({ type: "ping", id })) pendingPing = null;
    }
    if (ready && !destroyed) metricsTimer = setTimeout(sampleMetrics, 1000);
  }

  function listen<Name extends keyof (WindowEventMap & DocumentEventMap)>(target: EventTarget, name: Name, handler: (event: (WindowEventMap & DocumentEventMap)[Name]) => void, config: AddEventListenerOptions = {}) {
    target.addEventListener(name, handler as EventListener, { ...config, signal: listeners.signal });
  }

  function message(value: string) { setText("message", value); }

  function setToolsOpen(open: boolean, focus = false) {
    if (open) { lightBook?.cancel(); unlockKeyboard(); }
    if (options.onToolsOpen) {
      options.onToolsOpen(open, focus);
      requestAnimationFrame(() => { if (!destroyed) layoutTools(); });
      return;
    }
    element("tool-panel").hidden = !open;
    tools.classList.toggle("is-open", open);
    element("tools-toggle").setAttribute("aria-expanded", String(open));
    element("tools-toggle").setAttribute("aria-label", open ? "收起桌面工具栏" : "打开桌面工具栏");
    layoutTools();
    if (focus) (open ? toolHandle : canvas).focus({ preventScroll: true });
  }

  function validDimensions(width: number, height: number) {
    return Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 &&
      width <= 8192 && height <= 8192 && width * height <= 33554432;
  }

  function resizeCanvas(width: number, height: number, id?: number | null) {
    const source = typeof id === "number" ? frameSourceSizes.get(id) : undefined;
    const nextWidth = source?.width ?? (videoScaling ? sourceWidth : width);
    const nextHeight = source?.height ?? (videoScaling ? sourceHeight : height);
    const sourceChanged = sourceWidth !== nextWidth || sourceHeight !== nextHeight;
    const changed = canvas.width !== width || canvas.height !== height || sourceChanged;
    sourceWidth = nextWidth;
    sourceHeight = nextHeight;
    if (lastRequestedSize === `${sourceWidth}x${sourceHeight}`) lastRequestedSize = null;
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    canvas.style.setProperty("--desktop-aspect", String(sourceWidth / sourceHeight));
    updateCanvasDensity();
    if (sourceChanged) pictureZoom.reset();
    else if (changed) pictureZoom.layout();
    if (changed) publishSessionView();
  }

  function publishSessionView() {
    if (!options.onSessionView) return;
    const next: SessionView = {
      mode: isDesktopFullscreen() || windowMode !== "actual" ? "fit" : "actual",
      fullscreen: isDesktopFullscreen(), autoResolution: element("auto-resolution").checked,
      strategy: strategyPreference, width: sourceWidth, height: sourceHeight,
      keyboardCaptured, version: element("software-version").textContent ?? "", output: selectedOutput ?? "",
    };
    if (publishedView && (Object.keys(next) as (keyof SessionView)[]).every((key) => publishedView![key] === next[key])) return;
    publishedView = next;
    options.onSessionView(next);
  }

  function updateCanvasDensity() {
    const ratio = Number.isFinite(window.devicePixelRatio) && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
    canvas.style.setProperty("--desktop-native-width", `${sourceWidth / ratio}px`);
    const matchedDensity = element("auto-resolution").checked && lastAutoResolution === `${sourceWidth}x${sourceHeight}@${ratio}`;
    viewport.classList.toggle("native-pixel-fit", !mobile?.bounds() && desktopMode !== "physical" && matchedDensity);
  }

  function updateControls() {
    publishSessionView();
    viewport.classList.toggle("physical-desktop", desktopMode === "physical");
    mobile?.update();
    pictureZoom.setCover(!!mobile?.isFullscreen() && desktopMode === "physical");
    options.onControls?.({ ready: ready && !desktopPaused, wanted, audioAvailable, audioEnabled: audioEnabled && audioContext?.state === "running", resizeAvailable, desktopMode,
      switchingDesktop, refreshingDesktops, choices: desktopChoices, outputs: desktopOutputs });
    for (const id of controls) element(id).disabled = !ready || desktopPaused;
    for (const id of ["width", "height", "apply-size", "auto-resolution"] as const) element(id).disabled = !ready || desktopPaused || !resizeAvailable;
    element("resize").setAttribute("aria-disabled", String(!ready || !resizeAvailable));
    element("resize").setAttribute("title", translate(resizeAvailable ? "调整虚拟桌面分辨率" : "物理桌面不支持更改显示器分辨率"));
    element("desktop-controls").hidden = desktopChoices.length === 0;
    element("desktop-source").disabled = desktopChoices.length === 0;
    element("desktop-source").value = desktopMode;
    element("monitor-controls").hidden = desktopMode !== "physical" || desktopChoices.length === 0;
    element("monitor-source").disabled = !ready || switchingDesktop || desktopOutputs.length === 0;
    element("audio").disabled = !ready || desktopPaused || !audioAvailable;
    const audioLabel = audioContext && (!audioEnabled || audioContext.state === "running") ? "关闭声音" : "开启声音";
    for (const attribute of ["aria-label", "title", "data-tip"]) element("audio").setAttribute(attribute, translate(audioLabel));
    element("audio").setAttribute("aria-pressed", String(audioEnabled && audioContext?.state === "running"));
    for (const attribute of ["aria-label", "title", "data-tip"]) element("connection").setAttribute(attribute, translate(wanted ? "断开连接" : "重新连接"));
  }

  function send(command: Record<string, unknown>) {
    if (!ready || !socket || socket.readyState !== WebSocket.OPEN) return false;
    if (desktopPaused && ["move", "button", "key", "chord", "scroll", "text", "clipboard_get", "clipboard_set", "clipboard_paste_text", "clipboard_image_start", "clipboard_image_chunk", "clipboard_image_paste", "resize"].includes(String(command.type))) return false;
    if (socket.bufferedAmount > 262144) {
      message("Connection is too slow; reconnecting to release input safely.");
      closeSocket(socket, 4001, "Input backpressure");
      resetSession();
      return false;
    }
    try {
      socket.send(JSON.stringify(command));
      return true;
    } catch {
      closeSocket(socket);
      resetSession();
      return false;
    }
  }

  function release() {
    clipboardKeyEvent = null;
    pendingHardwareKey = null;
    lightBook?.reset();
    if (moveFrame !== null) cancelAnimationFrame(moveFrame);
    moveFrame = null;
    pendingMove = null;
    send({ type: "release" });
    keys.clear(); deferredMeta.clear(); localMeta=false;
    buttons.clear();
    const pointers = [...capturedPointers];
    capturedPointers.clear();
    for (const pointerId of pointers) {
      if (canvas.hasPointerCapture(pointerId)) canvas.releasePointerCapture(pointerId);
    }
  }

  function stopAudio(notify = true) {
    if (notify && audioEnabled && audioSession === null) send({ type: "audio", enabled: false });
    cancelTimer(audioConnectionTimer);
    audioConnectionTimer = null;
    const previousSocket = audioSocket;
    audioSocket = null;
    if (previousSocket) previousSocket.close(1000, "Audio stopped");
    audioEnabled = false;
    cancelAudioResume?.();
    cancelAudioPreparation?.();
    cancelAudioPreparation = null;
    audioWorker?.terminate();
    audioWorker = null;
    if (audioNode) {
      audioNode.onprocessorerror = null;
      audioNode.port.onmessage = null;
      audioNode.port.close();
      audioNode.disconnect();
      audioNode = null;
    }
    audioPlayback = null;
    pendingAudioPackets = 0;
    audioCodec = "pcm";
    for (const source of sources) {
      source.onended = null;
      try { source.stop(); } catch {}
      source.disconnect();
    }
    sources.clear();
    const previous = audioContext;
    audioContext = null;
    nextAudioTime = 0;
    if (previous) { previous.onstatechange = null; previous.close().catch(() => {}); }
    releaseAudioSession?.();
    releaseAudioSession = null;
    updateControls();
  }

  function resetSession() {
    desktopInitializing = false;
    pictureZoom.reset();
    cursorSession = null;
    cursor.reset(); frameCursorModes.clear(); frameSourceSizes.clear();
    videoScaling = false;
    lastVideoBounds = null;
    cancelTimer(videoScaleTimer); videoScaleTimer = null;
    clipboard.reset();
    clipboardSession=null;clipboardSync=false;
    keyChords = false;
    imageClipboard = false; textClipboard = false; clipboardTextInput = false;
    resizeScaleAvailable = false;
    resizeScales = LEGACY_SCALES;
    lastAutoResolution = null;
    lightBook?.reset();
    mobile?.reset();
    discoveryRequest += 1;
    discoveryController?.abort(); discoveryController = null;
    refreshingDesktops = false;
    canvas.hidden = true;
    canvas.style.setProperty("cursor", "default");
    unlockKeyboard();
    stopMetrics();
    cancelTimer(autoResizeTimer);
    autoResizeTimer = null;
    lastRequestedSize = null;
    ready = false;
    controlRelease = false;
    generation += 1;
    pendingFrame = null;
    clipboardPending = false;
    audioAvailable = false;
    audioSession = null;
    frameAcknowledgements = false;
    losslessAvailable = false;
    sequencedFrames = false;
    pendingFrameId = null;
    closeVideo();
    keys.clear(); deferredMeta.clear(); localMeta=false;
    buttons.clear();
    release();
    stopAudio(false);
    updateControls();
  }

  function waitForClose(current: WebSocket) {
    // A Close frame can be lost by a proxy. Desktop switches still require
    // explicit control release. A failed transport may retry: the server's
    // exclusive-controller gate rejects it until the old connection expires.
    if (wanted && current === socket && closeTimer === null) {
      closeTimer = setTimeout(() => {
        closeTimer = null;
        if (current !== socket) return;
        cancelTimer(connectionTimer);
        connectionTimer = null;
        current.onopen = current.onmessage = current.onerror = current.onclose = null;
        current.close(4000, "Control release timeout");
        socket = null;
        releasingSocket = null;
        connectedAt = null;
        const releaseTimedOut = switchingDesktop;
        switchingDesktop = false;
        if (releaseTimedOut) wanted = false;
        connectionFailure = releaseTimedOut
          ? "Previous connection did not close within 3 seconds. Reconnect or select another desktop."
          : "Connection close timed out; retrying.";
        resetSession();
        setText("state", "Disconnected");
        desktopStatus(connectionFailure);
        message(connectionFailure);
        if (!releaseTimedOut) scheduleRetry();
      }, 3000);
    }
  }

  function closeSocket(current: WebSocket, code = 1000, reason = "") {
    waitForClose(current);
    current.close(code, reason);
  }

  function completeRelease(current: WebSocket) {
    if (current !== socket || current !== releasingSocket || !switchingDesktop) return;
    cancelTimer(closeTimer);
    closeTimer = null;
    cancelTimer(connectionTimer);
    connectionTimer = null;
    current.onopen = current.onmessage = current.onerror = current.onclose = null;
    releasingSocket = null;
    socket = null;
    switchingDesktop = false;
    current.close(1000, "Switch desktop");
    updateControls();
    connect();
  }

  function armReadinessTimeout(current: WebSocket, delay: number) {
    cancelTimer(connectionTimer);
    connectionTimer = setTimeout(() => {
      connectionTimer = null;
      if (socket !== current || ready) return;
      if (desktopInitializing) wanted = false;
      connectionFailure = desktopInitializing
        ? "Desktop initialization timed out. Reconnect to try again."
        : "Desktop readiness timed out; reconnecting.";
      message(connectionFailure);
      desktopStatus(`${desktopLabel()}连接超时，可切回虚拟桌面。`);
      closeSocket(current, 4000, "Readiness timeout");
      resetSession();
    }, delay);
  }

  function connect() {
    if (!wanted || destroyed || socket || retryTimer !== null) return;
    if (navigator.onLine === false) { setText("state", "Waiting for network"); return; }
    setText("state", "Connecting");
    const url = new URL("./ws", location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    if (desktopConfirmed) url.searchParams.set("desktop", desktopMode);
    if (desktopMode === "physical" && selectedOutput) url.searchParams.set("output", selectedOutput);
    if (desktopChoices.length) desktopStatus(`正在连接${desktopLabel()}，尚未确认${selectedOutput ? ` · ${selectedOutput}` : ""}。`);
    let current;
    try { current = new WebSocket(url); } catch (error) {
      connectionFailure = `Could not connect: ${errorMessage(error)}`;
      message(connectionFailure);
      scheduleRetry();
      return;
    }
    socket = current;
    armReadinessTimeout(current, 15000);
    current.binaryType = "arraybuffer";
    current.onopen = () => {
      if (current !== socket || !wanted || destroyed) { current.close(); return; }
      setText("state", "Waiting for desktop");
      message("Connected; waiting for server readiness.");
    };
    current.onmessage = (event) => {
      if (current !== socket || !wanted || destroyed) return;
      lastServerActivity = performance.now();
      if (switchingDesktop) {
        if (typeof event.data === "string") {
          try { if (JSON.parse(event.data)?.type === "released") completeRelease(current); } catch {}
        }
        return;
      }
      if (typeof event.data === "string") {
        receiveJSON(event.data);
      } else if (ready && !desktopPaused && event.data instanceof ArrayBuffer && event.data.byteLength > 1) {
        const kind = new Uint8Array(event.data, 0, 1)[0];
        if (kind === 1 && event.data.byteLength <= 16777216) {
          pendingFrame = { data: event.data.slice(1), generation, videoToken, id: pendingFrameId };
          pendingFrameId = null;
          void decodeFrames();
        } else if (kind === 2) {
          if (audioSession === null) receiveAudio(event.data);
        } else if (kind === 3 && event.data.byteLength <= 16777216) {
          const id = pendingFrameId;
          pendingFrameId = null;
          receiveVideo(event.data, id);
        } else if (kind === 4) {
          const id = pendingFrameId; pendingFrameId = null;
          receiveDesktopUpdate(event.data, id);
        }
      }
    };
    current.onerror = () => {
      if (current !== socket || switchingDesktop) return;
      desktopStatus(`${desktopLabel()}连接失败，请检查会话是否可用，或切回虚拟桌面。`);
      connectionFailure ||= "Connection failed. Check browser authentication, server availability, or another active controller.";
      message(connectionFailure);
      closeSocket(current);
    };
    current.onclose = () => {
      if (current !== socket) return;
      cancelTimer(connectionTimer);
      connectionTimer = null;
      cancelTimer(closeTimer);
      closeTimer = null;
      socket = null;
      releasingSocket = null;
      if (connectedAt !== null && performance.now() - connectedAt >= 10_000) retries = 0;
      connectedAt = null;
      if (desktopInitializing && !ready && !switchingDesktop) {
        wanted = false;
        connectionFailure ||= "Desktop initialization stopped. Reconnect to try again.";
        message(connectionFailure);
        desktopStatus(connectionFailure);
      }
      resetSession();
      setText("state", "Disconnected");
      if (switchingDesktop) {
        switchingDesktop = false;
        updateControls();
        connect();
      } else scheduleRetry();
    };
  }

  function scheduleRetry() {
    if (!wanted || destroyed || retryTimer !== null) return;
    if (navigator.onLine === false) { setText("state", "Waiting for network"); return; }
    if (connectionFailure) desktopStatus(connectionFailure);
    const base = Math.min(30_000, 500 * 2 ** Math.min(retries++, 6));
    const delay = Math.round(Math.min(30_000, base * (0.8 + Math.random() * 0.4)));
    setText("state", `Reconnecting in ${delay / 1000}s`);
    retryTimer = setTimeout(() => {
      retryTimer = null;
      connect();
    }, delay);
  }

  function disconnect() {
    wanted = false;
    switchingDesktop = false;
    releasingSocket = null;
    connectionFailure = "";
    cancelTimer(closeTimer);
    closeTimer = null;
    cancelTimer(retryTimer);
    retryTimer = null;
    cancelTimer(connectionTimer);
    connectionTimer = null;
    release();
    stopAudio();
    resetSession();
    if (socket) socket.close(1000, "Client disconnected");
    setText("state", "Disconnected");
    updateControls();
  }

  function receiveJSON(raw: string) {
    let value;
    try { value = JSON.parse(raw); } catch { message("Invalid server JSON."); return; }
    if (!value || typeof value !== "object") return;
    if (clipboard.receive(value)) return;
    if (value.type === "desktop_initializing" && !ready && socket) {
      desktopInitializing = true;
      // Portal consent and first-frame capture have a bounded server deadline.
      // A broken initialization must not automatically reopen consent.
      armReadinessTimeout(socket, 60000);
    } else if (value.type === "ready") {
      const { width, height } = value;
      if (!validDimensions(width, height)) {
        message("Server supplied an invalid desktop size.");
        return;
      }
      sourceWidth = width; sourceHeight = height;
      resizeCanvas(width, height);
      cursorEmbedded = value.cursor === "embedded";
      cursor.present(cursorEmbedded);
      if (document.activeElement !== element("width") && document.activeElement !== element("height")) {
        element("width").value = String(width);
        element("height").value = String(height);
      }
      ready = true;
      desktopInitializing = false;
      desktopPaused = false;
      keyChords = value.key_chords === true;
      imageClipboard = value.image_clipboard === true;
      textClipboard = value.text_clipboard === true;
      clipboardTextInput = value.clipboard_text_input === true;
      clipboardSession=typeof value.clipboard_session==="string"&&/^[0-9a-f]{32}$/.test(value.clipboard_session)?value.clipboard_session:null;
      clipboardSync=value.clipboard_sync===true;
      resizeScaleAvailable = value.resize_scale === true;
      resizeScales = Array.isArray(value.resize_scales) ? CAPTURE_SCALES.filter(scale => value.resize_scales.includes(scale)) : LEGACY_SCALES;
      lastAutoResolution = null;
      controlRelease = value.control_release === true;
      connectionFailure = "";
      if (typeof value.version === "string" && value.version.length <= 64 && /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?$/.test(value.version)) {
        setText("software-version", `v${value.version}`);
      }
      desktopMode = value.desktop === "physical" ? "physical" : "virtual";
      desktopConfirmed = true;
      updateCanvasDensity();
      selectedOutput = desktopMode === "physical" && typeof value.output === "string" ? value.output : null;
      resizeAvailable = desktopMode !== "physical" && value.resize !== false;
      desktopStatus(`已连接${desktopLabel()}${selectedOutput ? ` · ${selectedOutput}` : ""}。`);
      populateDesktops(value);
      if (value.metrics === true) {
        stopMetrics();
        metricsStarted = performance.now();
        sampleMetrics();
      }
      frameAcknowledgements = value.frame_ack === true;
      sequencedFrames = value.frame_ids === true;
      losslessAvailable = value.lossless_updates === true && frameAcknowledgements && sequencedFrames;
      serverVideoCodecs = Array.isArray(value.video_codecs) ? value.video_codecs.filter((codec: unknown) => typeof codec === "string") : [];
      backendSelection = value.backend_selection === true;
      videoRequestIds = value.video_request_ids === true;
      videoScaling = value.video_scaling === true && frameAcknowledgements && sequencedFrames && !losslessAvailable;
      lastVideoBounds = null;
      videoRequestId = 0;
      activeBackend = null; waitingForBackend = false; rejectedBackends.clear();
      configureStreamStrategy();
      cancelTimer(connectionTimer);
      connectionTimer = null;
      connectedAt = performance.now();
      audioAvailable = value.audio === true;
      audioSession = Number.isSafeInteger(value.audio_session) && value.audio_session > 0 ? value.audio_session : null;
      opusAvailable = Array.isArray(value.audio_codecs) && value.audio_codecs.includes("opus");
      if (!audioAvailable) stopAudio(false);
      setText("state", "Connected");
      message(audioAvailable ? `Desktop ready. Audio is ${audioEnabled ? "on" : "off until enabled"}.` : "Desktop ready. Server audio is unavailable.");
      requestPreferredVideo(true);
      configureVideoScale();
      if (losslessAvailable) send({ type: "lossless_updates", enabled: true });
      if (sequencedFrames && frameAcknowledgements && typeof value.cursor_session === "string" && /^[0-9a-f]{32}$/.test(value.cursor_session)) {
        cursorSession = value.cursor_session;
        cursor.connect(value.cursor_session);
      }
      clipboard.connect(clipboardSession,clipboardSync);
      updateControls();
      if (document.activeElement === canvas) mobile?.focusDesktop();
      scheduleAutoResize();
      void captureKeyboard();
    } else if (value.type === "codec_capabilities") {
      const hadH264 = serverVideoCodecs.includes("h264");
      serverVideoCodecs = Array.isArray(value.video_codecs) ? value.video_codecs.filter((codec: unknown) => typeof codec === "string") : [];
      // A background probe may enable H.264 after ready. Honor the saved codec
      // and backend choices through the normal negotiation path.
      if (ready && !hadH264 && serverVideoCodecs.includes("h264") && videoPreference !== "jpeg") requestPreferredVideo();
    } else if (value.type === "desktop_state" && typeof value.available === "boolean") {
      desktopPaused = !value.available;
      if (desktopPaused) {
        lightBook?.reset();
        cursor.reset(); frameCursorModes.clear(); frameSourceSizes.clear();
        clipboard.reset();
        closeVideo();
        pendingFrame = null;
        pendingFrameId = null;
        drawing?.clearRect(0, 0, canvas.width, canvas.height);
        keys.clear(); deferredMeta.clear(); localMeta=false; buttons.clear();
        desktopStatus("Desktop locked or temporarily unavailable.");
      } else {
        if (cursorSession) cursor.connect(cursorSession);
        else cursor.present(cursorEmbedded);
        clipboard.connect(clipboardSession,clipboardSync);
        desktopStatus(`已连接${desktopLabel()}${selectedOutput ? ` · ${selectedOutput}` : ""}。`);
      }
      updateControls();
    } else if (value.type === "pong" && pendingPing && value.id === pendingPing.id) {
      latestRtt = Math.max(0, performance.now() - pendingPing.started);
      rttSamples.push(latestRtt);
      if (value.video_metrics && typeof value.video_metrics === "object") pipelineMetrics = value.video_metrics;
      if (rttSamples.length > 30) rttSamples.shift();
      pendingPing = null;
    } else if (value.type === "frame_id" && Number.isSafeInteger(value.id) && value.id > 0) {
      pendingFrameId = value.id;
      if (typeof value.cursor_embedded === "boolean") frameCursorModes.set(value.id, value.cursor_embedded);
      if (videoScaling && validDimensions(value.source_width, value.source_height)) {
        frameSourceSizes.set(value.id, { width: value.source_width, height: value.source_height });
        // Bound metadata retained while an asynchronous decode is pending.
        while (frameSourceSizes.size > 120) frameSourceSizes.delete(frameSourceSizes.keys().next().value!);
      }
    } else if (value.type === "video_config" && typeof value.codec === "string") {
      void configureVideo(value);
    } else if (value.type === "clipboard" && typeof value.text === "string") {
      if (!clipboardPending) return;
      clipboardPending = false;
      if (validText(value.text)) {
        textBox.value = value.text;
        message("Remote clipboard loaded into the text box.");
      }
    } else if (value.type === "error" && typeof value.message === "string") {
      if (!ready) {
        connectionFailure = `Server: ${value.message.slice(0, 2048)}`;
        if (value.retryable === false) {
          wanted = false;
          cancelTimer(connectionTimer); connectionTimer = null;
          cancelTimer(retryTimer); retryTimer = null;
          updateControls();
        }
      }
      message(`Server: ${value.message.slice(0, 2048)}`);
      desktopStatus(`${desktopLabel()}：${value.message.slice(0, 2048)}`);
    }
  }

  async function decodeFrames() {
    if (decoding) return;
    decoding = true;
    try {
      while (pendingFrame) {
        const frame = pendingFrame;
        pendingFrame = null;
        let bitmap;
        const started = performance.now();
        try {
          bitmap = await createImageBitmap(new Blob([frame.data], { type: "image/jpeg" }));
          if (frame.generation === generation && frame.videoToken === videoToken && ready && !desktopPaused && !destroyed) {
            if(!cursorFreeFrame(frame.id))continue;
            if (!validDimensions(bitmap.width, bitmap.height)) {
              message("Server supplied invalid JPEG dimensions.");
              continue;
            }
            resizeCanvas(bitmap.width, bitmap.height, frame.id);
            drawing?.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
            canvas.hidden = false;
            recordFrame("JPEG", started, frame.id);
          }
        } catch {
          if (frame.generation === generation && ready) message("Could not decode a desktop JPEG frame.");
        } finally {
          if (bitmap) bitmap.close();
          if (frame.generation === generation && frame.videoToken === videoToken && ready && frameAcknowledgements) acknowledgeFrame(frame.id);
        }
      }
    } finally { decoding = false; }
  }

  function receiveDesktopUpdate(data: ArrayBuffer, id: number | null) {
    const token = videoToken, session = generation;
    const active = () => token === videoToken && session === generation && ready && !destroyed;
    losslessChain = losslessChain.catch(() => {}).then(async () => {
      if (!active()) return;
      if(!cursorFreeFrame(id))return;
      const started = performance.now();
      const update = parseDesktopUpdate(data);
      if (update.width !== canvas.width || update.height !== canvas.height) throw Error("Desktop update does not match the canvas");
      const tiles = await decodeDesktopTiles(update.tiles);
      try {
        if (!active()) return;
        for (const tile of tiles) drawing?.drawImage(tile.image, tile.x, tile.y);
        canvas.hidden = false;
        recordFrame("PNG", started, id);
      } finally { for (const tile of tiles) tile.image.close(); }
    }).catch(() => {
      if (!active()) return;
      message("Could not decode a lossless desktop update.");
      if (socket) closeSocket(socket, 4001, "Invalid desktop update");
      resetSession();
    }).finally(() => { if (session === generation && ready) acknowledgeFrame(id); });
  }

  function closeVideo() {
    cancelTimer(videoTimer);
    videoTimer = null;
    videoToken += 1;
    configuringVideo = false;
    pendingVideo = null;
    videoFrameIds.clear();
    videoDecodeStarted.clear();
    const previous = videoDecoder;
    videoDecoder = null;
    if (previous && previous.state !== "closed") previous.close();
  }

  function acknowledgeFrame(id: number | null | undefined) {
    if (sequencedFrames) {
      if (typeof id === "number" && Number.isSafeInteger(id) && id > 0) send({ type: "frame_ack", id });
    } else send({ type: "frame_ack" });
  }

  function configureStreamStrategy() {
    if (ready && frameAcknowledgements) send({ type: "video_flow", enabled: true, sequenced: sequencedFrames, strategy: strategyPreference });
  }

  function configureVideoScale() {
    if (!ready || !videoScaling || document.hidden || destroyed) return;
    const bounds = mobile?.bounds();
    const native = strategyPreference === "quality_first" || pictureZoom.active() || (!isDesktopFullscreen() && windowMode === "actual");
    if (!native && mobile?.keyboardActive()) return;
    let width = bounds?.width ?? viewport.clientWidth;
    let height = bounds ? bounds.height - bounds.bar : viewport.clientHeight;
    if (pictureRotated) [width, height] = [height, width];
    const maxSize = native ? null : encodingBounds(width, height, window.devicePixelRatio);
    const key = JSON.stringify(maxSize);
    if (key !== lastVideoBounds && send({ type: "video_scale", max_size: maxSize })) lastVideoBounds = key;
  }

  function scheduleVideoScale() {
    if (!ready || !videoScaling) return;
    cancelTimer(videoScaleTimer);
    videoScaleTimer = setTimeout(() => { videoScaleTimer = null; configureVideoScale(); }, 250);
  }

  function requestVideo(codec: string, rejected?: string) {
    closeVideo();
    activeBackend = null;
    waitingForBackend = true;
    videoRequestId += 1;
    send({ type: "video", codec,
      ...(backendSelection && codec === "h264" ? { backend: backendPreference } : {}),
      ...(rejected ? { rejected_backend: rejected } : {}),
      ...(videoRequestIds ? { request_id: videoRequestId } : {}),
    });
  }

  function requestPreferredVideo(initial = false) {
    const available = frameAcknowledgements && sequencedFrames && serverVideoCodecs.includes("h264") && window.VideoDecoder && window.EncodedVideoChunk;
    if (videoPreference !== "jpeg" && available) {
      requestVideo("h264");
    } else {
      if (!initial || videoPreference !== "auto") requestVideo("jpeg");
      if (videoPreference === "h264") message("H.264 is unavailable; using compatible JPEG video.");
    }
  }

  listen(element("video-codec"), "change", () => {
    videoPreference = videoCodecs.includes(element("video-codec").value as typeof videoPreference) ? element("video-codec").value as typeof videoPreference : "auto";
    savePreference(videoCodecKey, videoPreference);
    if (ready) { closeVideo(); requestPreferredVideo(); }
  });
  listen(element("encoder-backend"), "change", () => {
    backendPreference = encoderBackends.includes(element("encoder-backend").value as typeof backendPreference) ? element("encoder-backend").value as typeof backendPreference : "auto";
    savePreference(encoderBackendKey, backendPreference);
    rejectedBackends.clear();
    if (ready) { closeVideo(); requestPreferredVideo(); }
  });
  listen(element("stream-strategy"), "change", () => {
    strategyPreference = streamStrategies.includes(element("stream-strategy").value as typeof strategyPreference) ? element("stream-strategy").value as typeof strategyPreference : "auto";
    savePreference(streamStrategyKey, strategyPreference);
    configureStreamStrategy();
    configureVideoScale();
    publishSessionView();
  });

  function fallbackVideo() {
    closeVideo();
    if (backendSelection && activeBackend && !rejectedBackends.has(activeBackend)) {
      rejectedBackends.add(activeBackend);
      requestVideo("h264", activeBackend);
      return;
    }
    requestVideo("jpeg");
    message("H.264 is unavailable; using compatible JPEG video.");
  }

  async function configureVideo(value: {codec: string; width: number; height: number; backend?: string; fallback_reason?: string; request_id?: number}) {
    if (videoRequestIds && value.request_id !== videoRequestId) return;
    closeVideo();
    pendingFrame = null;
    waitingForBackend = false;
    activeBackend = typeof value.backend === "string" && ["openh264", "turbojpeg", ...encoderBackends].includes(value.backend) ? value.backend : null;
    setText("encoder-status", [activeBackend, typeof value.fallback_reason === "string" ? value.fallback_reason : ""].filter(Boolean).join(" · "));
    if (value.codec === "jpeg") { message("Video: JPEG compatibility mode."); return; }
    if (!/^avc1\.[0-9a-f]{6}$/i.test(value.codec) || !window.VideoDecoder || !validDimensions(value.width, value.height)) { fallbackVideo(); return; }
    const token = videoToken;
    const session = generation;
    configuringVideo = true;
    try {
      const config = await supportedVideoConfig(window.VideoDecoder, value.codec, value.width, value.height);
      if (token !== videoToken || session !== generation || !ready || desktopPaused || destroyed) return;
      if (!config) { fallbackVideo(); return; }
      videoDecoder = new window.VideoDecoder({
        output(frame) {
          try {
            if (token !== videoToken || session !== generation || !ready || desktopPaused || destroyed) return;
            cancelTimer(videoTimer);
            videoTimer = null;
            if (!validDimensions(frame.displayWidth, frame.displayHeight)) { fallbackVideo(); return; }
            if(cursorFreeFrame(videoFrameIds.get(frame.timestamp))) {
              resizeCanvas(frame.displayWidth, frame.displayHeight, videoFrameIds.get(frame.timestamp));
              drawing?.drawImage(frame, 0, 0, canvas.width, canvas.height);
              canvas.hidden = false;
              recordFrame("H.264", videoDecodeStarted.get(frame.timestamp), videoFrameIds.get(frame.timestamp));
            }
            videoDecodeStarted.delete(frame.timestamp);
            const id = videoFrameIds.get(frame.timestamp);
            videoFrameIds.delete(frame.timestamp);
            updateVideoWatchdog();
            if (frameAcknowledgements) acknowledgeFrame(id);
          } finally { frame.close(); }
        },
        error() { if (token === videoToken && session === generation && ready) fallbackVideo(); },
      });
      videoDecoder.configure(config);
      configuringVideo = false;
      const packet = pendingVideo;
      pendingVideo = null;
      if (packet) receiveVideo(packet.data, packet.id);
      message("Video: low-latency H.264.");
    } catch { if (token === videoToken && session === generation && ready) fallbackVideo(); }
  }

  function updateVideoWatchdog() {
    cancelTimer(videoTimer);
    videoTimer = null;
    if (videoFrameIds.size === 0) return;
    const token = videoToken;
    const session = generation;
    videoTimer = setTimeout(() => {
      videoTimer = null;
      if (token === videoToken && session === generation && ready && !destroyed) fallbackVideo();
    }, 2000);
  }

  function receiveVideo(data: ArrayBuffer, id: number | null) {
    if (waitingForBackend) return;
    if (data.byteLength <= 10) { fallbackVideo(); return; }
    if (configuringVideo) {
      if (pendingVideo) { fallbackVideo(); return; }
      pendingVideo = { data, id };
      return;
    }
    if (!videoDecoder || videoDecoder.state !== "configured" || videoDecoder.decodeQueueSize > 2) { fallbackVideo(); return; }
    const view = new DataView(data);
    const timestamp = view.getUint32(2, true) + view.getUint32(6, true) * 4294967296;
    try {
      videoFrameIds.set(timestamp, id);
      videoDecodeStarted.set(timestamp, performance.now());
      updateVideoWatchdog();
      videoDecoder.decode(new window.EncodedVideoChunk({ type: view.getUint8(1) === 1 ? "key" : "delta", timestamp, data: new Uint8Array(data, 10) }));
    } catch { fallbackVideo(); }
  }

  function receiveAudio(data: ArrayBuffer) {
    if (!audioEnabled || !audioContext || audioContext.state !== "running" || document.hidden) return;
    if (audioWorker) {
      if (pendingAudioPackets >= 6) return;
      pendingAudioPackets += 1;
      audioWorker.postMessage({ type: "packet", buffer: data }, [data]);
      return;
    }
    if (sources.size >= 64) return;
    const bytes = data.byteLength - 1;
    if (bytes % 8 !== 0 || bytes === 0 || bytes > 96000) return;
    const frames = bytes / 8;
    const start = Math.max(audioContext.currentTime + 0.015, nextAudioTime);
    if (start + frames / 48000 > audioContext.currentTime + 0.3) return;
    const buffer = audioContext.createBuffer(2, frames, 48000);
    const view = new DataView(data);
    for (let channel = 0; channel < 2; channel += 1) {
      const samples = buffer.getChannelData(channel);
      for (let index = 0; index < frames; index += 1) {
        const sample = view.getFloat32(1 + (index * 2 + channel) * 4, true);
        samples[index] = Number.isFinite(sample) ? Math.max(-1, Math.min(1, sample)) : 0;
      }
    }
    const source = audioContext.createBufferSource();
    source.buffer = buffer;
    source.connect(audioContext.destination);
    source.onended = () => { sources.delete(source); source.disconnect(); };
    sources.add(source);
    source.start(start);
    nextAudioTime = start + buffer.duration;
  }

  function prepareAudio(context: AudioContext, session: number): Promise<string> {
    if (!context.audioWorklet || typeof Worker === "undefined" || typeof AudioWorkletNode === "undefined") return Promise.resolve("pcm");
    return new Promise((resolve, reject) => {
      let initialized = false;
      const active = () => session === generation && audioContext === context && ready && !document.hidden;
      const finish = (error?: Error, codec = "pcm") => {
        cancelTimer(audioConnectionTimer);
        audioConnectionTimer = null;
        cancelAudioPreparation = null;
        if (error) reject(error);
        else { initialized = true; resolve(codec); }
      };
      cancelAudioPreparation = () => finish(new Error("Audio initialization cancelled"));
      const failed = () => {
        if (!active()) return;
        if (!initialized) finish(new Error("Audio playback initialization failed"));
        else {
          stopAudio();
          message("Audio unavailable: Audio connection stopped; click to retry.");
        }
      };
      audioConnectionTimer = setTimeout(failed, 5000);
      void context.audioWorklet.addModule(new URL(audioWorkletUrl, location.href).href).then(() => {
        if (!active() || !cancelAudioPreparation) return;
        const node = new AudioWorkletNode(context, "remote-audio", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
        audioNode = node;
        node.connect(context.destination);
        node.onprocessorerror = failed;
        node.port.onmessage = ({ data }) => {
          if (active() && audioNode === node && Number.isFinite(data?.bufferMs)) audioPlayback = data;
        };
        const worker = new Worker(new URL(audioWorkerUrl, location.href), { type: "module" });
        audioWorker = worker;
        const channel = new MessageChannel();
        node.port.postMessage({ port: channel.port1 }, [channel.port1]);
        worker.onmessage = ({ data }) => {
          if (!active() || audioWorker !== worker) return;
          if (data?.type === "ready" && !initialized) finish(undefined, data.codec === "opus" ? "opus" : "pcm");
          else if (data?.type === "consumed") pendingAudioPackets = Math.max(0, pendingAudioPackets - 1);
          else if (data?.type === "error") failed();
        };
        worker.onerror = failed;
        worker.postMessage({ type: "init", port: channel.port2, allowOpus: opusAvailable && audioSession !== null }, [channel.port2]);
      }).catch(failed);
    });
  }

  async function toggleAudio() {
    if (audioContext) {
      if (audioEnabled && audioContext.state !== "running") resumeInterruptedAudio();
      else stopAudio();
      return;
    }
    if (!ready || !audioAvailable) return;
    const session = generation;
    let context: AudioContext | undefined;
    try {
      const Audio = window.AudioContext || window.webkitAudioContext;
      // Web Audio defaults to ambient on iOS. Remote media is explicit playback,
      // so it should use the media volume/category rather than the ringer switch.
      try {
        const session = navigator.audioSession;
        if (session) {
          const previous = session.type;
          session.type = "playback";
          releaseAudioSession = () => { try { if (session.type === "playback") session.type = previous; } catch {} };
        }
      } catch { /* Optional API; other browsers retain normal Web Audio playback. */ }
      context = new Audio({ sampleRate: 48000 });
      audioContext = context;
      context.onstatechange = () => {
        if (!context || audioContext !== context) return;
        if (context.state !== "running") {
          audioNode?.port.postMessage({ type: "reset" });
          for (const source of sources) { try { source.stop(); } catch {} source.disconnect(); }
          sources.clear(); nextAudioTime = 0;
        }
        updateControls();
      };
      updateControls();
      await resumeAudioContext(context);
      if (session === generation && audioContext === context && ready && !document.hidden) {
        audioCodec = await prepareAudio(context, session);
      }
      if (session !== generation || audioContext !== context || !ready || document.hidden) {
        if (audioContext === context) stopAudio();
        return;
      }
      if (audioSession === null) {
        audioEnabled = send({ type: "audio", enabled: true });
        if (!audioEnabled) stopAudio(false);
      } else {
        const url = new URL("./audio", location.href);
        url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
        url.searchParams.set("session", String(audioSession));
        if (audioCodec === "opus") url.searchParams.set("codec", "opus");
        const current = new WebSocket(url);
        audioSocket = current;
        current.binaryType = "arraybuffer";
        const active = () => current === audioSocket && session === generation && audioContext === context && ready;
        const failed = () => {
          if (!active()) return;
          stopAudio(false);
          message("Audio unavailable: Audio connection stopped; click to retry.");
        };
        audioConnectionTimer = setTimeout(failed, 5000);
        current.onmessage = (event) => {
          if (!active()) return;
          if (typeof event.data === "string") {
            let value;
            try { value = JSON.parse(event.data); } catch { failed(); return; }
            if (value.type === "audio_ready") {
              if (value.codec && value.codec !== audioCodec) { failed(); return; }
              cancelTimer(audioConnectionTimer);
              audioConnectionTimer = null;
              audioEnabled = true;
              updateControls();
            } else if (value.type === "error") failed();
          } else if (event.data instanceof ArrayBuffer && [2, 4].includes(new Uint8Array(event.data)[0])) {
            receiveAudio(event.data);
          }
        };
        current.onerror = failed;
        current.onclose = failed;
      }
      updateControls();
    } catch (error) {
      if (audioContext === context || (!context && session === generation)) {
        stopAudio(false);
        message(`Audio unavailable: ${errorMessage(error)}`);
      }
    }
  }

  function resumeAudioContext(context: AudioContext): Promise<void> {
    return new Promise((resolve, reject) => {
      const finish = (error?: unknown) => {
        if (cancelAudioResume !== cancel) return;
        clearTimeout(timer); cancelAudioResume = null;
        if (error) reject(error); else resolve();
      };
      const cancel = () => finish(new Error("Audio initialization cancelled"));
      const timer = setTimeout(() => finish(new Error("Audio playback did not start; tap audio to retry.")), 5000);
      cancelAudioResume = cancel;
      // Invoke synchronously inside the trusted gesture, before any await.
      try {
        void context.resume().then(() => finish(context.state === "running" ? undefined : new Error("Audio playback is paused; tap audio to retry.")), finish);
      } catch (error) { finish(error); }
    });
  }

  function resumeInterruptedAudio() {
    const context = audioContext;
    if (!context || !audioEnabled || context.state === "running" || cancelAudioResume || document.hidden) return;
    void resumeAudioContext(context).then(() => {
      if (audioContext === context) updateControls();
    }).catch((error) => {
      if (audioContext !== context) return;
      stopAudio(); message(`Audio unavailable: ${errorMessage(error)}`);
    });
  }
  function resumeAudioOnGesture(event: Event) {
    if (event.target === element("audio") || element("audio").contains?.(event.target as Node) || (event.target as Element)?.closest?.('[data-mobile-action="audio"]')) return;
    resumeInterruptedAudio();
  }
  listen(document, "pointerdown", resumeAudioOnGesture, { capture: true, passive: true });
  listen(document, "keydown", resumeAudioOnGesture, { capture: true });

  function validText(text: string, limit = 65536) {
    if (new TextEncoder().encode(text).byteLength <= limit) return true;
    message(`Text is too large (maximum ${limit / 1024} KiB UTF-8).`);
    return false;
  }

  function typeText(text: string) {
    if (!ready || !text || !validText(text, 16384)) return;
    release();
    const sent = clipboardTextInput
      ? clipboard.send({type:"clipboard_paste_text",text,shift:false})
      : send({ type: "text", text });
    if (sent) message("Text sent.");
  }

  function pasteText(text: string, shift=localPasteShift) {
    if (!textClipboard && !imageClipboard) { typeText(text); return; }
    if (!ready || !text || !validText(text)) return;
    release();
    clipboard.send({ type: "clipboard_paste_text", text, shift });
  }

  const capturedPointers = new Set<number>();
  function syncModifiers(event: KeyboardEvent | PointerEvent, flushMeta = true) {
    if (flushMeta) localMeta=false;
    for (const [flag, left, right] of [
      ["ctrlKey", "ControlLeft", "ControlRight"], ["shiftKey", "ShiftLeft", "ShiftRight"],
      ["altKey", "AltLeft", "AltRight"], ["metaKey", "MetaLeft", "MetaRight"],
    ] as const) {
      if (flag === "metaKey" && localMeta && !flushMeta) continue;
      if (flag === "metaKey" && deferredMeta.size) {
        if (!event.metaKey) deferredMeta.clear();
        else if (!flushMeta) continue;
        else {
          for (const code of deferredMeta) if (send({type:"key",code,pressed:true})) keys.add(code);
          deferredMeta.clear();
        }
      }
      if (event[flag]) {
        if (!("code" in event && (event.code === left || event.code === right)) && !keys.has(KEY_CODES[left]) && !keys.has(KEY_CODES[right])) {
          if (send({ type: "key", code: KEY_CODES[left], pressed: true })) keys.add(KEY_CODES[left]);
        }
      } else {
        for (const code of [KEY_CODES[left], KEY_CODES[right]]) {
          if (keys.delete(code)) send({ type: "key", code, pressed: false });
        }
      }
    }
  }

  function move(event: Pick<PointerEvent, "clientX" | "clientY">) {
    cursor.move(event.clientX, event.clientY, mobile?.bounds() ? "touch" : "mouse");
    const rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    const point = pictureNormalizedPoint(rect, { x: event.clientX, y: event.clientY }, pictureRotated);
    send({ type: "move", x: Math.max(0, Math.min(1, point.x)), y: Math.max(0, Math.min(1, point.y)) });
  }

  function flushMove() {
    if (moveFrame !== null) cancelAnimationFrame(moveFrame);
    moveFrame = null;
    const event = pendingMove;
    pendingMove = null;
    if (event) move(event);
  }

  listen(toolHandle, "click", (event) => {
    if (suppressToolClick && event.detail !== 0) {
      suppressToolClick = false; event.preventDefault(); event.stopImmediatePropagation(); return;
    }
    if (!options.onToolsOpen) setToolsOpen(Boolean(element("tool-panel").hidden), true);
  }, { capture: true });
  listen(toolHandle, "pointerdown", (event) => {
    if (event.button !== 0 || toolDrag) return;
    suppressToolClick = false;
    const bounds = toolHandle.getBoundingClientRect();
    toolDrag = { id: event.pointerId, startX: event.clientX, startY: event.clientY,
      offsetX: event.clientX - bounds.left, offsetY: event.clientY - bounds.top,
      x: bounds.left, y: bounds.top, moved: false };
    toolHandle.setPointerCapture(event.pointerId);
  });
  listen(toolHandle, "pointermove", (event) => {
    if (!toolDrag || toolDrag.id !== event.pointerId) return;
    if (event.buttons === 0) { finishToolDrag(event); return; }
    if (!toolDrag.moved && Math.hypot(event.clientX - toolDrag.startX, event.clientY - toolDrag.startY) < 6) return;
    toolDrag.x = event.clientX - toolDrag.offsetX;
    toolDrag.y = event.clientY - toolDrag.offsetY;
    if (!toolDrag.moved) {
      toolDrag.moved = true;
      tools.setAttribute("data-dragging", "true");
      setToolsOpen(false);
    }
    layoutTools();
  });
  function finishToolDrag(event: PointerEvent) {
    if (!toolDrag || toolDrag.id !== event.pointerId) return;
    const drag = toolDrag;
    toolDrag = null;
    tools.removeAttribute("data-dragging");
    suppressToolClick = drag.moved;
    if (drag.moved && event.type !== "pointercancel") {
      const bounds = mobile?.bounds();
      const width = bounds?.width || window.innerWidth || 1280;
      const height = bounds ? bounds.height - bounds.bar : window.innerHeight || 720;
      const handleWidth = toolHandle.offsetWidth || 40, handleHeight = toolHandle.offsetHeight || 40;
      const maxX = Math.max(12, width - handleWidth - 12), maxY = Math.max(12, height - handleHeight - 12);
      // Capture-loss events can omit coordinates. Snap from the last visible
      // drag position instead of reverting to the previous dock or using 0,0.
      const left = Math.max(12, Math.min(maxX, drag.x - (bounds?.left ?? 0)));
      const top = Math.max(12, Math.min(maxY, drag.y - (bounds?.top ?? 0)));
      const edge = nearestToolEdge(left + handleWidth / 2, top + handleHeight / 2, width, height);
      const horizontal = edge === "top" || edge === "bottom";
      const ratio = horizontal ? (left - 12) / Math.max(1, maxX - 12) : (top - 12) / Math.max(1, maxY - 12);
      toolPosition = { edge, ratio: Math.max(0, Math.min(1, ratio)) };
      saveToolPosition();
    } else layoutTools();
    if (toolHandle.hasPointerCapture(event.pointerId)) toolHandle.releasePointerCapture(event.pointerId);
  }
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"] as const) listen(toolHandle, name, finishToolDrag);
  listen(window, "pointerup", finishToolDrag, { capture: true });
  listen(toolHandle, "keydown", (event) => {
    const edges: Record<string, string> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "top", ArrowDown: "bottom" };
    if (!event.altKey || !edges[event.code]) return;
    event.preventDefault();
    toolPosition = { edge: edges[event.code], ratio: 0.5 };
    saveToolPosition();
  });
  listen(window, "resize", () => layoutTools());
  listen(document, "fullscreenchange", () => layoutTools());
  const toolObserver = window.ResizeObserver ? new window.ResizeObserver(() => layoutTools()) : null;
  toolObserver?.observe(element("tool-panel"));
  toolObserver?.observe(toolHandle);
  listen(document, "keydown", (event) => {
    if (event.key === "Escape" && !options.onToolsOpen && !element("tool-panel").hidden && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      event.preventDefault();
      setToolsOpen(false, true);
    }
  });

  listen(canvas, "pointerdown", (event) => {
    if (mobile?.handlePointer(event, "down")) return;
    if (!ready || event.button < 0 || event.button > 2) return;
    event.preventDefault();
    setToolsOpen(false);
    canvas.focus({ preventScroll: true });
    mobile?.focusDesktop();
    void captureKeyboard();
    syncModifiers(event);
    canvas.setPointerCapture(event.pointerId);
    capturedPointers.add(event.pointerId);
    pendingMove = event;
    flushMove();
    if (!buttons.has(event.button) && send({ type: "button", button: event.button, pressed: true })) buttons.add(event.button);
  });
  listen(canvas, "pointermove", (event) => {
    if (mobile?.handlePointer(event, "move")) return;
    if (!ready) return;
    pendingMove = { clientX: event.clientX, clientY: event.clientY };
    if (moveFrame === null) moveFrame = requestAnimationFrame(flushMove);
  });
  listen(canvas, "pointerup", (event) => {
    if (mobile?.handlePointer(event, "up")) return;
    flushMove();
    if (buttons.delete(event.button)) send({ type: "button", button: event.button, pressed: false });
    if (!buttons.size) {
      capturedPointers.delete(event.pointerId);
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    }
  });
  listen(canvas, "pointercancel", (event) => { if (!mobile?.handlePointer(event, "cancel")) release(); });
  listen(canvas, "lostpointercapture", (event) => {
    if (event.pointerType === "touch") { mobile?.lostPointer(event.pointerId); return; }
    if (capturedPointers.has(event.pointerId)) release();
  });
  listen(canvas, "contextmenu", (event) => event.preventDefault());
  listen(canvas, "wheel", (event) => {
    if (!ready || event.ctrlKey || event.metaKey) return;
    event.preventDefault();
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1;
    const bounded = (value: number) => Number.isFinite(value) ? Math.max(-1000, Math.min(1000, value * scale)) : 0;
    const delta = pictureScrollDelta(event.deltaX, event.deltaY, pictureRotated);
    send({ type: "scroll", x: bounded(delta.x), y: bounded(delta.y) });
  }, { passive: false });
  function hardwareKey(event: KeyboardEvent, editable = false) {
    clipboardKeyEvent = null;
    pendingHardwareKey = null;
    if (!ready || desktopPaused || event.isComposing || composing || event.keyCode === 229) return false;
    if (event.code === "Escape" && event.ctrlKey && event.altKey && event.shiftKey) {
      event.preventDefault();
      release();
      keyboardCaptureWanted = false;
      setToolsOpen(true);
      toolHandle.focus();
      return false;
    }
    // A local Command+V must not become a bare Super tap in the remote
    // compositor. Defer Meta until a remote chord/click or a deliberate tap.
    if ((event.code === "MetaLeft" || event.code === "MetaRight") && !keys.has(KEY_CODES[event.code])) {
      event.preventDefault();
      deferredMeta.add(KEY_CODES[event.code]);
      syncModifiers(event, false);
      return true;
    }
    // Wait for insertText before forwarding printable keys: some IMEs start
    // composition after the first otherwise ordinary keydown.
    if (editable && event.key?.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey) {
      pendingHardwareKey = event;
      return false;
    }
    // Browser/Electron paste events carry local text and images even in fullscreen.
    if ((event.code === "KeyV" && (event.ctrlKey || event.metaKey))
      || (event.code === "Insert" && event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey)) {
      localPasteShift = event.code === "KeyV" && event.shiftKey;
      release(); localMeta=event.metaKey;
      return false;
    }
    if(appleClipboard&&event.metaKey&&!event.ctrlKey&&!event.altKey&&!event.shiftKey
      &&(event.code==="KeyC"||event.code==="KeyX")) {
      if(event.repeat){event.preventDefault();return true;}
      if(clipboard.copyKey(KEY_CODES[event.code],undefined,true)) {
        event.preventDefault();release();localMeta=true;clipboardKeyEvent=KEY_CODES[event.code];return true;
      }
    }
    syncModifiers(event);
    const code = Object.hasOwn(KEY_CODES, event.code) ? KEY_CODES[event.code] : undefined;
    if (code === undefined) return false;
    if(!event.repeat&&!keys.has(code)&&(event.ctrlKey||event.metaKey)&&["KeyC","KeyX"].includes(event.code)&&clipboard.copyKey(code)) {
      event.preventDefault();keys.add(code);clipboardKeyEvent=code;return true;
    }
    // Editable focus is required to start a local IME. Cancel its eventual
    // beforeinput after forwarding a physical key, rather than cancelling keydown.
    if (!editable || event.ctrlKey || event.altKey || event.metaKey || event.key?.length !== 1) event.preventDefault();
    if (!event.repeat && !keys.has(code) && send({ type: "key", code, pressed: true })) keys.add(code);
    return true;
  }
  listen(canvas, "keydown", (event) => { hardwareKey(event); });
  listen(window, "keyup", (event) => {
    const code = KEY_CODES[event.code];
    if (!event.metaKey && (event.code==="MetaLeft"||event.code==="MetaRight")) localMeta=false;
    if (deferredMeta.delete(code) && !event.metaKey && ready && !desktopPaused && keyboardFocused()) {
      event.preventDefault();
      send({type:"key",code,pressed:true});
      send({type:"key",code,pressed:false});
    }
    if (keys.delete(code)) {
      event.preventDefault();
      send({ type: "key", code, pressed: false });
    }
    if (ready && keyboardFocused() && !(event.code === "KeyV" && (event.ctrlKey || event.metaKey))) syncModifiers(event, false);
  });
  listen(canvas, "compositionstart", () => { composing = true; release(); });
  listen(canvas, "compositionend", (event) => { composing = false; typeText(event.data); });
  listen(textBox, "compositionstart", () => { composing = true; });
  listen(textBox, "compositionend", () => { composing = false; });
  listen(textBox, "blur", () => { composing = false; });
  listen(document,"pointerdown",()=>{clipboardKeyEvent=null;},{capture:true});
  // Native browser/Electron menu accelerators can invoke Copy/Cut without
  // delivering the matching keydown. Never copy the hidden IME sentinel.
  for (const target of [canvas,keyboardInput]) {
    for (const type of ["copy","cut"] as const) listen(target,type,(event)=>{
      event.preventDefault();
      if(!ready||desktopPaused)return;
      const code=type==="copy"?KEY_CODES.KeyC:KEY_CODES.KeyX;
      const paired=clipboardKeyEvent===code&&clipboard.copying();
      clipboardKeyEvent=null;
      const accepted=paired||clipboard.copyKey(code,undefined,true);
      release();localMeta=appleClipboard;
      if(!accepted)message("Remote clipboard synchronization is unavailable for this desktop.");
    });
  }
  listen(canvas, "paste", (event) => { void clipboard.paste(event); });
  listen(canvas, "focus", () => { mobile?.focusDesktop(); void captureKeyboard(); });
  listen(keyboardInput, "focus", () => { void captureKeyboard(); });
  listen(keyboardInput, "blur", () => { release(); unlockKeyboard(); });
  for (const type of ["copy","cut"] as const) listen(document,type,(event)=>{
    if(event.target!==canvas&&event.target!==keyboardInput&&!event.defaultPrevented)clipboard.localChanged();
  });
  listen(window, "focus", () => { clipboard.localFocusEntered(); void captureKeyboard(); });
  listen(canvas, "blur", (event) => { if (event.relatedTarget !== keyboardInput) { release(); unlockKeyboard(); } });
  listen(window, "blur", () => { composing = false; clipboard.localFocusLeft(); release(); unlockKeyboard(); });
  listen(document, "visibilitychange", () => {
    if (document.hidden) { composing = false; clipboard.localFocusLeft(); release(); stopAudio(); unlockKeyboard(); }
    else { clipboard.localFocusEntered(); scheduleAutoResize(); scheduleVideoScale(); }
  });
  listen(element("connection"), "click", () => {
    if (wanted) disconnect();
    else {
      wanted = true; retries = 0; connectionFailure = ""; updateControls();
      if (socket) closeSocket(socket, 1000, "Reconnect");
      else connect();
    }
  });
  listen(element("audio"), "click", () => { void toggleAudio(); });
  listen(element("desktop-source"), "change", () => {
    switchDesktop(element("desktop-source").value);
  });
  listen(element("monitor-source"), "change", () => {
    const output = element("monitor-source").value;
    if (desktopMode === "physical" && desktopOutputs.some((monitor) => monitor.name === output)) switchDesktop("physical", output);
  });
  async function refreshDesktops() {
    if (destroyed || document.hidden || refreshingDesktops || switchingDesktop) return;
    const request = ++discoveryRequest;
    const controller = new AbortController();
    discoveryController = controller;
    refreshingDesktops = true;
    try {
      const response = await fetch(new URL("./desktops", location.href), {
        credentials: "same-origin", cache: "no-store", signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const value = await response.json();
      if (request !== discoveryRequest || destroyed) return;
      if (!value || !Array.isArray(value.desktops)) throw new Error("无效的桌面列表");
      populateDesktops(value);
    } catch (error) {
      if (request === discoveryRequest && !destroyed && !controller.signal.aborted) desktopStatus(`刷新失败：${errorMessage(error)}。仍可切回虚拟桌面。`);
    } finally {
      if (request === discoveryRequest) { discoveryController = null; refreshingDesktops = false; updateControls(); }
    }
  }
  async function displayMode(mode: string) {
    pictureZoom.reset();
    try {
      if (mode === "fullscreen") {
        // Include portalled menus and dialogs in the browser fullscreen tree.
        mobile?.hideKeyboard();
        try {
          await (document.documentElement ?? viewport).requestFullscreen();
        } catch (error) {
          if (!mobile?.isTouch()) throw error;
          fullscreenFallback = true;
        }
        viewport.classList.toggle("fit", true);
        synchronizeFullscreen();
      } else {
        windowMode = mode === "actual" ? "actual" : "fit";
        fullscreenFallback = false;
        mobile?.setFullscreen(false);
        if (document.fullscreenElement) await document.exitFullscreen();
        synchronizeFullscreen();
        if (windowMode === "actual") {
          lastAutoResolution = null;
          element("auto-resolution").checked = false;
          cancelTimer(autoResizeTimer);
          autoResizeTimer = null;
        }
        viewport.classList.toggle("fit", windowMode === "fit");
      }
      canvas.focus({ preventScroll: true });
      if (mode === "fullscreen") await captureKeyboard();
      updateDisplayMode();
    } catch (error) {
      updateDisplayMode();
      message(`Fullscreen unavailable: ${errorMessage(error)}`);
    }
  }
  function updateDisplayMode() {
    configureVideoScale();
    publishSessionView();
    updateCanvasDensity();
    const mode = isDesktopFullscreen() ? "fullscreen" : windowMode;
    element("mode-fit").setAttribute("aria-pressed", String(mode === "fit"));
    element("mode-actual").setAttribute("aria-pressed", String(mode === "actual"));
    element("fullscreen").setAttribute("aria-pressed", String(mode === "fullscreen"));
    setText("display-mode", mode === "actual" ? "原始大小 · 按设定分辨率 1:1 显示" : mode === "fullscreen" ? "全屏 · Ctrl+Alt+Shift+Esc 释放键盘" : "适应窗口");
    for (const attribute of ["aria-label", "title", "data-tip"]) element("fullscreen").setAttribute(attribute, translate(mode === "fullscreen" ? "退出全屏" : "全屏"));
  }
  listen(element("mode-fit"), "click", () => { void displayMode("fit"); });
  listen(element("mode-actual"), "click", () => { void displayMode("actual"); });
  function scheduleAutoResize() {
    cancelTimer(autoResizeTimer);
    autoResizeTimer = null;
    if (!ready || !resizeAvailable || mobile?.keyboardActive() || !element("auto-resolution").checked) return;
    autoResizeTimer = setTimeout(() => {
      autoResizeTimer = null;
      if (!ready || !resizeAvailable || mobile?.keyboardActive() || !element("auto-resolution").checked || document.hidden || viewport.clientWidth <= 0 || viewport.clientHeight <= 0) return;
      const bounds = mobile?.bounds();
      if (bounds && pictureRotated) {
        const width = bounds.width; bounds.width = bounds.height-bounds.bar; bounds.height = width; bounds.bar = 0;
      }
      const scaled = resizeScaleAvailable;
      const target = scaled ? (bounds
        ? mobileResolution(bounds.width, bounds.height - bounds.bar, window.devicePixelRatio || 1)
        : captureResolution(viewport.clientWidth, viewport.clientHeight, window.devicePixelRatio || 1, resizeScales)) : {
        width: Math.max(320, Math.min(3840, Math.floor((bounds?.width ?? viewport.clientWidth) / 2) * 2)),
        height: Math.max(200, Math.min(2160, Math.floor((bounds ? bounds.height - bounds.bar : viewport.clientHeight) / 2) * 2)), scale: 1,
      };
      const { width, height, scale } = target;
      const size = `${width}x${height}`;
      const resolution = `${size}@${scale}`;
      if (scaled ? lastAutoResolution === resolution : lastRequestedSize === size || (sourceWidth === width && sourceHeight === height)) return;
      release();
      if (send({ type: "resize", width, height, ...(scaled ? { scale } : {}) })) {
        lastAutoResolution = resolution;
        lastRequestedSize = size;
        updateCanvasDensity();
        element("width").value = String(width);
        element("height").value = String(height);
        message(`跟随窗口：${width} × ${height}`);
      }
    }, 250);
  }
  listen(window, "resize", () => { updateCanvasDensity(); scheduleAutoResize(); scheduleVideoScale(); publishSessionView(); });
  function watchPixelRatio() {
    const query = window.matchMedia?.(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    if (query) listen(query, "change", () => { updateCanvasDensity(); scheduleAutoResize(); scheduleVideoScale(); watchPixelRatio(); }, { once: true });
  }
  watchPixelRatio();
  listen(element("auto-resolution"), "change", () => { lastAutoResolution = null; updateCanvasDensity(); scheduleAutoResize(); publishSessionView(); });
  listen(element("fullscreen"), "click", async () => {
    await displayMode(isDesktopFullscreen() ? windowMode : "fullscreen");
  });
  function synchronizeFullscreen() {
    const fullscreen = isDesktopFullscreen();
    const entering = fullscreen && !desktopFullscreen;
    desktopFullscreen = fullscreen;
    if (entering && mobile?.isTouch()) release();
    mobile?.setFullscreen(fullscreen);
    pictureZoom.setCover(!!mobile?.isFullscreen() && desktopMode === "physical");
    if (fullscreen) {
      if (fullscreenAutoResolution === null) {
        fullscreenAutoResolution = element("auto-resolution").checked;
        if (resizeAvailable) element("auto-resolution").checked = true;
      }
    } else if (fullscreenAutoResolution !== null) {
      element("auto-resolution").checked = fullscreenAutoResolution;
      fullscreenAutoResolution = null;
    }
    if (!fullscreen) { screenRotation.reset(); release(); unlockKeyboard(); }
    updateDisplayMode();
    viewport.classList.toggle("fit", fullscreen ? true : windowMode === "fit");
    scheduleAutoResize();
    if (entering) {
      keyboardCaptureWanted = true;
      setToolsOpen(false, true);
      canvas.focus({ preventScroll: true });
      void captureKeyboard();
    }
  }
  listen(document, "fullscreenchange", () => {
    // Some mobile browsers leave native fullscreen when the IME is focused.
    // Keep the same viewport layout while that requested keyboard is open.
    if (!nativeDesktopFullscreen() && desktopFullscreen && mobile?.isFullscreen() && mobile.keyboardActive()) fullscreenFallback = true;
    synchronizeFullscreen();
  });
  listen(element("keyboard-capture"), "click", async () => {
    if (keyboardCaptured) {
      keyboardCaptureWanted = false;
      release();
      unlockKeyboard();
      return;
    }
    keyboardCaptureWanted = true;
    setToolsOpen(false);
    if (!isDesktopFullscreen()) await displayMode("fullscreen");
    else { canvas.focus({ preventScroll: true }); await captureKeyboard(); }
  });
  listen(element("switch-input-method"), "click", () => {
    if (!ready || desktopPaused) return;
    release();
    sendChord([KEY_CODES.ControlLeft, KEY_CODES.Space]);
    setToolsOpen(false, true);
  });
  element("shortcut-key").replaceChildren();
  for (const code of Object.keys(KEY_CODES)) {
    const option = document.createElement("option");
    option.value = code;
    option.textContent = code;
    element("shortcut-key").appendChild(option);
  }
  element("shortcut-key").value = "Enter";
  listen(element("send-shortcut"), "click", () => {
    if (!ready) return;
    const mainKey = element("shortcut-key").value;
    if (!Object.hasOwn(KEY_CODES, mainKey)) return;
    const chord: number[] = [];
    for (const [id, code] of [["shortcut-ctrl", "ControlLeft"], ["shortcut-alt", "AltLeft"], ["shortcut-shift", "ShiftLeft"], ["shortcut-super", "MetaLeft"]] as const) {
      if (element(id).checked) chord.push(KEY_CODES[code]);
    }
    if (!chord.includes(KEY_CODES[mainKey])) chord.push(KEY_CODES[mainKey]);
    sendChord(chord);
    message("组合键已发送到远程桌面。");
    setToolsOpen(false, true);
  });
  function sendChord(chord: number[]) {
    release();
    if(chord.length===2&&chord.includes(KEY_CODES.Insert)&&chord.some(code=>code===KEY_CODES.ShiftLeft||code===KEY_CODES.ShiftRight)) {
      localPasteShift=false;void clipboard.read();return;
    }
    if(chord.includes(KEY_CODES.KeyV)&&chord.some(code=>code===KEY_CODES.ControlLeft||code===KEY_CODES.ControlRight)
      &&chord.every(code=>[KEY_CODES.ControlLeft,KEY_CODES.ControlRight,KEY_CODES.ShiftLeft,KEY_CODES.ShiftRight,KEY_CODES.KeyV].includes(code))) {
      localPasteShift=chord.some(code=>code===KEY_CODES.ShiftLeft||code===KEY_CODES.ShiftRight);
      void clipboard.read();return;
    }
    const copyCode=chord.find(code=>code===KEY_CODES.KeyC||code===KEY_CODES.KeyX);
    if(copyCode!==undefined&&chord.some(code=>[KEY_CODES.ControlLeft,KEY_CODES.ControlRight,KEY_CODES.MetaLeft,KEY_CODES.MetaRight].includes(code))&&clipboard.copyKey(copyCode,chord))return;
    if (keyChords) { send({ type: "chord", codes: chord }); return; }
    for (const code of chord) {
      if (!send({ type: "key", code, pressed: true })) return;
      keys.add(code);
    }
    for (const code of [...chord].reverse()) {
      if (!send({ type: "key", code, pressed: false })) return;
      keys.delete(code);
    }
  }
  listen(element("resize"), "submit", (event) => {
    event.preventDefault();
    lastAutoResolution = null;
    if (!ready || !resizeAvailable) return;
    const width = Number(element("width").value);
    const height = Number(element("height").value);
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 320 || width > 3840 || height < 200 || height > 2160) {
      message("Resolution must be 320–3840 × 200–2160 whole pixels.");
      return;
    }
    release();
    element("auto-resolution").checked = false;
    updateCanvasDensity();
    cancelTimer(autoResizeTimer);
    autoResizeTimer = null;
    lastRequestedSize = null;
    if (send({ type: "resize", width, height })) message("Resize requested; waiting for the server.");
    publishSessionView();
  });
  listen(element("send-text"), "click", () => { if (!composing) typeText(textBox.value); });
  listen(element("clipboard-get"), "click", () => {
    clipboardPending = send({ type: "clipboard_get" });
  });
  listen(element("clipboard-set"), "click", () => {
    if (validText(textBox.value) && send({ type: "clipboard_set", text: textBox.value })) message("Remote clipboard update requested.");
  });
  listen(element("paste-local"), "click", () => { localPasteShift = false; setToolsOpen(false, true); void clipboard.read(true); });

  function destroy() {
    if (destroyed) return;
    screenRotation.dispose();
    disconnect();
    destroyed = true;
    discoveryController?.abort();
    lightBook?.dispose();
    mobile?.dispose();
    pictureZoom.dispose();
    cursor.dispose();
    toolObserver?.disconnect();
    listeners.abort();
    if (socket) {
      socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
      socket = null;
    }
  }

  listen(window, "online", () => {
    if (!wanted || destroyed || socket) return;
    cancelTimer(retryTimer); retryTimer = null;
    connect();
  });
  listen(document, "visibilitychange", () => {
    if (!document.hidden) lastServerActivity = performance.now();
  });

  listen(window, "pagehide", (event) => {
    resumeWanted = wanted;
    if (event.persisted) disconnect();
    else destroy();
  });
  listen(window, "pageshow", (event) => {
    if (event.persisted && resumeWanted) { wanted = true; updateControls(); connect(); }
  });
  function setLightBook(enabled: boolean) {
    const next = enabled && supportsLightBook();
    if (next === lightBookEnabled) return;
    release(); mobile?.reset(); pictureZoom.reset();
    lightBookEnabled = next;
    viewport.classList.toggle("lightbook-mode", next);
    mobile?.refreshLayout();
  }
  lightBook = createLightBookInput({
    root: element("mobile-shortcuts"), active: () => lightBookEnabled, ready: () => ready && !desktopPaused,
    key(code, pressed) {
      if (!Object.hasOwn(KEY_CODES, code) || !ready || desktopPaused) return false;
      const value = KEY_CODES[code];
      const modifier=[KEY_CODES.ControlLeft,KEY_CODES.ControlRight,KEY_CODES.MetaLeft,KEY_CODES.MetaRight].some(key=>keys.has(key));
      if(pressed&&(keys.has(KEY_CODES.ControlLeft)||keys.has(KEY_CODES.ControlRight))&&code==="KeyV"){
        localPasteShift=keys.has(KEY_CODES.ShiftLeft)||keys.has(KEY_CODES.ShiftRight);
        release();void clipboard.read();return true;
      }
      if(pressed&&modifier&&(code==="KeyC"||code==="KeyX")&&clipboard.copyKey(value)){keys.add(value);return true;}
      if (pressed) { if (keys.has(value)) return true; if (send({type:"key",code:value,pressed:true})) { keys.add(value); return true; } }
      else if (keys.delete(value)) return send({type:"key",code:value,pressed:false});
      return false;
    },
    button(button, pressed) {
      if (!ready || desktopPaused) return false;
      if (pressed) { if (buttons.has(button)) return true; if (send({type:"button",button,pressed:true})) { buttons.add(button); return true; } }
      else if (buttons.delete(button)) return send({type:"button",button,pressed:false});
      return false;
    },
    engage: () => setToolsOpen(false), release,
    position: () => cursor.position(), dimensions: () => canvas.getBoundingClientRect(),
    move(point) { pictureZoom.reveal(point); cursor.moveNormalized(point.x,point.y); send({type:"move",...point}); },
    scroll: (x,y) => send({type:"scroll",x:Math.max(-1000,Math.min(1000,x)),y:Math.max(-1000,Math.min(1000,y))}),
    zoomActive: pictureZoom.active,
    pan(x,y) {
      const bounds=pictureBounds();
      const anchor={x:(bounds.left+bounds.right)/2,y:(bounds.top+bounds.bottom)/2};
      pictureZoom.gesture(anchor,{x:anchor.x+x,y:anchor.y+y},1);
    },
    zoom(ratio) {
      const point = cursor.position(), rect = canvas.getBoundingClientRect();
      const anchor = {x:rect.left+point.x*rect.width,y:rect.top+point.y*rect.height};
      pictureZoom.gesture(anchor,anchor,ratio);
    },
  });
  mobile = createMobileControls({
    canvas, viewport, input: element("mobile-input"), bar: element("mobile-shortcuts"), toggle: element("mobile-keyboard"),
    fullscreenExit: element("mobile-fullscreen-exit"), exitFullscreen: () => { void displayMode(windowMode); },
    fullscreenRotate: element("mobile-fullscreen-rotate"), rotateScreen: screenRotation.toggle,
    toggleFullscreen: () => { void displayMode(isDesktopFullscreen() ? windowMode : "fullscreen"); },
    toggleAudio: () => { void toggleAudio(); }, audioAvailable: () => audioAvailable, rotated: () => pictureRotated,
    ready: () => ready && !desktopPaused, lightBook: () => lightBookEnabled, zoom: pictureZoom, send, text: typeText, move, tools: (open) => setToolsOpen(open), translate,
    isToolsOpen: () => options.isToolsOpen?.() ?? !element("tool-panel").hidden,
    hardwareKey: (event) => hardwareKey(event, true), composition: release,
    hardwareInput: (event) => {
      const pending = pendingHardwareKey;
      pendingHardwareKey = null;
      if (!pending || event.inputType !== "insertText" || !/^[ -~]$/.test(event.data ?? "")) return false;
      return hardwareKey(pending);
    },
    paste: (event) => { void clipboard.paste(event); },
    chord: (codes) => {
      if(!ready||!codes.every(code=>Object.hasOwn(KEY_CODES,code)))return;
      if(codes.includes("KeyV")&&codes.some(code=>code==="ControlLeft"||code==="ControlRight")) {
        localPasteShift=codes.some(code=>code==="ShiftLeft"||code==="ShiftRight");release();void clipboard.read();return;
      }
      sendChord(codes.map(code=>KEY_CODES[code]));
    },
    layout: () => { updateCanvasDensity(); pictureZoom.setCover(!!mobile?.isFullscreen() && desktopMode === "physical"); pictureZoom.layout(); layoutTools(); scheduleAutoResize(); scheduleVideoScale(); },
  });
  setLightBook(options.lightBook === true);
  wanted = true;
  setToolsOpen(false);
  updateDisplayMode();
  updateControls();
  if (drawing) connect();
  else { disconnect(); message("Canvas rendering is unavailable in this browser."); }
  return { destroy, setLightBook, refreshDesktops, refreshLanguage() {
    for (const [id, text] of texts) {
      element(id).textContent = translate(text);
      if (id === "state" || id === "performance" || id === "video-details" || id === "message" || id === "desktop-status") options.onStatus?.(id, translate(text));
    }
    if (desktopChoices.length) populateDesktops({ desktops: desktopChoices, outputs: desktopOutputs, reason: discoveryReason });
    updateControls(); updateDisplayMode(); layoutTools();
  }, layoutTools };
}
