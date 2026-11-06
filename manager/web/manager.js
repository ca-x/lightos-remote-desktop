(() => {
  "use strict";
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "懒猫远程", subtitle: "懒猫微服网络保障您的隐私和数据安全。", devices: "在线设备", empty: "暂无在线设备", offlineDevices: "离线设备", offlineEmpty: "暂无离线设备",
    help: "在电脑上登录懒猫微服客户端，即可在这里管理。", loading: "正在获取设备…", connect: "连接桌面", consent: "需本机授权",
    failed: "操作失败，请确认设备在线后重试。", login_required: "登录已失效，请重新登录后刷新。", unavailable: "部分桌面来源暂不可用，其他设备仍可使用。",
    count: n => `${n} 台设备`, noPreview: "暂无预览", start: "开启", stop: "关闭", working: "处理中…", currentDevice: "当前设备",
    screen: "屏幕录制", input: "键盘与鼠标", granted: "已授权", denied: "未授权", needs_user_action: "待确认", unknown: "待检查",
    updating: "正在更新", disabled: "已关闭", running: "运行中", starting: "启动中", permission_required: "待授权", error: "启动失败", offline: "离线",
    permissions: "申请系统权限", requesting: "请在设备上确认权限", container: "LightOS实例桌面", footnote: "桌面仅在开启后运行，系统权限由设备本机确认。",
    stopped: "桌面进程已停止，可重新开启。", runtime: "远程桌面", not_installed: "未安装", installedUnknown: "已安装（版本未知）", latest: "最新", install: "安装远程桌面", update: "更新远程桌面", instanceManaged: "由 LightOS 管理",
    client_update_required: "请先更新并重启这台设备上的懒猫 PC 客户端，再安装或更新远程桌面。",
    interruptNotice: "更新会中断当前远程桌面连接。确认后将自动关闭服务并更新，完成后恢复服务。", cancel: "取消", confirm: "确认", stopping: "正在关闭远程桌面",
    preparing: "正在准备", downloading: "正在下载", verifying: "正在校验", installing: "正在安装", checking: "正在检查新版本", cleaning: "正在清理旧版本", failedInstall: "安装 / 更新失败，可重试", retryOperation: "重试",
    usable: "可用", desktopUnavailable: "桌面不可用", details: "查看原因", permissionTitle: "桌面访问检查", diagnosticDetails: "诊断详情", recheck: "刷新状态", rechecking: "正在检查…", checkRequested: "设备状态会根据后台检查结果自动更新。", close: "关闭",
    connectionHelp: "首次使用请先申请权限。", enableConnectionHelp: "可先申请权限，再开启远程桌面。", currentConnectionHelp: "可在此申请权限，授权后从另一台设备连接。",
    authorizeAt: name => `请在「${name}」电脑的系统窗口中允许屏幕与输入访问。当前页面会显示申请结果。`, authorizationSuccess: "已授权。现在可以连接桌面。", authorizationPending: "正在等待被控电脑确认…", authorizationIncomplete: "授权尚未完成。请处理被控电脑上的系统提示，或重新申请。", authorizationNotSaved: "系统没有保存这次授权，连接桌面时仍需确认。", retryAuthorization: "重新申请", authorizationUpdate: "请先更新这台设备的远程桌面，再使用独立授权入口。", authorizationNotStarted: "申请尚未启动，请稍后重试。",
    permissionHelp: { session_unavailable: "无法定位当前登录的图形桌面。请在这台电脑登录桌面，并从该桌面启动客户端后重新检查。", portal_unavailable: "系统没有可用的桌面共享组件。请检查 xdg-desktop-portal 及与当前桌面匹配的后端是否已安装并可运行。", portal_session_mismatch: "暂时无法连接这台电脑的桌面，会自动重新检查。若持续失败，可点击“重新检查”或提供诊断详情。", portal_input_unavailable: "当前桌面共享组件不支持键盘与鼠标控制，需要支持远程控制的桌面后端。", portal_capture_unavailable: "当前桌面共享组件不支持显示器采集，请检查或更新对应的桌面后端。", portal_cursor_unavailable: "当前桌面共享组件缺少所需的光标支持，请检查或更新对应的桌面后端。", portal_timeout: "桌面共享服务没有及时响应。请重新检查，并确认当前图形会话的桌面共享服务正常运行。", capture_unavailable: "当前图形会话没有可用的屏幕与输入接口。请查看诊断详情，检查会话配置和桌面共享组件。" },
  } : {
    title: "Lazycat Remote", subtitle: "The Lazycat network protects your privacy and data security.", devices: "Online devices", empty: "No online devices", offlineDevices: "Offline devices", offlineEmpty: "No offline devices",
    help: "Sign in to the Lazycat client on your computer to manage it here.", loading: "Loading devices…", connect: "Connect", consent: "Local consent required",
    failed: "Operation failed. Check that the device is online and try again.", login_required: "Your session expired. Sign in again.", unavailable: "Some desktop sources are unavailable. Other devices remain available.",
    count: n => `${n} devices`, noPreview: "No preview", start: "Enable", stop: "Disable", working: "Working…", currentDevice: "Current device",
    screen: "Screen", input: "Input", granted: "Granted", denied: "Not granted", needs_user_action: "Needs approval", unknown: "Not checked",
    updating: "Updating", disabled: "Disabled", running: "Running", starting: "Starting", permission_required: "Needs permission", error: "Failed", offline: "Offline",
    permissions: "Request permissions", requesting: "Approve permissions on the device", container: "LightOS instance desktop", footnote: "Desktop capture runs only when enabled. System permissions require approval on the device.",
    stopped: "The desktop process stopped. Try enabling it again.", runtime: "Remote desktop", not_installed: "Not installed", installedUnknown: "Installed (version unknown)", latest: "Latest", install: "Install remote desktop", update: "Update remote desktop", instanceManaged: "Managed by LightOS",
    client_update_required: "Update and restart the Lazycat PC client on this device before installing or updating remote desktop.",
    interruptNotice: "Updating will disconnect the current remote desktop session. Confirm to stop the service and update it. The service will resume when the update finishes.", cancel: "Cancel", confirm: "Confirm", stopping: "Stopping remote desktop",
    preparing: "Preparing", downloading: "Downloading", verifying: "Verifying", installing: "Installing", checking: "Checking the new version", cleaning: "Removing the old version", failedInstall: "Installation / update failed; retry is available", retryOperation: "Retry",
    usable: "Available", desktopUnavailable: "Desktop unavailable", details: "View details", permissionTitle: "Desktop access check", diagnosticDetails: "Diagnostic details", recheck: "Refresh status", rechecking: "Checking…", checkRequested: "Device status updates automatically from background checks.", close: "Close",
    connectionHelp: "Request permissions before first use.", enableConnectionHelp: "You can approve permissions before enabling the desktop.", currentConnectionHelp: "Approve permissions here, then connect from another device.",
    authorizeAt: name => `Allow screen and input access in the system dialog on “${name}”. The result will appear here.`, authorizationSuccess: "Permissions approved. You can now connect to the desktop.", authorizationPending: "Waiting for approval on the controlled computer…", authorizationIncomplete: "Approval is not complete. Respond to the system dialog on the controlled computer or request again.", authorizationNotSaved: "The system did not retain this approval. Confirmation is still needed when connecting.", retryAuthorization: "Request again", authorizationUpdate: "Update this device's desktop runtime before requesting permissions here.", authorizationNotStarted: "The request has not started. Try again shortly.",
    permissionHelp: { session_unavailable: "The logged-in graphical desktop could not be located. Sign in to that desktop, launch the client there and check again.", portal_unavailable: "Desktop sharing services are unavailable. Check that xdg-desktop-portal and the backend for this desktop are installed and running.", portal_session_mismatch: "The system desktop service is temporarily unavailable. Checks retry automatically; you can also select Check again.", portal_input_unavailable: "The sharing service does not support keyboard and mouse control. A remote-control-capable desktop backend is required.", portal_capture_unavailable: "The sharing service does not support monitor capture. Check or update the desktop backend.", portal_cursor_unavailable: "The sharing service lacks the required cursor support. Check or update the desktop backend.", portal_timeout: "The sharing service did not respond in time. Check again and verify that the service is running in the selected graphical session.", capture_unavailable: "No usable screen and input interface was found. Review the diagnostic details and check the graphical session and desktop sharing services." },
  };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = copy.title;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const grid = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty");
  const offlineGrid = document.getElementById("offline-desktops"), offlineStatus = document.getElementById("offline-status"), offlineEmpty = document.getElementById("offline-empty");
  const cards = new Map(), pendingActions = new Map();
  const permissionDialog = document.getElementById("permission-dialog");
  let permissionDevice = null;
  let permissionMode = "diagnostics", authorizationSubmission = null, authorizationNotice = "";
  const linuxDevice = item => item.kind === "device" && item.platform === "linux";
  const connectionConsent = item => linuxDevice(item) && (item.permissions?.backend === "portal" || item.permissions?.request_mode === "on_connection" || item.requires_consent === true) && ![item.permissions?.screen, item.permissions?.input].every(value => value === "granted");
  function permissionHelp(item) {
    if (!linuxDevice(item) || !item.installed) return "";
    if (connectionConsent(item)) return item.current_device ? copy.currentConnectionHelp : item.enabled ? copy.connectionHelp : copy.enableConnectionHelp;
    if (item.permissions?.can_start === false) return copy.permissionHelp[item.permissions.reason_code] || copy.permissionHelp.capture_unavailable;
    return "";
  }
  function updatePermissionDialog(item) {
    if (!permissionDialog.open || item.id !== permissionDevice) return;
    if (permissionMode === "authorization") {
      const p = item.permissions || {}, granted = p.screen === "granted" && p.input === "granted";
      document.getElementById("permission-title").textContent = copy.permissions;
      document.getElementById("permission-summary").textContent = granted ? copy.authorizationSuccess : copy.authorizeAt(item.name);
      document.getElementById("permission-feedback").textContent = granted ? "" : authorizationNotice || (p.requesting || authorizationSubmission?.id === item.id ? copy.authorizationPending : p.reason_code === "authorization_not_saved" ? copy.authorizationNotSaved : copy.authorizationIncomplete);
      const failed = p.reason_code === "authorization_failed" || p.reason_code === "authorization_not_saved" || p.can_start === false;
      document.getElementById("permission-diagnostic").textContent = failed ? p.reason || "" : "";
      document.getElementById("permission-details").hidden = !failed || !p.reason;
      const check = document.getElementById("permission-recheck");
      check.hidden = granted;check.textContent = copy.retryAuthorization;
      check.disabled = !item.online || p.requesting === true || authorizationSubmission?.id === item.id;
      return;
    }
    document.getElementById("permission-title").textContent = copy.permissionTitle;
    document.getElementById("permission-summary").textContent = permissionHelp(item) || copy.usable;
    document.getElementById("permission-diagnostic").textContent = item.permissions?.reason || "";
    document.getElementById("permission-details").hidden = !item.permissions?.reason;
    const check = document.getElementById("permission-recheck");
    check.hidden = false;
    check.disabled = !item.online || item.permissions?.requesting === true || pendingActions.has(item.id);
    check.textContent = item.permissions?.requesting ? copy.rechecking : copy.recheck;
  }
  function showPermissionDetails(item, authorize = false) {
    if (permissionDialog.open) return;
    permissionDevice = item.id;
    permissionMode = authorize ? "authorization" : "diagnostics";authorizationNotice = "";
    document.getElementById("permission-feedback").textContent = "";
    permissionDialog.showModal();
    updatePermissionDialog(item);
    if (authorize) void requestPermissions(item.id);
  }
  async function requestPermissions(id) {
    const item = cards.get(id)?.target;
    if (!item || authorizationSubmission?.id === id) return;
    if (item.permissions?.request_mode !== "system") { authorizationNotice = copy.authorizationUpdate;updatePermissionDialog(item);return; }
    const submission = { id };authorizationSubmission = submission;authorizationNotice = "";updatePermissionDialog(item);
    try {
      // The existing controller may already be running a read-only probe.
      // Retry only when its reply explicitly says a request has not started.
      for (let attempt = 0; attempt < 10 && permissionDevice === id && authorizationSubmission === submission; attempt++) {
        const reply = await act(id, "permissions");
        if (permissionDevice !== id || authorizationSubmission !== submission) break;
        if (!reply) { authorizationNotice = copy.failed;break; }
        const p = reply.state?.permissions;
        if (!p || p.requesting || p.screen === "granted" && p.input === "granted") break;
        if (attempt === 9) authorizationNotice = copy.authorizationNotStarted;
        else await new Promise(resolve => setTimeout(resolve, 400));
      }
    } finally {
      if (authorizationSubmission === submission) authorizationSubmission = null;
      const current = cards.get(id)?.target;if (current) updatePermissionDialog(current);
    }
  }
  permissionDialog.addEventListener("close", () => { permissionDevice = null; });
  document.getElementById("permission-recheck").addEventListener("click", () => {
    if (!permissionDevice) return;
    if (permissionMode === "authorization") { void requestPermissions(permissionDevice);return; }
    document.getElementById("permission-feedback").textContent = copy.checkRequested;
    void load(true);
  });
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
    const guidance = element("p", "permission-help");guidance.hidden = true;
    const runtimeHelp = element("p", "runtime-help", copy.client_update_required);runtimeHelp.hidden = true;
    runtimeHelp.id = `runtime-help-${item.id}`;
    heading.append(name, badge); actions.append(toggle, install, connect); body.append(heading, platform, version, permissions, guidance, operation, error, runtimeHelp, actions); card.append(cover, body);
    card.parts = { version, install, operation, operationLabel, operationValue, progress, image, placeholder, name, badge, platform, permissions, screen, input, connect, toggle, request, error, guidance, runtimeHelp };
    toggle.addEventListener("click", () => act(card.target.id, card.target.enabled ? "stop" : "start"));
    request.addEventListener("click", () => linuxDevice(card.target) ? showPermissionDetails(card.target, connectionConsent(card.target)) : act(card.target.id, "permissions"));
    grid.append(card); return card;
  }
  function updateCard(card, item) {
    card.target = item;
    const p = card.parts, permissions = item.permissions || {}, deployment = item.deployment || { stage: "idle" };
    const active = !["idle", "completed", "failed"].includes(deployment.stage), switching = active;
    const pending = pendingActions.get(item.id);
    if (pending && (Date.now() > pending.until || pending.action === "start" && item.enabled || pending.action === "stop" && !item.enabled || pending.action === "permissions" && permissions.requesting || ["install", "update", "update-confirmed"].includes(pending.action) && deployment.stage !== "idle")) pendingActions.delete(item.id);
    const processing = pendingActions.has(item.id);
    const linux = linuxDevice(item), deferred = connectionConsent(item), unavailable = linux && permissions.can_start === false;
    p.name.textContent = item.name; p.name.title = item.name;
    p.platform.textContent = item.kind === "instance" ? copy.container : platforms[item.platform] || item.platform || "";
    const state = item.online === false ? "offline" : item.status || (item.ready ? "running" : "disabled");
    p.badge.textContent = copy[state] || copy.disabled; p.badge.dataset.state = state;
    if (unavailable && state === "permission_required") p.badge.textContent = copy.desktopUnavailable;
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
      const label = linux && value === "granted" && permissions.backend !== "portal" ? copy.usable : deferred && value === "needs_user_action" ? copy.needs_user_action : unavailable && value === "denied" ? copy.desktopUnavailable : copy[value];
      node.textContent = `${copy[kind]} · ${label}`; node.dataset.granted = String(value === "granted");
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
    const clientUpdateRequired = item.kind === "device" && item.online && !p.install.hidden
      && !item.can_install && !(item.controller_protocol >= 2) && !active && !processing;
    p.runtimeHelp.hidden = !clientUpdateRequired;
    p.install.title = clientUpdateRequired ? copy.client_update_required : "";
    if (clientUpdateRequired) p.install.setAttribute("aria-describedby", p.runtimeHelp.id);
    else p.install.removeAttribute("aria-describedby");
    p.permissions.hidden = false;
    p.toggle.hidden = !item.managed || !item.installed;
    p.toggle.textContent = processing ? copy.working : item.enabled ? copy.stop : copy.start;
    p.toggle.disabled = !item.online || processing || switching;
    p.toggle.ariaLabel = `${item.enabled ? copy.stop : copy.start}: ${item.name}`;
    p.request.hidden = !item.managed || !item.installed || ![permissions.screen, permissions.input].some(value => value === "denied" || value === "needs_user_action");
    p.request.textContent = permissions.requesting ? copy.requesting : copy.permissions;
    p.request.disabled = !item.online || processing || switching || permissions.requesting === true;
    if (linux) {
      p.request.hidden = !item.managed || !item.installed || !(unavailable || deferred);
      p.request.textContent = permissions.requesting ? copy.requesting : deferred ? copy.permissions : copy.details;
    }
    p.guidance.textContent = permissionHelp(item);p.guidance.hidden = !p.guidance.textContent;
    p.connect.hidden = item.kind === "device" && !item.installed && !item.current_device;
    p.connect.textContent = item.current_device ? copy.currentDevice : copy.connect;
    const ready = !item.current_device && item.online !== false && item.ready !== false && !switching && pending?.action !== "stop" && !(deferred && permissions.requesting);
    p.connect.setAttribute("aria-disabled", String(!ready)); p.connect.tabIndex = ready ? 0 : -1;
    if (ready) p.connect.href = item.url; else p.connect.removeAttribute("href");
    p.connect.ariaLabel = item.current_device ? copy.currentDevice : `${copy.connect}: ${item.name}`;
    p.error.textContent = deployment.stage === "failed" ? deployment.error || copy.failedInstall : item.runtime_error === "desktop_process_stopped" ? copy.stopped : item.runtime_error || "";
    p.error.hidden = !p.error.textContent;
    updatePermissionDialog(item);
  }
  function render(targets) {
    const ids = new Set(targets.map(item => item.id));
    if (permissionDevice && !ids.has(permissionDevice)) permissionDialog.close();
    for (const [id, card] of cards) if (!ids.has(id)) { card.remove(); cards.delete(id); }
    for (const item of targets) {
      let card = cards.get(item.id);
      if (!card) { card = createCard(item); cards.set(item.id, card); }
      updateCard(card, item);
      const destination = item.online === false ? offlineGrid : grid;
      if (card.parentElement !== destination) destination.appendChild(card);
    }
    const offline = targets.filter(item => item.online === false).length, online = targets.length - offline;
    empty.hidden = online > 0; status.textContent = copy.count(online);
    offlineStatus.textContent = copy.count(offline); offlineEmpty.hidden = offline > 0;
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
        throw Error(["update_confirmation_required", "client_update_required"].includes(failure.error) ? failure.error : "failed");
      }
      warning.hidden = true;
      const reply = await response.json();
      if (action === "permissions") pendingActions.delete(id);
      const state = reply?.state;
      controlRevision += 1;
      if (card && state && typeof state.ready === "boolean" && typeof state.enabled === "boolean") {
        updateCard(card, { ...card.target, enabled: state.enabled, ready: state.ready,
          status: state.status, permissions: state.permissions || card.target.permissions,
          deployment: state.deployment || card.target.deployment, runtime_error: state.error });
      }
      await load(true);
      return reply;
    } catch (error) {
      pendingActions.delete(id);
      if (error.message === "update_confirmation_required") {
        await load();
        if (await confirmUpdate()) await act(id, "update-confirmed");
        return;
      }
      warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (action === "permissions" && permissionDevice === id) document.getElementById("permission-feedback").textContent = warning.textContent;
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
    const requesting = [...cards.values()].some(card => card.target.permissions?.requesting);
    if (Date.now() - refreshedAt >= (transitioning ? 250 : active || requesting ? 1000 : 4000)) void load();
  }
  let timer = setInterval(refresh, 250);
  window.addEventListener("pagehide", () => { pending?.abort(); clearInterval(timer); timer = undefined; });
  window.addEventListener("pageshow", event => { if (event.persisted) { timer ??= setInterval(refresh, 250); void load(); } });
  void load();
})();
