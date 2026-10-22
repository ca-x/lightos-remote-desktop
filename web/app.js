"use strict";

const KEY_CODES = Object.freeze({
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

function createClient() {
  const element = (id) => document.getElementById(id);
  const canvas = element("desktop");
  const drawing = canvas.getContext("2d", { alpha: false });
  const viewport = element("viewport");
  const textBox = element("text");
  const controls = ["apply-size", "send-text", "clipboard-get", "clipboard-set", "paste-local", "keyboard-capture", "send-shortcut"];
  const listeners = new AbortController();
  const keys = new Set();
  const buttons = new Set();
  const sources = new Set();
  let socket = null;
  let wanted = false;
  let ready = false;
  let destroyed = false;
  let generation = 0;
  let retryTimer = null;
  let connectionTimer = null;
  let retries = 0;
  let pendingFrame = null;
  let decoding = false;
  let audioAvailable = false;
  let audioContext = null;
  let audioEnabled = false;
  let nextAudioTime = 0;
  let composing = false;
  let clipboardPending = false;
  let resumeWanted = false;
  let moveFrame = null;
  let pendingMove = null;
  let frameAcknowledgements = false;
  let windowMode = "fit";
  let videoDecoder = null;
  let videoToken = 0;
  let configuringVideo = false;
  let pendingVideo = null;
  let videoTimer = null;
  let sequencedFrames = false;
  let pendingFrameId = null;
  const videoFrameIds = new Map();
  let autoResizeTimer = null;
  let lastRequestedSize = null;
  let keyboardCaptured = false;
  let keyboardCaptureWanted = true;
  let keyboardRequest = false;
  let keyboardToken = 0;

  function updateKeyboardCapture() {
    element("keyboard-capture").setAttribute("aria-pressed", String(keyboardCaptured));
    element("keyboard-capture").textContent = keyboardCaptured ? "释放键盘捕获" : "全屏捕获键盘";
  }

  function unlockKeyboard() {
    keyboardToken += 1;
    try { navigator.keyboard?.unlock(); } catch {}
    keyboardCaptured = false;
    element("keyboard-state").textContent = "未捕获键盘。全屏可尝试捕获；系统拦截的按键可通过通用组合键发送器发送。";
    updateKeyboardCapture();
  }

  async function captureKeyboard() {
    if (!ready || destroyed || keyboardRequest || keyboardCaptured || !keyboardCaptureWanted || document.hidden || !document.hasFocus() || document.fullscreenElement !== viewport || document.activeElement !== canvas) return;
    if (!navigator.keyboard?.lock) {
      element("keyboard-state").textContent = "浏览器不支持键盘捕获。系统截走的组合键可用下方通用发送器发送。";
      return;
    }
    const token = keyboardToken;
    keyboardRequest = true;
    try {
      await navigator.keyboard.lock();
      if (token !== keyboardToken || !ready || destroyed || document.hidden || !document.hasFocus() || document.fullscreenElement !== viewport || document.activeElement !== canvas) {
        navigator.keyboard.unlock();
        return;
      }
      keyboardCaptured = true;
      updateKeyboardCapture();
      element("keyboard-state").textContent = "已请求捕获所有可用按键。部分系统快捷键仍可能被本机拦截。Ctrl+Alt+Shift+Esc 释放捕获并打开工具栏。";
    } catch {
      if (token === keyboardToken) element("keyboard-state").textContent = "键盘捕获被拒绝，请检查浏览器权限或使用下方通用发送器。";
    } finally {
      keyboardRequest = false;
      if (token !== keyboardToken) void captureKeyboard();
    }
  }
  const toolHandle = element("tools-toggle");
  const tools = element("tools");
  const toolPositionKey = "lightos-desktop-toolbar-position";
  let toolPosition = { edge: "right", ratio: 0.5 };
  let toolDrag = null;
  let suppressToolClick = false;
  try {
    const saved = JSON.parse(localStorage.getItem(toolPositionKey));
    if (["left", "right", "top", "bottom"].includes(saved?.edge) && Number.isFinite(saved.ratio)) {
      toolPosition = { edge: saved.edge, ratio: Math.max(0, Math.min(1, saved.ratio)) };
    }
  } catch {}

  function layoutTools(point = null) {
    const width = window.innerWidth || 1280;
    const height = window.innerHeight || 720;
    const maxX = Math.max(12, width - 52);
    const maxY = Math.max(12, height - 60);
    const horizontal = toolPosition.edge === "top" || toolPosition.edge === "bottom";
    const left = point ? Math.max(12, Math.min(maxX, point.x)) :
      horizontal ? 12 + toolPosition.ratio * (maxX - 12) : toolPosition.edge === "left" ? 12 : maxX;
    const top = point ? Math.max(12, Math.min(maxY, point.y)) :
      horizontal ? toolPosition.edge === "top" ? 12 : maxY : 12 + toolPosition.ratio * (maxY - 12);
    tools.style.setProperty("--tool-x", `${left}px`);
    tools.style.setProperty("--tool-y", `${top}px`);
    if (element("tool-panel").hidden) return;
    const panel = element("tool-panel").getBoundingClientRect();
    const panelX = toolPosition.edge === "left" ? left + 48 : toolPosition.edge === "right" ? left - panel.width - 8 : left + 20 - panel.width / 2;
    const panelY = toolPosition.edge === "top" ? top + 56 : toolPosition.edge === "bottom" ? top - panel.height - 8 : top + 24 - panel.height / 2;
    tools.style.setProperty("--panel-x", `${Math.max(12, Math.min(width - panel.width - 12, panelX))}px`);
    tools.style.setProperty("--panel-y", `${Math.max(12, Math.min(height - panel.height - 12, panelY))}px`);
  }

  function saveToolPosition() {
    try { localStorage.setItem(toolPositionKey, JSON.stringify(toolPosition)); } catch {}
    layoutTools();
  }
  let metricsTimer = null;
  let metricsStarted = 0;
  let renderedFrames = 0;
  let pingSequence = 0;
  let pendingPing = null;
  let latestRtt = null;
  const rttSamples = [];

  function recordFrame(codec) {
    renderedFrames += 1;
    element("video-details").textContent = `${codec} · ${canvas.width} × ${canvas.height}`;
  }

  function stopMetrics() {
    clearTimeout(metricsTimer);
    metricsTimer = null;
    pendingPing = null;
    latestRtt = null;
    rttSamples.length = 0;
    renderedFrames = 0;
    element("performance").textContent = "-- FPS · RTT -- ms";
    element("video-details").textContent = "等待画面";
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
    element("performance").textContent = `${fps.toFixed(1)} FPS · RTT ${rtt} ms · P95 ${p95} ms`;
    if (pendingPing && now - pendingPing.started >= 5000) {
      pendingPing = null;
      latestRtt = null;
      element("performance").textContent = `${fps.toFixed(1)} FPS · RTT 超时 · P95 ${p95} ms`;
    }
    if (!pendingPing && !document.hidden) {
      const id = ++pingSequence;
      pendingPing = { id, started: now };
      if (!send({ type: "ping", id })) pendingPing = null;
    }
    if (ready && !destroyed) metricsTimer = setTimeout(sampleMetrics, 1000);
  }

  function listen(target, name, handler, options = {}) {
    target.addEventListener(name, handler, { ...options, signal: listeners.signal });
  }

  function message(value) { element("message").textContent = value; }

  function setToolsOpen(open, focus = false) {
    if (open) unlockKeyboard();
    element("tool-panel").hidden = !open;
    element("tools-toggle").setAttribute("aria-expanded", String(open));
    element("tools-toggle").setAttribute("aria-label", open ? "收起桌面工具栏" : "打开桌面工具栏");
    layoutTools();
    if (focus) (open ? element("connection") : canvas).focus({ preventScroll: true });
  }

  function validDimensions(width, height) {
    return Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 &&
      width <= 8192 && height <= 8192 && width * height <= 33554432;
  }

  function resizeCanvas(width, height) {
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    canvas.style.setProperty("--desktop-aspect", String(width / height));
  }

  function updateControls() {
    for (const id of controls) element(id).disabled = !ready;
    element("audio").disabled = !ready || !audioAvailable;
    const audioLabel = audioContext ? "关闭声音" : "开启声音";
    for (const attribute of ["aria-label", "title", "data-tip"]) element("audio").setAttribute(attribute, audioLabel);
    element("audio-icon").setAttribute("href", audioEnabled ? "#icon-volume" : "#icon-muted");
    element("audio").setAttribute("aria-pressed", String(audioEnabled));
    for (const attribute of ["aria-label", "title", "data-tip"]) element("connection").setAttribute(attribute, wanted ? "断开连接" : "重新连接");
  }

  function send(command) {
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
    if (notify && audioEnabled) send({ type: "audio", enabled: false });
    audioEnabled = false;
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
    unlockKeyboard();
    stopMetrics();
    clearTimeout(autoResizeTimer);
    autoResizeTimer = null;
    lastRequestedSize = null;
    ready = false;
    generation += 1;
    pendingFrame = null;
    clipboardPending = false;
    audioAvailable = false;
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

  function connect() {
    if (!wanted || destroyed || socket || retryTimer !== null) return;
    element("state").textContent = "Connecting";
    const url = new URL("./ws", location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    let current;
    try { current = new WebSocket(url); } catch (error) {
      message(`Could not connect: ${error.message}`);
      scheduleRetry();
      return;
    }
    socket = current;
    connectionTimer = setTimeout(() => {
      connectionTimer = null;
      if (socket !== current || ready) return;
      message("Desktop readiness timed out; reconnecting.");
      current.close(4000, "Readiness timeout");
      resetSession();
    }, 15000);
    current.binaryType = "arraybuffer";
    current.onopen = () => {
      if (current !== socket || !wanted || destroyed) { current.close(); return; }
      element("state").textContent = "Waiting for desktop";
      message("Connected; waiting for server readiness.");
    };
    current.onmessage = (event) => {
      if (current !== socket || !wanted || destroyed) return;
      if (typeof event.data === "string") {
        receiveJSON(event.data);
      } else if (ready && event.data instanceof ArrayBuffer && event.data.byteLength > 1) {
        const kind = new Uint8Array(event.data, 0, 1)[0];
        if (kind === 1 && event.data.byteLength <= 16777216) {
          pendingFrame = { data: event.data.slice(1), generation, videoToken, id: pendingFrameId };
          pendingFrameId = null;
          void decodeFrames();
        } else if (kind === 2) {
          receiveAudio(event.data);
        } else if (kind === 3 && event.data.byteLength <= 16777216) {
          const id = pendingFrameId;
          pendingFrameId = null;
          receiveVideo(event.data, id);
        }
      }
    };
    current.onerror = () => {
      if (current !== socket) return;
      message("Connection failed. Check browser authentication, server availability, or another active controller.");
      current.close();
    };
    current.onclose = () => {
      if (current !== socket) return;
      clearTimeout(connectionTimer);
      connectionTimer = null;
      socket = null;
      resetSession();
      element("state").textContent = "Disconnected";
      scheduleRetry();
    };
  }

  function scheduleRetry() {
    if (!wanted || destroyed || retryTimer !== null) return;
    const delay = Math.min(10000, 500 * 2 ** Math.min(retries++, 5));
    element("state").textContent = `Reconnecting in ${delay / 1000}s`;
    retryTimer = setTimeout(() => {
      retryTimer = null;
      connect();
    }, delay);
  }

  function disconnect() {
    wanted = false;
    clearTimeout(retryTimer);
    retryTimer = null;
    clearTimeout(connectionTimer);
    connectionTimer = null;
    release();
    stopAudio();
    resetSession();
    if (socket) socket.close(1000, "Client disconnected");
    element("state").textContent = "Disconnected";
    updateControls();
  }

  function receiveJSON(raw) {
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
      if (document.activeElement !== element("width") && document.activeElement !== element("height")) {
        element("width").value = width;
        element("height").value = height;
      }
      ready = true;
      if (value.metrics === true) {
        stopMetrics();
        metricsStarted = performance.now();
        sampleMetrics();
      }
      frameAcknowledgements = value.frame_ack === true;
      sequencedFrames = value.frame_ids === true;
      if (frameAcknowledgements) send({ type: "video_flow", enabled: true, sequenced: sequencedFrames });
      if (frameAcknowledgements && sequencedFrames && value.video_codecs?.includes("h264") && window.VideoDecoder && window.EncodedVideoChunk) send({ type: "video", codec: "h264" });
      clearTimeout(connectionTimer);
      connectionTimer = null;
      retries = 0;
      audioAvailable = value.audio === true;
      if (!audioAvailable) stopAudio(false);
      element("state").textContent = "Connected";
      message(audioAvailable ? `Desktop ready. Audio is ${audioEnabled ? "on" : "off until enabled"}.` : "Desktop ready. Server audio is unavailable.");
      updateControls();
      scheduleAutoResize();
      void captureKeyboard();
    } else if (value.type === "pong" && pendingPing && value.id === pendingPing.id) {
      latestRtt = Math.max(0, performance.now() - pendingPing.started);
      rttSamples.push(latestRtt);
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
      message(`Server: ${value.message.slice(0, 2048)}`);
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
        try {
          bitmap = await createImageBitmap(new Blob([frame.data], { type: "image/jpeg" }));
          if (frame.generation === generation && frame.videoToken === videoToken && ready && !destroyed) {
            if (!validDimensions(bitmap.width, bitmap.height)) {
              message("Server supplied invalid JPEG dimensions.");
              continue;
            }
            resizeCanvas(bitmap.width, bitmap.height);
            drawing.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
            recordFrame("JPEG");
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
    clearTimeout(videoTimer);
    videoTimer = null;
    videoToken += 1;
    configuringVideo = false;
    pendingVideo = null;
    videoFrameIds.clear();
    const previous = videoDecoder;
    videoDecoder = null;
    if (previous && previous.state !== "closed") previous.close();
  }

  function acknowledgeFrame(id) {
    if (sequencedFrames) {
      if (Number.isSafeInteger(id) && id > 0) send({ type: "frame_ack", id });
    } else send({ type: "frame_ack" });
  }

  function fallbackVideo() {
    closeVideo();
    send({ type: "video", codec: "jpeg" });
    message("H.264 is unavailable; using compatible JPEG video.");
  }

  async function configureVideo(value) {
    closeVideo();
    pendingFrame = null;
    if (value.codec === "jpeg") { message("Video: JPEG compatibility mode."); return; }
    if (!/^avc1\.[0-9a-f]{6}$/i.test(value.codec) || !window.VideoDecoder || !validDimensions(value.width, value.height)) { fallbackVideo(); return; }
    const token = videoToken;
    const session = generation;
    configuringVideo = true;
    try {
      const config = { codec: value.codec, codedWidth: value.width, codedHeight: value.height, optimizeForLatency: true, hardwareAcceleration: "prefer-hardware" };
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
            clearTimeout(videoTimer);
            videoTimer = null;
            if (!validDimensions(frame.displayWidth, frame.displayHeight)) { fallbackVideo(); return; }
            resizeCanvas(frame.displayWidth, frame.displayHeight);
            drawing.drawImage(frame, 0, 0, canvas.width, canvas.height);
            recordFrame("H.264");
            const id = videoFrameIds.get(frame.timestamp);
            videoFrameIds.delete(frame.timestamp);
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

  function receiveVideo(data, id) {
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
      videoTimer = setTimeout(() => { videoTimer = null; fallbackVideo(); }, 2000);
      videoDecoder.decode(new window.EncodedVideoChunk({ type: view.getUint8(1) === 1 ? "key" : "delta", timestamp, data: new Uint8Array(data, 10) }));
    } catch { fallbackVideo(); }
  }

  function receiveAudio(data) {
    if (!audioEnabled || !audioContext || audioContext.state !== "running" || document.hidden) return;
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

  async function toggleAudio() {
    if (audioContext) { stopAudio(); return; }
    if (!ready || !audioAvailable) return;
    const session = generation;
    let context;
    try {
      const Audio = window.AudioContext || window.webkitAudioContext;
      context = new Audio({ sampleRate: 48000 });
      audioContext = context;
      updateControls();
      await context.resume();
      if (session !== generation || audioContext !== context || !ready || document.hidden) {
        if (audioContext === context) stopAudio();
        return;
      }
      audioEnabled = send({ type: "audio", enabled: true });
      if (!audioEnabled) stopAudio(false);
      updateControls();
    } catch (error) {
      if (audioContext === context) stopAudio(false);
      message(`Audio unavailable: ${error.message}`);
    }
  }

  function validText(text, limit = 65536) {
    if (new TextEncoder().encode(text).byteLength <= limit) return true;
    message(`Text is too large (maximum ${limit / 1024} KiB UTF-8).`);
    return false;
  }

  function typeText(text) {
    if (!ready || !text || !validText(text, 16384)) return;
    release();
    if (send({ type: "text", text })) message("Text sent.");
  }

  const capturedPointers = new Set();
  function syncModifiers(event) {
    for (const [flag, left, right] of [
      ["ctrlKey", "ControlLeft", "ControlRight"], ["shiftKey", "ShiftLeft", "ShiftRight"],
      ["altKey", "AltLeft", "AltRight"], ["metaKey", "MetaLeft", "MetaRight"],
    ]) {
      if (event[flag]) {
        if (event.code !== left && event.code !== right && !keys.has(KEY_CODES[left]) && !keys.has(KEY_CODES[right])) {
          if (send({ type: "key", code: KEY_CODES[left], pressed: true })) keys.add(KEY_CODES[left]);
        }
      } else {
        for (const code of [KEY_CODES[left], KEY_CODES[right]]) {
          if (keys.delete(code)) send({ type: "key", code, pressed: false });
        }
      }
    }
  }

  function move(event) {
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
    if (suppressToolClick && event.detail !== 0) { suppressToolClick = false; return; }
    setToolsOpen(element("tool-panel").hidden, true);
  });
  listen(toolHandle, "pointerdown", (event) => {
    if (event.button !== 0 || toolDrag) return;
    suppressToolClick = false;
    const bounds = toolHandle.getBoundingClientRect();
    toolDrag = { id: event.pointerId, startX: event.clientX, startY: event.clientY,
      offsetX: event.clientX - bounds.left, offsetY: event.clientY - bounds.top, moved: false };
    toolHandle.setPointerCapture(event.pointerId);
  });
  listen(toolHandle, "pointermove", (event) => {
    if (!toolDrag || toolDrag.id !== event.pointerId) return;
    if (!toolDrag.moved && Math.hypot(event.clientX - toolDrag.startX, event.clientY - toolDrag.startY) < 6) return;
    toolDrag.moved = true;
    setToolsOpen(false);
    layoutTools({ x: event.clientX - toolDrag.offsetX, y: event.clientY - toolDrag.offsetY });
  });
  function finishToolDrag(event) {
    if (!toolDrag || toolDrag.id !== event.pointerId) return;
    const drag = toolDrag;
    toolDrag = null;
    suppressToolClick = drag.moved;
    if (drag.moved && event.type === "pointerup") {
      const width = window.innerWidth || 1280;
      const height = window.innerHeight || 720;
      const centerX = Math.max(0, Math.min(width, event.clientX - drag.offsetX + 20));
      const centerY = Math.max(0, Math.min(height, event.clientY - drag.offsetY + 24));
      const distances = { left: centerX, right: width - centerX, top: centerY, bottom: height - centerY };
      const edge = Object.keys(distances).reduce((nearest, candidate) => distances[candidate] < distances[nearest] ? candidate : nearest);
      const horizontal = edge === "top" || edge === "bottom";
      const ratio = horizontal ? (centerX - 32) / Math.max(1, width - 64) : (centerY - 36) / Math.max(1, height - 72);
      toolPosition = { edge, ratio: Math.max(0, Math.min(1, ratio)) };
      saveToolPosition();
    } else layoutTools();
    if (toolHandle.hasPointerCapture(event.pointerId)) toolHandle.releasePointerCapture(event.pointerId);
  }
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) listen(toolHandle, name, finishToolDrag);
  listen(toolHandle, "keydown", (event) => {
    const edges = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "top", ArrowDown: "bottom" };
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
    if (event.key === "Escape" && !element("tool-panel").hidden && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      event.preventDefault();
      setToolsOpen(false, true);
    }
  });

  listen(canvas, "pointerdown", (event) => {
    if (!ready || event.button < 0 || event.button > 2) return;
    event.preventDefault();
    setToolsOpen(false);
    canvas.focus({ preventScroll: true });
    syncModifiers(event);
    canvas.setPointerCapture(event.pointerId);
    capturedPointers.add(event.pointerId);
    pendingMove = event;
    flushMove();
    if (!buttons.has(event.button) && send({ type: "button", button: event.button, pressed: true })) buttons.add(event.button);
  });
  listen(canvas, "pointermove", (event) => {
    if (!ready) return;
    pendingMove = { clientX: event.clientX, clientY: event.clientY };
    if (moveFrame === null) moveFrame = requestAnimationFrame(flushMove);
  });
  listen(canvas, "pointerup", (event) => {
    flushMove();
    if (buttons.delete(event.button)) send({ type: "button", button: event.button, pressed: false });
    if (!buttons.size) {
      capturedPointers.delete(event.pointerId);
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    }
  });
  listen(canvas, "pointercancel", release);
  listen(canvas, "lostpointercapture", (event) => {
    if (capturedPointers.has(event.pointerId)) release();
  });
  listen(canvas, "contextmenu", (event) => event.preventDefault());
  listen(canvas, "wheel", (event) => {
    if (!ready) return;
    event.preventDefault();
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1;
    const bounded = (value) => Number.isFinite(value) ? Math.max(-1000, Math.min(1000, value * scale)) : 0;
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
    if (event.code === "KeyV" && (event.ctrlKey || event.metaKey)) return;
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
    else { wanted = true; retries = 0; updateControls(); connect(); }
  });
  listen(element("audio"), "click", () => { void toggleAudio(); });
  async function displayMode(mode) {
    try {
      if (mode === "fullscreen") {
        await viewport.requestFullscreen();
        viewport.classList.toggle("fit", true);
      } else {
        windowMode = mode === "actual" ? "actual" : "fit";
        if (windowMode === "actual") {
          element("auto-resolution").checked = false;
          clearTimeout(autoResizeTimer);
          autoResizeTimer = null;
        }
        if (document.fullscreenElement) await document.exitFullscreen();
        viewport.classList.toggle("fit", windowMode === "fit");
      }
      canvas.focus({ preventScroll: true });
      if (mode === "fullscreen") await captureKeyboard();
      updateDisplayMode();
    } catch (error) {
      updateDisplayMode();
      message(`Fullscreen unavailable: ${error.message}`);
    }
  }
  function updateDisplayMode() {
    const mode = document.fullscreenElement ? "fullscreen" : windowMode;
    element("mode-fit").setAttribute("aria-pressed", String(mode === "fit"));
    element("mode-actual").setAttribute("aria-pressed", String(mode === "actual"));
    element("fullscreen").setAttribute("aria-pressed", String(mode === "fullscreen"));
    element("display-mode").textContent = mode === "actual" ? "原始大小 · 按设定分辨率 1:1 显示" : mode === "fullscreen" ? "全屏 · Ctrl+Alt+Shift+Esc 释放键盘" : "适应窗口";
    for (const attribute of ["aria-label", "title", "data-tip"]) element("fullscreen").setAttribute(attribute, mode === "fullscreen" ? "退出全屏" : "全屏");
  }
  listen(element("mode-fit"), "click", () => { void displayMode("fit"); });
  listen(element("mode-actual"), "click", () => { void displayMode("actual"); });
  function scheduleAutoResize() {
    clearTimeout(autoResizeTimer);
    autoResizeTimer = null;
    if (!ready || !element("auto-resolution").checked) return;
    autoResizeTimer = setTimeout(() => {
      autoResizeTimer = null;
      if (!ready || !element("auto-resolution").checked || document.hidden || viewport.clientWidth <= 0 || viewport.clientHeight <= 0) return;
      const width = Math.max(320, Math.min(3840, Math.floor(viewport.clientWidth / 2) * 2));
      const height = Math.max(200, Math.min(2160, Math.floor(viewport.clientHeight / 2) * 2));
      const size = `${width}x${height}`;
      if (lastRequestedSize === size || (canvas.width === width && canvas.height === height)) return;
      release();
      if (send({ type: "resize", width, height })) {
        lastRequestedSize = size;
        element("width").value = width;
        element("height").value = height;
        message(`跟随窗口：${width} × ${height}`);
      }
    }, 250);
  }
  listen(window, "resize", scheduleAutoResize);
  listen(element("auto-resolution"), "change", scheduleAutoResize);
  listen(element("fullscreen"), "click", async () => {
    await displayMode(document.fullscreenElement ? windowMode : "fullscreen");
  });
  listen(document, "fullscreenchange", () => {
    if (document.fullscreenElement !== viewport) { release(); unlockKeyboard(); }
    updateDisplayMode();
    viewport.classList.toggle("fit", document.fullscreenElement ? true : windowMode === "fit");
    scheduleAutoResize();
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
    if (document.fullscreenElement !== viewport) await displayMode("fullscreen");
    else { canvas.focus({ preventScroll: true }); await captureKeyboard(); }
  });
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
    const chord = [];
    for (const [id, code] of [["shortcut-ctrl", "ControlLeft"], ["shortcut-alt", "AltLeft"], ["shortcut-shift", "ShiftLeft"], ["shortcut-super", "MetaLeft"]]) {
      if (element(id).checked) chord.push(KEY_CODES[code]);
    }
    if (!chord.includes(KEY_CODES[mainKey])) chord.push(KEY_CODES[mainKey]);
    release();
    for (const code of chord) {
      if (!send({ type: "key", code, pressed: true })) return;
      keys.add(code);
    }
    for (const code of [...chord].reverse()) {
      if (!send({ type: "key", code, pressed: false })) return;
      keys.delete(code);
    }
    message("组合键已发送到远程桌面。");
  });
  listen(element("resize"), "submit", (event) => {
    event.preventDefault();
    const width = Number(element("width").value);
    const height = Number(element("height").value);
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 320 || width > 3840 || height < 200 || height > 2160) {
      message("Resolution must be 320–3840 × 200–2160 whole pixels.");
      return;
    }
    release();
    element("auto-resolution").checked = false;
    clearTimeout(autoResizeTimer);
    autoResizeTimer = null;
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
  wanted = true;
  setToolsOpen(false);
  updateDisplayMode();
  updateControls();
  if (drawing) connect();
  else { disconnect(); message("Canvas rendering is unavailable in this browser."); }
  return { destroy };
}

const client = createClient();
