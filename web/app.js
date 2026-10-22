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
  const controls = ["apply-size", "send-text", "clipboard-get", "clipboard-set", "paste-local"];
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

  function listen(target, name, handler, options = {}) {
    target.addEventListener(name, handler, { ...options, signal: listeners.signal });
  }

  function message(value) { element("message").textContent = value; }

  function validDimensions(width, height) {
    return Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 &&
      width <= 8192 && height <= 8192 && width * height <= 33554432;
  }

  function resizeCanvas(width, height) {
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
  }

  function updateControls() {
    for (const id of controls) element(id).disabled = !ready;
    element("audio").disabled = !ready || !audioAvailable;
    element("audio").textContent = audioContext ? "Disable audio" : "Enable audio";
    element("audio").setAttribute("aria-pressed", String(audioEnabled));
    element("connection").textContent = wanted ? "Disconnect" : "Connect";
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
    ready = false;
    generation += 1;
    pendingFrame = null;
    clipboardPending = false;
    audioAvailable = false;
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
          pendingFrame = { data: event.data.slice(1), generation };
          void decodeFrames();
        } else if (kind === 2) {
          receiveAudio(event.data);
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
      clearTimeout(connectionTimer);
      connectionTimer = null;
      retries = 0;
      audioAvailable = value.audio === true;
      if (!audioAvailable) stopAudio(false);
      element("state").textContent = "Connected";
      message(audioAvailable ? `Desktop ready. Audio is ${audioEnabled ? "on" : "off until enabled"}.` : "Desktop ready. Server audio is unavailable.");
      updateControls();
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
          if (frame.generation === generation && ready && !destroyed) {
            if (!validDimensions(bitmap.width, bitmap.height)) {
              message("Server supplied invalid JPEG dimensions.");
              continue;
            }
            resizeCanvas(bitmap.width, bitmap.height);
            drawing.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
          }
        } catch {
          if (frame.generation === generation && ready) message("Could not decode a desktop JPEG frame.");
        } finally {
          if (bitmap) bitmap.close();
        }
      }
    } finally { decoding = false; }
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

  listen(canvas, "pointerdown", (event) => {
    if (!ready || event.button < 0 || event.button > 2) return;
    event.preventDefault();
    canvas.focus({ preventScroll: true });
    syncModifiers(event);
    canvas.setPointerCapture(event.pointerId);
    capturedPointers.add(event.pointerId);
    move(event);
    if (!buttons.has(event.button) && send({ type: "button", button: event.button, pressed: true })) buttons.add(event.button);
  });
  listen(canvas, "pointermove", move);
  listen(canvas, "pointerup", (event) => {
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
  listen(canvas, "blur", release);
  listen(window, "blur", () => { composing = false; release(); });
  listen(document, "visibilitychange", () => {
    if (document.hidden) { composing = false; release(); stopAudio(); }
  });
  listen(element("connection"), "click", () => {
    if (wanted) disconnect();
    else { wanted = true; retries = 0; updateControls(); connect(); }
  });
  listen(element("audio"), "click", () => { void toggleAudio(); });
  listen(element("fit"), "change", (event) => viewport.classList.toggle("fit", event.target.checked));
  listen(element("fullscreen"), "click", async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await viewport.requestFullscreen();
      canvas.focus({ preventScroll: true });
    } catch (error) { message(`Fullscreen unavailable: ${error.message}`); }
  });
  listen(document, "fullscreenchange", () => {
    element("fullscreen").textContent = document.fullscreenElement ? "Exit fullscreen" : "Fullscreen";
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
  updateControls();
  if (drawing) connect();
  else { disconnect(); message("Canvas rendering is unavailable in this browser."); }
  return { destroy };
}

const client = createClient();
