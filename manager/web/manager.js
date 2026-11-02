(() => {
  "use strict";
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "懒猫远程", subtitle: "懒猫微服网络保障您的隐私和数据安全。", devices: "我的设备", empty: "暂无设备",
    help: "在电脑上运行懒猫远程，即可在这里管理。", loading: "正在获取设备…", connect: "连接桌面", consent: "需本机授权",
    failed: "操作失败，请确认设备在线后重试。", login_required: "登录已失效，请重新登录后刷新。", unavailable: "部分桌面来源暂不可用，其他设备仍可使用。",
    count: n => `${n} 台设备`, noPreview: "暂无预览", lastSession: "上次连接", start: "开启", stop: "关闭", working: "处理中…",
    screen: "屏幕录制", input: "键盘与鼠标", granted: "已授权", denied: "未授权", needs_user_action: "待确认", unknown: "待检查",
    disabled: "已关闭", running: "运行中", starting: "启动中", permission_required: "待授权", error: "启动失败", offline: "离线",
    permissions: "申请系统权限", requesting: "请在设备上确认权限", container: "LightOS实例桌面", footnote: "桌面仅在开启后运行，系统权限由设备本机确认。",
    stopped: "桌面进程已停止，可重新开启。",
  } : {
    title: "Lazycat Remote", subtitle: "The Lazycat network protects your privacy and data security.", devices: "My devices", empty: "No devices yet",
    help: "Run Lazycat Remote on your computer to manage it here.", loading: "Loading devices…", connect: "Connect", consent: "Local consent required",
    failed: "Operation failed. Check that the device is online and try again.", login_required: "Your session expired. Sign in again.", unavailable: "Some desktop sources are unavailable. Other devices remain available.",
    count: n => `${n} devices`, noPreview: "No preview", lastSession: "Last session", start: "Enable", stop: "Disable", working: "Working…",
    screen: "Screen", input: "Input", granted: "Granted", denied: "Not granted", needs_user_action: "Needs approval", unknown: "Not checked",
    disabled: "Disabled", running: "Running", starting: "Starting", permission_required: "Needs permission", error: "Failed", offline: "Offline",
    permissions: "Request permissions", requesting: "Approve permissions on the device", container: "LightOS instance desktop", footnote: "Desktop capture runs only when enabled. System permissions require approval on the device.",
    stopped: "The desktop process stopped. Try enabling it again.",
  };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = copy.title;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const grid = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty");
  const cards = new Map(), pendingActions = new Map();
  let busy = false, pending;
  const platforms = { darwin: "macOS", macos: "macOS", win32: "Windows", windows: "Windows", linux: "Linux" };
  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  }
  function createCard(item) {
    const card = element("article", "device-card"), cover = element("div", "cover"), image = element("img", "cover-image"), placeholder = element("div", "no-preview", copy.noPreview), caption = element("span", "preview-caption", copy.lastSession);
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 32 32"); svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "1.4"); svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS(svg.namespaceURI, "path"); path.setAttribute("d", "M5 5h22a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2ZM11 29h10M16 24v5"); svg.append(path); placeholder.prepend(svg);
    image.alt = ""; image.hidden = true; image.loading = "lazy"; caption.hidden = true;
    image.addEventListener("load", () => { image.hidden = false; placeholder.hidden = true; caption.hidden = false; });
    image.addEventListener("error", () => { image.hidden = true; placeholder.hidden = false; caption.hidden = true; });
    cover.append(image, placeholder, caption);
    const body = element("div", "card-body"), heading = element("div", "device-heading"), name = element("h2", "device-name"), badge = element("span", "badge"), platform = element("p", "platform"), permissions = element("div", "permissions"), screen = element("span", "permission"), input = element("span", "permission"), actions = element("div", "card-actions"), connect = element("a", "connect", copy.connect), toggle = element("button", "toggle"), request = element("button", "request-permissions"), error = element("p", "device-error");
    toggle.type = request.type = "button"; connect.target = "_blank"; connect.rel = "noopener noreferrer";
    const permissionStates = element("div", "permission-states");
    permissionStates.append(screen, input); permissions.append(permissionStates, request);
    heading.append(name, badge); actions.append(toggle, connect); body.append(heading, platform, permissions, error, actions); card.append(cover, body);
    card.parts = { image, placeholder, caption, name, badge, platform, permissions, screen, input, connect, toggle, request, error };
    toggle.addEventListener("click", () => act(card.target.id, card.target.enabled ? "stop" : "start"));
    request.addEventListener("click", () => act(card.target.id, "permissions"));
    grid.append(card); return card;
  }
  function updateCard(card, item) {
    card.target = item;
    const p = card.parts, permissions = item.permissions || {};
    const pending = pendingActions.get(item.id);
    if (pending && (Date.now() > pending.until || pending.action === "start" && item.enabled || pending.action === "stop" && !item.enabled || pending.action === "permissions" && permissions.requesting)) pendingActions.delete(item.id);
    const processing = pendingActions.has(item.id);
    p.name.textContent = item.name; p.name.title = item.name;
    p.platform.textContent = item.kind === "instance" ? copy.container : platforms[item.platform] || item.platform || "";
    const state = item.online === false ? "offline" : item.status || (item.ready ? "running" : "disabled");
    p.badge.textContent = copy[state] || copy.disabled; p.badge.dataset.state = state;
    if (item.preview) {
      const preview = new URL(item.preview, location.href);
      if (preview.origin === location.origin && preview.pathname.startsWith("/api/previews/")) {
        // A hidden lazy image never intersects the viewport, so its load event cannot reveal it.
        p.image.hidden = false;
        if (p.image.src !== preview.href) p.image.src = preview.href;
      }
    }
    for (const [kind, node] of [["screen", p.screen], ["input", p.input]]) {
      const value = ["granted", "denied", "needs_user_action"].includes(permissions[kind]) ? permissions[kind] : "unknown";
      node.textContent = `${copy[kind]} · ${copy[value]}`; node.dataset.granted = String(value === "granted");
    }
    p.permissions.hidden = false;
    p.toggle.hidden = !item.managed;
    p.toggle.textContent = processing ? copy.working : item.enabled ? copy.stop : copy.start;
    p.toggle.disabled = !item.online || processing;
    p.toggle.ariaLabel = `${item.enabled ? copy.stop : copy.start}: ${item.name}`;
    p.request.hidden = !item.managed || ![permissions.screen, permissions.input].some(value => value === "denied" || value === "needs_user_action");
    p.request.textContent = permissions.requesting ? copy.requesting : copy.permissions;
    p.request.disabled = !item.online || processing || permissions.requesting === true;
    const ready = item.online !== false && item.ready !== false;
    p.connect.setAttribute("aria-disabled", String(!ready)); p.connect.tabIndex = ready ? 0 : -1;
    if (ready) p.connect.href = item.url; else p.connect.removeAttribute("href");
    p.connect.ariaLabel = `${copy.connect}: ${item.name}`;
    p.error.textContent = item.agent_error === "desktop_process_stopped" ? copy.stopped : item.agent_error || "";
    p.error.hidden = !p.error.textContent;
  }
  function render(targets) {
    const ids = new Set(targets.map(item => item.id));
    for (const [id, card] of cards) if (!ids.has(id)) { card.remove(); cards.delete(id); }
    for (const item of targets) {
      let card = cards.get(item.id);
      if (!card) { card = createCard(item); cards.set(item.id, card); }
      updateCard(card, item);
    }
    empty.hidden = targets.length > 0; status.textContent = copy.count(targets.length);
  }
  async function act(id, action) {
    if (pendingActions.has(id)) return;
    pendingActions.set(id, { action, until: Date.now() + 15000 });
    const card = cards.get(id); if (card) updateCard(card, card.target);
    try {
      const response = await fetch(`/api/hosts/${encodeURIComponent(id)}/${action}`, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: "{}", signal: AbortSignal.timeout(16000) });
      if (response.redirected || response.status === 401 || response.status === 403) throw Error("login_required");
      if (!response.ok) throw Error("failed");
      warning.hidden = true;
      await load();
    } catch (error) {
      pendingActions.delete(id); warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (card) updateCard(card, card.target);
    }
  }
  async function load() {
    if (busy || document.hidden) return;
    busy = true;
    if (!cards.size) status.textContent = copy.loading;
    const controller = new AbortController(); pending = controller;
    const timeout = setTimeout(() => controller.abort(), 20000);
    try {
      const response = await fetch("/api/targets", { credentials: "same-origin", cache: "no-store", signal: controller.signal });
      if (response.redirected || response.status === 401 || response.status === 403) throw Error("login_required");
      if (!response.ok) throw Error("failed");
      const data = await response.json();
      if (!Array.isArray(data.targets)) throw Error("failed");
      const targets = data.targets.filter(item => {
        if (!item || typeof item.id !== "string" || typeof item.url !== "string" || typeof item.name !== "string") return false;
        try { const url = new URL(item.url, location.href); return url.protocol === "https:" && !url.username && !url.password || url.origin === location.origin && url.pathname.startsWith("/desktop/"); } catch { return false; }
      });
      render(targets); warning.textContent = copy.unavailable; warning.hidden = !data.unavailable?.length;
    } catch (error) {
      if (document.hidden && controller.signal.aborted) return;
      status.textContent = ""; warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (error.message === "login_required") render([]);
    } finally { clearTimeout(timeout); busy = false; pending = undefined; }
  }
  document.addEventListener("visibilitychange", () => { if (document.hidden) pending?.abort(); else void load(); });
  let timer = setInterval(load, 4000);
  window.addEventListener("pagehide", () => { pending?.abort(); clearInterval(timer); timer = undefined; });
  window.addEventListener("pageshow", event => { if (event.persisted) { timer ??= setInterval(load, 4000); void load(); } });
  void load();
})();
