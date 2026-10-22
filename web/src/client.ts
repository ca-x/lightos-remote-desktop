import type { ClientOptions, DesktopChoice, DesktopOutput, Elements } from "./types";
import { createMobileControls, mobileResolution } from "./mobile";
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
  };
  const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);
  type Timer = ReturnType<typeof setTimeout>;
  const cancelTimer = (timer: Timer | null) => { if (timer !== null) clearTimeout(timer); };
  const canvas = element("desktop");
  const drawing = canvas.getContext("2d", { alpha: false });
  const viewport = element("viewport");
  const textBox = element("text");
  const controls = ["apply-size", "send-text", "clipboard-get", "clipboard-set", "paste-local", "keyboard-capture", "send-shortcut"] as const;
  const listeners = new AbortController();
  const keys = new Set<number>();
  const buttons = new Set<number>();
  const sources = new Set<AudioBufferSourceNode>();
  let socket: WebSocket | null = null;
  let wanted = false;
  let ready = false;
  let destroyed = false;
  let generation = 0;
  let retryTimer: Timer | null = null;
  let connectionTimer: Timer | null = null;
  let closeTimer: Timer | null = null;
  let retries = 0;
  let connectionFailure = "";
  let pendingFrame: { data: ArrayBuffer; generation: number; videoToken: number; id: number | null } | null = null;
  let decoding = false;
  let audioAvailable = false;
  let audioContext: AudioContext | null = null;
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
  let frameAcknowledgements = false;
  let windowMode = "fit";
  let fullscreenAutoResolution: boolean | null = null;
  let desktopFullscreen = false;
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
  let lastAutoResolution: string | null = null;
  let lastRequestedSize: string | null = null;
  let keyboardCaptured = false;
  let keyboardCaptureWanted = true;
  let keyboardRequest = false;
  let keyboardToken = 0;
  let desktopMode = "virtual";
  let selectedOutput: string | null = null;
  let desktopChoices: DesktopChoice[] = [];
  let desktopOutputs: DesktopOutput[] = [];
  let resizeAvailable = true;
  let switchingDesktop = false;
  let controlRelease = false;
  let releasingSocket: WebSocket | null = null;
  let discoveryRequest = 0;
  let refreshingDesktops = false;
  let mobile: ReturnType<typeof createMobileControls> | null = null;
  let discoveryReason = "";

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
    element("keyboard-capture").setAttribute("aria-pressed", String(keyboardCaptured));
    setText("keyboard-capture", keyboardCaptured ? "释放键盘捕获" : "全屏捕获键盘");
  }

  function isDesktopFullscreen() {
    const fullscreen = document.fullscreenElement;
    return fullscreen === viewport || fullscreen === canvas || fullscreen?.contains(canvas) === true;
  }

  function unlockKeyboard() {
    keyboardToken += 1;
    try { navigator.keyboard?.unlock(); } catch {}
    keyboardCaptured = false;
    setText("keyboard-state", "未捕获键盘。全屏可尝试捕获；系统拦截的按键可通过通用组合键发送器发送。");
    updateKeyboardCapture();
  }

  async function captureKeyboard() {
    if (!ready || destroyed || keyboardRequest || keyboardCaptured || !keyboardCaptureWanted || document.hidden || !document.hasFocus() || !isDesktopFullscreen() || document.activeElement !== canvas) return;
    if (!navigator.keyboard?.lock) {
      setText("keyboard-state", "浏览器不支持键盘捕获。系统截走的组合键可用下方通用发送器发送。");
      return;
    }
    const token = keyboardToken;
    keyboardRequest = true;
    try {
      await navigator.keyboard.lock();
      if (token !== keyboardToken || !ready || destroyed || document.hidden || !document.hasFocus() || !isDesktopFullscreen() || document.activeElement !== canvas) {
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

  function layoutTools(point: {x: number; y: number} | null = toolDrag?.moved ? toolDrag : null) {
    const bounds = mobile?.bounds();
    const width = bounds?.width || window.innerWidth || 1280;
    const height = bounds ? bounds.height - bounds.bar : window.innerHeight || 720;
    const originX = bounds?.left ?? 0;
    const originY = bounds?.top ?? 0;
    const maxX = Math.max(12, width - 52);
    const maxY = Math.max(12, height - 60);
    const horizontal = toolPosition.edge === "top" || toolPosition.edge === "bottom";
    const left = point ? Math.max(12, Math.min(maxX, point.x - originX)) :
      horizontal ? 12 + toolPosition.ratio * (maxX - 12) : toolPosition.edge === "left" ? 12 : maxX;
    const top = point ? Math.max(12, Math.min(maxY, point.y - originY)) :
      horizontal ? toolPosition.edge === "top" ? 12 : maxY : 12 + toolPosition.ratio * (maxY - 12);
    tools.style.setProperty("--tool-x", `${left + originX}px`);
    tools.style.setProperty("--tool-y", `${top + originY}px`);
    tools.style.setProperty("--panel-origin", toolPosition.edge === "left" ? "left center" : toolPosition.edge === "right" ? "right center" : toolPosition.edge === "top" ? "center top" : "center bottom");
    if (options.isToolsOpen ? !options.isToolsOpen() : element("tool-panel").hidden) return;
    const panel = element("tool-panel").getBoundingClientRect();
    const panelX = toolPosition.edge === "left" ? left + 48 : toolPosition.edge === "right" ? left - panel.width - 8 : left + 20 - panel.width / 2;
    const panelY = toolPosition.edge === "top" ? top + 56 : toolPosition.edge === "bottom" ? top - panel.height - 8 : top + 24 - panel.height / 2;
    tools.style.setProperty("--panel-x", `${originX + Math.max(12, Math.min(width - panel.width - 12, panelX))}px`);
    tools.style.setProperty("--panel-y", `${originY + Math.max(12, Math.min(height - panel.height - 12, panelY))}px`);
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
  const rttSamples: number[] = [];
  const renderSamples: number[] = [];
  let pipelineMetrics: Record<string, { count: number; p50_ms: number | null; p95_ms: number | null }> = {};

  function recordFrame(codec: string, started?: number) {
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
    if (open) unlockKeyboard();
    if (options.onToolsOpen) {
      options.onToolsOpen(open, focus);
      requestAnimationFrame(() => { if (!destroyed) layoutTools(); });
      return;
    }
    element("tool-panel").hidden = !open;
    element("tools-toggle").setAttribute("aria-expanded", String(open));
    element("tools-toggle").setAttribute("aria-label", open ? "收起桌面工具栏" : "打开桌面工具栏");
    layoutTools();
    if (focus) (open ? element("connection") : canvas).focus({ preventScroll: true });
  }

  function validDimensions(width: number, height: number) {
    return Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 &&
      width <= 8192 && height <= 8192 && width * height <= 33554432;
  }

  function resizeCanvas(width: number, height: number) {
    if (lastRequestedSize === `${width}x${height}`) lastRequestedSize = null;
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    canvas.style.setProperty("--desktop-aspect", String(width / height));
  }

  function updateControls() {
    mobile?.update();
    options.onControls?.({ ready, wanted, audioAvailable, audioEnabled, resizeAvailable, desktopMode,
      switchingDesktop, refreshingDesktops, choices: desktopChoices, outputs: desktopOutputs });
    for (const id of controls) element(id).disabled = !ready;
    for (const id of ["width", "height", "apply-size", "auto-resolution"] as const) element(id).disabled = !ready || !resizeAvailable;
    element("resize").setAttribute("aria-disabled", String(!ready || !resizeAvailable));
    element("resize").setAttribute("title", translate(resizeAvailable ? "调整虚拟桌面分辨率" : "物理桌面不支持更改显示器分辨率"));
    element("desktop-controls").hidden = desktopChoices.length === 0;
    element("desktop-source").disabled = desktopChoices.length === 0;
    element("desktop-source").value = desktopMode;
    element("monitor-controls").hidden = desktopMode !== "physical" || desktopChoices.length === 0;
    element("monitor-source").disabled = !ready || switchingDesktop || desktopOutputs.length === 0;
    element("refresh-desktops").disabled = refreshingDesktops || switchingDesktop;
    element("audio").disabled = !ready || !audioAvailable;
    const audioLabel = audioContext ? "关闭声音" : "开启声音";
    for (const attribute of ["aria-label", "title", "data-tip"]) element("audio").setAttribute(attribute, translate(audioLabel));
    element("audio").setAttribute("aria-pressed", String(audioEnabled));
    for (const attribute of ["aria-label", "title", "data-tip"]) element("connection").setAttribute(attribute, translate(wanted ? "断开连接" : "重新连接"));
  }

  function send(command: Record<string, unknown>) {
    if (!ready || !socket || socket.readyState !== WebSocket.OPEN) return false;
    if (socket.bufferedAmount > 262144) {
      message("Connection is too slow; reconnecting to release input safely.");
      socket.close(4001, "Input backpressure");
      resetSession();
      return false;
    }
    try {
      socket.send(JSON.stringify(command));
      return true;
    } catch {
      socket.close();
      resetSession();
      return false;
    }
  }

  function release() {
    if (moveFrame !== null) cancelAnimationFrame(moveFrame);
    moveFrame = null;
    pendingMove = null;
    send({ type: "release" });
    keys.clear();
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
    if (previous) previous.close().catch(() => {});
    updateControls();
  }

  function resetSession() {
    resizeScaleAvailable = false;
    lastAutoResolution = null;
    mobile?.reset();
    discoveryRequest += 1;
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
    sequencedFrames = false;
    pendingFrameId = null;
    closeVideo();
    keys.clear();
    buttons.clear();
    release();
    stopAudio(false);
    updateControls();
  }

  function waitForClose(current: WebSocket) {
    // A Close frame can be lost by a proxy. Stop waiting, but require an
    // explicit user action before opening another transport. The server still
    // owns the single-controller gate if the abandoned transport is alive.
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
        switchingDesktop = false;
        wanted = false;
        connectionFailure = "Previous connection did not close within 3 seconds. Reconnect or select another desktop.";
        resetSession();
        setText("state", "Disconnected");
        desktopStatus(connectionFailure);
        message(connectionFailure);
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

  function connect() {
    if (!wanted || destroyed || socket || retryTimer !== null) return;
    setText("state", "Connecting");
    const url = new URL("./ws", location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("desktop", desktopMode);
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
    connectionTimer = setTimeout(() => {
      connectionTimer = null;
      if (socket !== current || ready) return;
      connectionFailure = "Desktop readiness timed out; reconnecting.";
      message(connectionFailure);
      desktopStatus(`${desktopLabel()}连接超时，可切回虚拟桌面。`);
      closeSocket(current, 4000, "Readiness timeout");
      resetSession();
    }, 15000);
    current.binaryType = "arraybuffer";
    current.onopen = () => {
      if (current !== socket || !wanted || destroyed) { current.close(); return; }
      setText("state", "Waiting for desktop");
      message("Connected; waiting for server readiness.");
    };
    current.onmessage = (event) => {
      if (current !== socket || !wanted || destroyed) return;
      if (switchingDesktop) {
        if (typeof event.data === "string") {
          try { if (JSON.parse(event.data)?.type === "released") completeRelease(current); } catch {}
        }
        return;
      }
      if (typeof event.data === "string") {
        receiveJSON(event.data);
      } else if (ready && event.data instanceof ArrayBuffer && event.data.byteLength > 1) {
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
    if (retries >= 2) {
      wanted = false;
      connectionFailure ||= "Desktop connection failed.";
      desktopStatus(connectionFailure);
      message(`Automatic reconnect stopped after two retries. ${connectionFailure}`);
      updateControls();
      return;
    }
    const delay = 500 * 2 ** retries++;
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
    if (value.type === "ready") {
      const { width, height } = value;
      if (!validDimensions(width, height)) {
        message("Server supplied an invalid desktop size.");
        return;
      }
      resizeCanvas(width, height);
      canvas.style.setProperty("cursor", value.cursor === "embedded" ? "none" : "default");
      if (document.activeElement !== element("width") && document.activeElement !== element("height")) {
        element("width").value = String(width);
        element("height").value = String(height);
      }
      ready = true;
      resizeScaleAvailable = value.resize_scale === true;
      lastAutoResolution = null;
      controlRelease = value.control_release === true;
      connectionFailure = "";
      if (typeof value.version === "string" && value.version.length <= 64 && /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?$/.test(value.version)) {
        setText("software-version", `v${value.version}`);
      }
      desktopMode = value.desktop === "physical" ? "physical" : "virtual";
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
      if (frameAcknowledgements) send({ type: "video_flow", enabled: true, sequenced: sequencedFrames });
      if (frameAcknowledgements && sequencedFrames && value.video_codecs?.includes("h264") && window.VideoDecoder && window.EncodedVideoChunk) send({ type: "video", codec: "h264" });
      cancelTimer(connectionTimer);
      connectionTimer = null;
      retries = 0;
      audioAvailable = value.audio === true;
      audioSession = Number.isSafeInteger(value.audio_session) && value.audio_session > 0 ? value.audio_session : null;
      opusAvailable = Array.isArray(value.audio_codecs) && value.audio_codecs.includes("opus");
      if (!audioAvailable) stopAudio(false);
      setText("state", "Connected");
      message(audioAvailable ? `Desktop ready. Audio is ${audioEnabled ? "on" : "off until enabled"}.` : "Desktop ready. Server audio is unavailable.");
      updateControls();
      scheduleAutoResize();
      void captureKeyboard();
    } else if (value.type === "pong" && pendingPing && value.id === pendingPing.id) {
      latestRtt = Math.max(0, performance.now() - pendingPing.started);
      rttSamples.push(latestRtt);
      if (value.video_metrics && typeof value.video_metrics === "object") pipelineMetrics = value.video_metrics;
      if (rttSamples.length > 30) rttSamples.shift();
      pendingPing = null;
    } else if (value.type === "frame_id" && Number.isSafeInteger(value.id) && value.id > 0) {
      pendingFrameId = value.id;
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
      if (!ready) connectionFailure = `Server: ${value.message.slice(0, 2048)}`;
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
          if (frame.generation === generation && frame.videoToken === videoToken && ready && !destroyed) {
            if (!validDimensions(bitmap.width, bitmap.height)) {
              message("Server supplied invalid JPEG dimensions.");
              continue;
            }
            resizeCanvas(bitmap.width, bitmap.height);
            drawing?.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
            canvas.hidden = false;
            recordFrame("JPEG", started);
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

  function fallbackVideo() {
    closeVideo();
    send({ type: "video", codec: "jpeg" });
    message("H.264 is unavailable; using compatible JPEG video.");
  }

  async function configureVideo(value: {codec: string; width: number; height: number}) {
    closeVideo();
    pendingFrame = null;
    if (value.codec === "jpeg") { message("Video: JPEG compatibility mode."); return; }
    if (!/^avc1\.[0-9a-f]{6}$/i.test(value.codec) || !window.VideoDecoder || !validDimensions(value.width, value.height)) { fallbackVideo(); return; }
    const token = videoToken;
    const session = generation;
    configuringVideo = true;
    try {
      const config: VideoDecoderConfig = { codec: value.codec, codedWidth: value.width, codedHeight: value.height, optimizeForLatency: true, hardwareAcceleration: "prefer-hardware" };
      let support = await window.VideoDecoder.isConfigSupported(config);
      if (!support.supported) {
        config.hardwareAcceleration = "no-preference";
        support = await window.VideoDecoder.isConfigSupported(config);
      }
      if (token !== videoToken || session !== generation || !ready || destroyed) return;
      if (!support.supported) { fallbackVideo(); return; }
      videoDecoder = new window.VideoDecoder({
        output(frame) {
          try {
            if (token !== videoToken || session !== generation || !ready || destroyed) return;
            cancelTimer(videoTimer);
            videoTimer = null;
            if (!validDimensions(frame.displayWidth, frame.displayHeight)) { fallbackVideo(); return; }
            resizeCanvas(frame.displayWidth, frame.displayHeight);
            drawing?.drawImage(frame, 0, 0, canvas.width, canvas.height);
            canvas.hidden = false;
            recordFrame("H.264", videoDecodeStarted.get(frame.timestamp));
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
    if (audioContext) { stopAudio(); return; }
    if (!ready || !audioAvailable) return;
    const session = generation;
    let context: AudioContext | undefined;
    try {
      const Audio = window.AudioContext || window.webkitAudioContext;
      context = new Audio({ sampleRate: 48000 });
      audioContext = context;
      updateControls();
      await context.resume();
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
      if (audioContext === context) {
        stopAudio(false);
        message(`Audio unavailable: ${errorMessage(error)}`);
      }
    }
  }

  function validText(text: string, limit = 65536) {
    if (new TextEncoder().encode(text).byteLength <= limit) return true;
    message(`Text is too large (maximum ${limit / 1024} KiB UTF-8).`);
    return false;
  }

  function typeText(text: string) {
    if (!ready || !text || !validText(text, 16384)) return;
    release();
    if (send({ type: "text", text })) message("Text sent.");
  }

  const capturedPointers = new Set<number>();
  function syncModifiers(event: KeyboardEvent | PointerEvent) {
    for (const [flag, left, right] of [
      ["ctrlKey", "ControlLeft", "ControlRight"], ["shiftKey", "ShiftLeft", "ShiftRight"],
      ["altKey", "AltLeft", "AltRight"], ["metaKey", "MetaLeft", "MetaRight"],
    ] as const) {
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
    const rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    send({ type: "move", x: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)),
      y: Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)) });
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
    if (!toolDrag.moved && Math.hypot(event.clientX - toolDrag.startX, event.clientY - toolDrag.startY) < 6) return;
    toolDrag.x = event.clientX - toolDrag.offsetX;
    toolDrag.y = event.clientY - toolDrag.offsetY;
    if (!toolDrag.moved) {
      toolDrag.moved = true;
      setToolsOpen(false);
    }
    layoutTools();
  });
  function finishToolDrag(event: PointerEvent) {
    if (!toolDrag || toolDrag.id !== event.pointerId) return;
    const drag = toolDrag;
    toolDrag = null;
    suppressToolClick = drag.moved;
    if (drag.moved && event.type === "pointerup") {
      const width = window.innerWidth || 1280;
      const height = window.innerHeight || 720;
      const centerX = Math.max(0, Math.min(width, event.clientX - drag.offsetX + 20));
      const centerY = Math.max(0, Math.min(height, event.clientY - drag.offsetY + 24));
      const distances: Record<string, number> = { left: centerX, right: width - centerX, top: centerY, bottom: height - centerY };
      const edge = Object.keys(distances).reduce((nearest, candidate) => distances[candidate] < distances[nearest] ? candidate : nearest);
      const horizontal = edge === "top" || edge === "bottom";
      const ratio = horizontal ? (centerX - 32) / Math.max(1, width - 64) : (centerY - 36) / Math.max(1, height - 72);
      toolPosition = { edge, ratio: Math.max(0, Math.min(1, ratio)) };
      saveToolPosition();
    } else layoutTools();
    if (toolHandle.hasPointerCapture(event.pointerId)) toolHandle.releasePointerCapture(event.pointerId);
  }
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"] as const) listen(toolHandle, name, finishToolDrag);
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
  listen(element("tools-close"), "click", () => setToolsOpen(false, true));
  listen(element("clipboard-toggle"), "click", () => { element("clipboard-section").open = !element("clipboard-section").open; });
  listen(element("settings-toggle"), "click", () => { element("settings-section").open = !element("settings-section").open; });
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
    if (!ready) return;
    event.preventDefault();
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1;
    const bounded = (value: number) => Number.isFinite(value) ? Math.max(-1000, Math.min(1000, value * scale)) : 0;
    send({ type: "scroll", x: bounded(event.deltaX), y: bounded(event.deltaY) });
  }, { passive: false });
  listen(canvas, "keydown", (event) => {
    if (!ready || event.isComposing || composing || event.keyCode === 229) return;
    if (event.code === "Escape" && event.ctrlKey && event.altKey && event.shiftKey) {
      event.preventDefault();
      release();
      keyboardCaptureWanted = false;
      setToolsOpen(true);
      element("connection").focus();
      return;
    }
    syncModifiers(event);
    // Outside capture, keep the browser paste event for local text transfer.
    // During capture, the chord belongs to the remote desktop and its clipboard.
    if (!keyboardCaptured && event.code === "KeyV" && (event.ctrlKey || event.metaKey)) return;
    const code = Object.hasOwn(KEY_CODES, event.code) ? KEY_CODES[event.code] : undefined;
    if (code === undefined) return;
    event.preventDefault();
    if (!event.repeat && !keys.has(code) && send({ type: "key", code, pressed: true })) keys.add(code);
  });
  listen(window, "keyup", (event) => {
    const code = KEY_CODES[event.code];
    if (keys.delete(code)) {
      event.preventDefault();
      send({ type: "key", code, pressed: false });
    }
    if (ready && document.activeElement === canvas) syncModifiers(event);
  });
  listen(canvas, "compositionstart", () => { composing = true; release(); });
  listen(canvas, "compositionend", (event) => { composing = false; typeText(event.data); });
  listen(textBox, "compositionstart", () => { composing = true; });
  listen(textBox, "compositionend", () => { composing = false; });
  listen(textBox, "blur", () => { composing = false; });
  listen(canvas, "paste", (event) => {
    event.preventDefault();
    typeText(event.clipboardData?.getData("text/plain") || "");
  });
  listen(canvas, "focus", () => { void captureKeyboard(); });
  listen(window, "focus", () => { void captureKeyboard(); });
  listen(canvas, "blur", () => { release(); unlockKeyboard(); });
  listen(window, "blur", () => { composing = false; release(); unlockKeyboard(); });
  listen(document, "visibilitychange", () => {
    if (document.hidden) { composing = false; release(); stopAudio(); unlockKeyboard(); }
    else scheduleAutoResize();
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
  listen(element("refresh-desktops"), "click", async () => {
    if (refreshingDesktops || switchingDesktop || !desktopChoices.length) return;
    const request = ++discoveryRequest;
    refreshingDesktops = true;
    updateControls();
    try {
      const response = await fetch(new URL("./desktops", location.href), { credentials: "same-origin", cache: "no-store" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const value = await response.json();
      if (request !== discoveryRequest || destroyed) return;
      if (!value || !Array.isArray(value.desktops)) throw new Error("无效的桌面列表");
      desktopStatus(ready ? `已连接${desktopLabel()}${selectedOutput ? ` · ${selectedOutput}` : ""}。` : "列表已刷新，当前连接尚未确认。");
      populateDesktops(value);
    } catch (error) {
      if (request === discoveryRequest && !destroyed) desktopStatus(`刷新失败：${errorMessage(error)}。仍可切回虚拟桌面。`);
    } finally {
      if (request === discoveryRequest) { refreshingDesktops = false; updateControls(); }
    }
  });
  async function displayMode(mode: string) {
    try {
      if (mode === "fullscreen") {
        await viewport.requestFullscreen();
        viewport.classList.toggle("fit", true);
      } else {
        windowMode = mode === "actual" ? "actual" : "fit";
        if (document.fullscreenElement) await document.exitFullscreen();
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
      const scaled = !!bounds && resizeScaleAvailable;
      const target = scaled ? mobileResolution(bounds.width, bounds.height - bounds.bar, window.devicePixelRatio || 1) : {
        width: Math.max(320, Math.min(3840, Math.floor(viewport.clientWidth / 2) * 2)),
        height: Math.max(200, Math.min(2160, Math.floor((bounds ? bounds.height - bounds.bar : viewport.clientHeight) / 2) * 2)), scale: 1,
      };
      const { width, height, scale } = target;
      const size = `${width}x${height}`;
      const resolution = `${size}@${scale}`;
      if (scaled ? lastAutoResolution === resolution : lastRequestedSize === size || (canvas.width === width && canvas.height === height)) return;
      release();
      if (send({ type: "resize", width, height, ...(scaled ? { scale } : {}) })) {
        lastAutoResolution = resolution;
        lastRequestedSize = size;
        element("width").value = String(width);
        element("height").value = String(height);
        message(`跟随窗口：${width} × ${height}`);
      }
    }, 250);
  }
  listen(window, "resize", scheduleAutoResize);
  listen(element("auto-resolution"), "change", () => { lastAutoResolution = null; scheduleAutoResize(); });
  listen(element("fullscreen"), "click", async () => {
    await displayMode(isDesktopFullscreen() ? windowMode : "fullscreen");
  });
  listen(document, "fullscreenchange", () => {
    const fullscreen = isDesktopFullscreen();
    const entering = fullscreen && !desktopFullscreen;
    desktopFullscreen = fullscreen;
    if (fullscreen) {
      if (fullscreenAutoResolution === null) {
        fullscreenAutoResolution = element("auto-resolution").checked;
        if (resizeAvailable) element("auto-resolution").checked = true;
      }
    } else if (fullscreenAutoResolution !== null) {
      element("auto-resolution").checked = fullscreenAutoResolution;
      fullscreenAutoResolution = null;
    }
    if (!fullscreen) { release(); unlockKeyboard(); }
    updateDisplayMode();
    viewport.classList.toggle("fit", fullscreen ? true : windowMode === "fit");
    scheduleAutoResize();
    if (entering) {
      keyboardCaptureWanted = true;
      setToolsOpen(false, true);
      canvas.focus({ preventScroll: true });
      void captureKeyboard();
    }
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
  });
  function sendChord(chord: number[]) {
    release();
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
    cancelTimer(autoResizeTimer);
    autoResizeTimer = null;
    lastRequestedSize = null;
    if (send({ type: "resize", width, height })) message("Resize requested; waiting for the server.");
  });
  listen(element("send-text"), "click", () => { if (!composing) typeText(textBox.value); });
  listen(element("clipboard-get"), "click", () => {
    clipboardPending = send({ type: "clipboard_get" });
  });
  listen(element("clipboard-set"), "click", () => {
    if (validText(textBox.value) && send({ type: "clipboard_set", text: textBox.value })) message("Remote clipboard update requested.");
  });
  listen(element("paste-local"), "click", async () => {
    const session = generation;
    try {
      const text = await navigator.clipboard.readText();
      if (session === generation && ready && !destroyed) typeText(text);
    } catch { message("Clipboard access denied or unavailable. Paste into the text box instead."); }
  });

  function destroy() {
    if (destroyed) return;
    disconnect();
    destroyed = true;
    mobile?.dispose();
    toolObserver?.disconnect();
    listeners.abort();
    if (socket) {
      socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
      socket = null;
    }
  }

  listen(window, "pagehide", (event) => {
    resumeWanted = wanted;
    if (event.persisted) disconnect();
    else destroy();
  });
  listen(window, "pageshow", (event) => {
    if (event.persisted && resumeWanted) { wanted = true; updateControls(); connect(); }
  });
  mobile = createMobileControls({
    canvas, viewport, input: element("mobile-input"), bar: element("mobile-shortcuts"), toggle: element("mobile-keyboard"),
    ready: () => ready, send, text: typeText, move, tools: (open) => setToolsOpen(open), translate,
    chord: (codes) => { if (ready && codes.every((code) => Object.hasOwn(KEY_CODES, code))) sendChord(codes.map((code) => KEY_CODES[code])); },
    layout: () => { layoutTools(); scheduleAutoResize(); },
  });
  wanted = true;
  setToolsOpen(false);
  updateDisplayMode();
  updateControls();
  if (drawing) connect();
  else { disconnect(); message("Canvas rendering is unavailable in this browser."); }
  return { destroy, refreshLanguage() {
    for (const [id, text] of texts) element(id).textContent = translate(text);
    if (desktopChoices.length) populateDesktops({ desktops: desktopChoices, outputs: desktopOutputs, reason: discoveryReason });
    updateControls(); updateDisplayMode(); layoutTools();
  }, layoutTools };
}
