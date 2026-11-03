(() => {
  "use strict";
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "懒猫远程", subtitle: "懒猫微服网络保障您的隐私和数据安全。", devices: "我的设备", empty: "暂无设备",
    help: "在电脑上登录懒猫微服客户端，即可在这里管理。", loading: "正在获取设备…", connect: "连接桌面", consent: "需本机授权",
    failed: "操作失败，请确认设备在线后重试。", login_required: "登录已失效，请重新登录后刷新。", unavailable: "部分桌面来源暂不可用，其他设备仍可使用。",
    count: n => `${n} 台设备`, noPreview: "暂无预览", start: "开启", stop: "关闭", working: "处理中…",
    screen: "屏幕录制", input: "键盘与鼠标", granted: "已授权", denied: "未授权", needs_user_action: "待确认", unknown: "待检查",
    updating: "正在更新", disabled: "已关闭", running: "运行中", starting: "启动中", permission_required: "待授权", error: "启动失败", offline: "离线",
    permissions: "申请系统权限", requesting: "请在设备上确认权限", container: "LightOS实例桌面", footnote: "桌面仅在开启后运行，系统权限由设备本机确认。",
    stopped: "桌面进程已停止，可重新开启。", runtime: "远程桌面", not_installed: "未安装", installedUnknown: "已安装（版本未知）", latest: "最新", install: "安装远程桌面", update: "更新远程桌面", instanceManaged: "由 LightOS 管理",
    interruptNotice: "更新会中断当前远程桌面连接。确认后将自动关闭服务并更新，完成后恢复服务。", cancel: "取消", confirm: "确认", stopping: "正在关闭远程桌面",
    preparing: "正在准备", downloading: "正在下载", verifying: "正在校验", installing: "正在安装", checking: "正在检查新版本", cleaning: "正在清理旧版本", failedInstall: "安装 / 更新失败，可重试", retryOperation: "重试",
  } : {
    title: "Lazycat Remote", subtitle: "The Lazycat network protects your privacy and data security.", devices: "My devices", empty: "No devices yet",
    help: "Sign in to the Lazycat client on your computer to manage it here.", loading: "Loading devices…", connect: "Connect", consent: "Local consent required",
    failed: "Operation failed. Check that the device is online and try again.", login_required: "Your session expired. Sign in again.", unavailable: "Some desktop sources are unavailable. Other devices remain available.",
    count: n => `${n} devices`, noPreview: "No preview", start: "Enable", stop: "Disable", working: "Working…",
    screen: "Screen", input: "Input", granted: "Granted", denied: "Not granted", needs_user_action: "Needs approval", unknown: "Not checked",
    updating: "Updating", disabled: "Disabled", running: "Running", starting: "Starting", permission_required: "Needs permission", error: "Failed", offline: "Offline",
    permissions: "Request permissions", requesting: "Approve permissions on the device", container: "LightOS instance desktop", footnote: "Desktop capture runs only when enabled. System permissions require approval on the device.",
    stopped: "The desktop process stopped. Try enabling it again.", runtime: "Remote desktop", not_installed: "Not installed", installedUnknown: "Installed (version unknown)", latest: "Latest", install: "Install remote desktop", update: "Update remote desktop", instanceManaged: "Managed by LightOS",
    interruptNotice: "Updating will disconnect the current remote desktop session. Confirm to stop the service and update it. The service will resume when the update finishes.", cancel: "Cancel", confirm: "Confirm", stopping: "Stopping remote desktop",
    preparing: "Preparing", downloading: "Downloading", verifying: "Verifying", installing: "Installing", checking: "Checking the new version", cleaning: "Removing the old version", failedInstall: "Installation / update failed; retry is available", retryOperation: "Retry",
  };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = copy.title;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const grid = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty");
  const cards = new Map(), pendingActions = new Map();
  let busy = false, pending, reloadRequested = false, refreshedAt = 0, controlRevision = 0;
  const platforms = { darwin: "macOS", macos: "macOS", win32: "Windows", windows: "Windows", linux: "Linux" };
  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  }
  function confirmUpdate() {
    const dialog = document.getElementById("update-confirmation");
    if (dialog.open) return Promise.resolve(false);
    dialog.returnValue = "cancel";
    return new Promise(resolve => {
      dialog.addEventListener("close", () => resolve(dialog.returnValue === "confirm"), { once: true });
      dialog.showModal();
    });
  }
  function createCard(item) {
    const card = element("article", "device-card"), cover = element("div", "cover"), image = element("img", "cover-image"), placeholder = element("div", "no-preview", copy.noPreview);
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 32 32"); svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "1.4"); svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS(svg.namespaceURI, "path"); path.setAttribute("d", "M5 5h22a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2ZM11 29h10M16 24v5"); svg.append(path); placeholder.prepend(svg);
    image.alt = ""; image.hidden = true; image.loading = "lazy";
    image.addEventListener("load", () => { image.hidden = false; placeholder.hidden = true; });
    image.addEventListener("error", () => { image.hidden = true; placeholder.hidden = false; });
    cover.append(image, placeholder);
    const body = element("div", "card-body"), heading = element("div", "device-heading"), name = element("h2", "device-name"), badge = element("span", "badge"), platform = element("p", "platform"), permissions = element("div", "permissions"), screen = element("span", "permission"), input = element("span", "permission"), actions = element("div", "card-actions"), connect = element("a", "connect", copy.connect), toggle = element("button", "toggle"), request = element("button", "request-permissions"), error = element("p", "device-error");
    toggle.type = request.type = "button"; connect.target = "_blank"; connect.rel = "noopener noreferrer";
    const version = element("p", "runtime-version"), install = element("button", "install-runtime"), operation = element("div", "deployment"), operationLabel = element("span", "deployment-label"), operationValue = element("span", "deployment-value"), operationHeading = element("div", "deployment-heading"), progress = element("progress", "deployment-progress");
    install.type = "button"; progress.max = 100; operationHeading.append(operationLabel, operationValue); operation.append(operationHeading, progress);
    install.addEventListener("click", async () => {
      const { id, installed, enabled } = card.target;
      if (installed && enabled && !(await confirmUpdate())) return;
      void act(id, installed ? enabled ? "update-confirmed" : "update" : "install");
    });
    const permissionStates = element("div", "permission-states");
    permissionStates.append(screen, input); permissions.append(permissionStates, request);
    heading.append(name, badge); actions.append(toggle, install, connect); body.append(heading, platform, version, permissions, operation, error, actions); card.append(cover, body);
    card.parts = { version, install, operation, operationLabel, operationValue, progress, image, placeholder, name, badge, platform, permissions, screen, input, connect, toggle, request, error };
    toggle.addEventListener("click", () => act(card.target.id, card.target.enabled ? "stop" : "start"));
    request.addEventListener("click", () => act(card.target.id, "permissions"));
    grid.append(card); return card;
  }
  function updateCard(card, item) {
    card.target = item;
    const p = card.parts, permissions = item.permissions || {}, deployment = item.deployment || { stage: "idle" };
    const active = !["idle", "completed", "failed"].includes(deployment.stage), switching = active;
    const pending = pendingActions.get(item.id);
    if (pending && (Date.now() > pending.until || pending.action === "start" && item.enabled || pending.action === "stop" && !item.enabled || pending.action === "permissions" && permissions.requesting || ["install", "update", "update-confirmed"].includes(pending.action) && deployment.stage !== "idle")) pendingActions.delete(item.id);
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
    p.version.textContent = `${copy.runtime} · ${item.kind === "instance" ? item.runtime_version || copy.instanceManaged : item.installed ? item.runtime_version || copy.installedUnknown : copy.not_installed}`;
    if ((!item.installed || item.update_available) && item.available_version) p.version.textContent += `  ·  ${copy.latest} ${item.available_version}`;
    p.operation.hidden = !active && deployment.stage !== "failed";
    p.operationLabel.textContent = deployment.stage === "failed" ? copy.failedInstall : copy[deployment.stage] || copy.working;
    const value = Number.isFinite(deployment.progress) ? Math.max(0, Math.min(100, deployment.progress)) : 0;
    p.progress.value = value; p.progress.setAttribute("aria-label", p.operationLabel.textContent);
    p.operationValue.textContent = `${value}%`;
    if (deployment.stage === "downloading" && deployment.total > 0) p.operationValue.textContent += ` · ${(deployment.received / 1048576).toFixed(1)} / ${(deployment.total / 1048576).toFixed(1)} MiB`;
    p.install.hidden = !item.managed || item.installed && !item.update_available && !active && deployment.stage !== "failed";
    p.install.textContent = active ? copy.working : deployment.stage === "failed" ? copy.retryOperation : item.installed ? copy.update : copy.install;
    p.install.classList.toggle("install-primary", !item.installed);
    p.install.disabled = !item.online || !item.can_install || active || processing;
    p.permissions.hidden = false;
    p.toggle.hidden = !item.managed || !item.installed;
    p.toggle.textContent = processing ? copy.working : item.enabled ? copy.stop : copy.start;
    p.toggle.disabled = !item.online || processing || switching;
    p.toggle.ariaLabel = `${item.enabled ? copy.stop : copy.start}: ${item.name}`;
    p.request.hidden = !item.managed || !item.installed || ![permissions.screen, permissions.input].some(value => value === "denied" || value === "needs_user_action");
    p.request.textContent = permissions.requesting ? copy.requesting : copy.permissions;
    p.request.disabled = !item.online || processing || switching || permissions.requesting === true;
    p.connect.hidden = item.kind === "device" && !item.installed;
    const ready = item.online !== false && item.ready !== false && !switching && pending?.action !== "stop";
    p.connect.setAttribute("aria-disabled", String(!ready)); p.connect.tabIndex = ready ? 0 : -1;
    if (ready) p.connect.href = item.url; else p.connect.removeAttribute("href");
    p.connect.ariaLabel = `${copy.connect}: ${item.name}`;
    p.error.textContent = deployment.stage === "failed" ? deployment.error || copy.failedInstall : item.runtime_error === "desktop_process_stopped" ? copy.stopped : item.runtime_error || "";
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
      if (!response.ok) {
        const failure = await response.json().catch(() => ({}));
        throw Error(failure.error === "update_confirmation_required" ? failure.error : "failed");
      }
      warning.hidden = true;
      const reply = await response.json();
      const state = reply?.state;
      controlRevision += 1;
      if (card && state && typeof state.ready === "boolean" && typeof state.enabled === "boolean") {
        updateCard(card, { ...card.target, enabled: state.enabled, ready: state.ready,
          status: state.status, permissions: state.permissions || card.target.permissions,
          deployment: state.deployment || card.target.deployment, runtime_error: state.error });
      }
      await load(true);
    } catch (error) {
      pendingActions.delete(id);
      if (error.message === "update_confirmation_required") {
        await load();
        if (await confirmUpdate()) await act(id, "update-confirmed");
        return;
      }
      warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (card) updateCard(card, card.target);
    }
  }
  async function load(force = false) {
    if (document.hidden) return;
    if (busy) { reloadRequested ||= force; return; }
    busy = true;
    refreshedAt = Date.now();
    const revision = controlRevision;
    if (!cards.size) status.textContent = copy.loading;
    const controller = new AbortController(); pending = controller;
    const timeout = setTimeout(() => controller.abort(), 20000);
    try {
      const response = await fetch("/api/targets", { credentials: "same-origin", cache: "no-store", signal: controller.signal });
      if (response.redirected || response.status === 401 || response.status === 403) throw Error("login_required");
      if (!response.ok) throw Error("failed");
      const data = await response.json();
      if (!Array.isArray(data.targets)) throw Error("failed");
      if (revision !== controlRevision) { reloadRequested = true; return; }
      const targets = data.targets.filter(item => {
        if (!item || typeof item.id !== "string" || typeof item.url !== "string" || typeof item.name !== "string") return false;
        try { const url = new URL(item.url, location.href); return url.protocol === "https:" && !url.username && !url.password || url.origin === location.origin && url.pathname.startsWith("/desktop/"); } catch { return false; }
      });
      render(targets); warning.textContent = copy.unavailable; warning.hidden = !data.unavailable?.length;
    } catch (error) {
      if (document.hidden && controller.signal.aborted) return;
      status.textContent = ""; warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (error.message === "login_required") render([]);
    } finally {
      clearTimeout(timeout); busy = false; pending = undefined;
      if (reloadRequested) { reloadRequested = false; void load(); }
    }
  }
  document.addEventListener("visibilitychange", () => { if (document.hidden) pending?.abort(); else void load(); });
  function refresh() {
    const transitioning = [...pendingActions.values()].some(p => p.action === "start" || p.action === "stop") || [...cards.values()].some(card => card.target.status === "starting");
    const active = [...cards.values()].some(card => !["idle", "completed", "failed"].includes(card.target.deployment?.stage || "idle"));
    if (Date.now() - refreshedAt >= (transitioning ? 250 : active ? 1000 : 4000)) void load();
  }
  let timer = setInterval(refresh, 250);
  window.addEventListener("pagehide", () => { pending?.abort(); clearInterval(timer); timer = undefined; });
  window.addEventListener("pageshow", event => { if (event.persisted) { timer ??= setInterval(refresh, 250); void load(); } });
  void load();
})();
