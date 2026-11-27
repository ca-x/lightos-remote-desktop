(() => {
  "use strict";
  const platform = navigator.userAgentData?.platform || navigator.platform || "";
  const mobile = /\b(Android|iPhone|iPad|iPod)\b/i.test(`${platform} ${navigator.userAgent}`)
    || (/\bMac/i.test(platform) && navigator.maxTouchPoints > 1);
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "懒猫远程", subtitle: "懒猫微服网络保障您的隐私和数据安全。", devices: "在线设备", empty: "暂无在线设备",
    help: "在电脑上登录懒猫微服客户端，即可在这里管理。", loading: "正在获取设备…", connect: "连接桌面", consent: "需本机授权",
    failed: "操作未完成，请刷新设备状态后重试。", login_required: "登录已失效，请重新登录后刷新。", unavailable: "部分桌面来源暂不可用，其他设备仍可使用。",
    device_unavailable: "暂时无法确认设备可用，请刷新设备列表后重试。",
    device_authorization_failed: "微服暂时无法获取设备控制授权，请稍后重试。",
    control_unavailable: "此设备客户端网络状态异常",
    control_rejected: "设备控制通道未接受这次操作，请刷新设备状态后重试。",
    invalid_control_response: "无法确认设备返回的操作结果，请先刷新设备状态。",
    invalid_control_origin: "当前页面无法发起设备操作，请重新打开懒猫远程管理页。",
    control_timeout: "等待操作结果超时，请刷新设备状态，确认操作是否已开始。",
    desktop_control_busy: "设备正在处理远程桌面启停，请稍后重试。",
    desktop_preference_save_failed: "微服未能保存这次启停设置，请重试。",
    count: n => `${n} 台设备`, noPreview: "暂无预览", menu: "设备菜单", start: "开启远程桌面", stop: "关闭远程桌面", uninstall: "卸载远程桌面", removing: "正在卸载远程桌面", working: "处理中…", currentDevice: "当前设备",
    powerEnable: "申请禁用休眠", powerRestore: "关闭防止休眠", powerChecking: "正在读取休眠状态…", powerPending: "请在被控设备上确认系统授权", power_unknown: "暂时无法读取休眠状态",
    power_update_required: "更新远程桌面后可管理休眠", power_busy: "设备正在处理其他请求，请稍后重试。", power_not_applied: "休眠设置未完成，请检查被控设备上的系统授权或电源策略。",
    screen: "屏幕录制", input: "键盘与鼠标", granted: "已授权", denied: "未授权", needs_user_action: "待确认", unknown: "待检查",
    updating: "正在更新", disabled: "已关闭", running: "运行中", starting: "启动中", locked: "已锁屏", temporarily_unavailable: "暂不可连接", permission_required: "待授权", error: "启动失败", offline: "离线",
    permissions: "请求设备权限", requesting: "等待设备确认", permissionApproved: "权限已批准", permissionUnapproved: "权限未批准", unlockedOnly: "仅在桌面解锁时可访问", container: "LightOS实例", instanceManagedFallback: "版本和状态由lightos管理", footnote: "开启后允许连接。进入桌面时启动服务，最后一个连接断开后停止服务。",
    enterDesktop: "点击进入桌面", enterLocked: "点击进入锁屏界面",
    lockAccessHelp: "如需在锁屏时访问，请点击“请求设备权限”，并在这台 Windows 电脑上确认。",
    lockAccessPending: "请在被控 Windows 电脑上查看“懒猫远程 · 锁屏访问”确认窗口。",
    lockAccessDeferred: "设备尚未收到锁屏授权请求。请确认懒猫客户端在线后重试。",
    lockRequestResults: { install_failed: "锁屏辅助服务安装失败，请重新请求设备权限。", service_not_ready: "安装程序已退出，尚未确认锁屏辅助服务正在运行。可重新请求设备权限。", declined: "已在被控电脑上取消锁屏授权。", unlock_required: "请先在被控电脑上解锁桌面，再请求权限。", failed: "未能完成锁屏授权。请检查被控电脑上的管理员确认后重试。", submitted: "已提交安装请求。若仍只能访问解锁的桌面，请在被控电脑上检查安装结果。" },
    installToEnter: "安装后进入桌面", authorizeToEnter: "授权后进入桌面", startingDesktop: "桌面启动中…",
    stopped: "桌面进程已退出，请重新进入桌面。", runtime: "懒猫远程", not_installed: "未安装", installedUnknown: "已安装（版本未知）", latest: "最新", install: "安装远程桌面", update: "更新远程桌面", instanceManaged: "由 LightOS 管理",
    client_update_required: "请先更新并重启这台设备上的懒猫 PC 客户端，再安装或更新远程桌面。",
    interruptNotice: "更新会中断当前远程桌面连接。确认后将自动关闭服务并更新，完成后恢复服务。", cancel: "取消", confirm: "确认", stopping: "正在关闭远程桌面",
    stop_pending: "关闭待确认", stopRetrying: "设备尚未确认关闭。设备在线时会自动重试。",
    preparing: "正在准备", downloading: "正在下载", verifying: "正在校验", installing: "正在安装", checking: "正在检查新版本", cleaning: "正在清理旧版本", failedInstall: "安装 / 更新失败，可重试", retryRemoval: "重试卸载", retryOperation: "重试",
    virtualAvailable: "可连接独立虚拟桌面；物理桌面仍需解锁或授权，不会自动切换。", physicalLocked: "物理桌面已锁定，请在被控电脑解锁后重新连接。",
    usable: "可用", details: "查看原因", permissionTitle: "桌面访问检查", diagnosticDetails: "诊断详情", recheck: "刷新状态", rechecking: "正在检查…", checkRequested: "设备状态会根据后台检查结果自动更新。", close: "关闭",
    connectionHelp: "首次使用请先申请权限。", enableConnectionHelp: "可先申请权限，再开启远程桌面。", currentConnectionHelp: "可在此申请权限，授权后从另一台设备连接。",
    authorizeAt: name => `请在「${name}」电脑的系统窗口中允许屏幕与输入访问。当前页面会显示申请结果。`, authorizationSuccess: "已授权。现在可以连接桌面。", authorizationPending: "正在等待被控电脑确认…", authorizationIncomplete: "授权尚未完成。请处理被控电脑上的系统提示，或重新申请。", authorizationNotSaved: "系统没有保存这次授权，连接桌面时仍需确认。", retryAuthorization: "重新申请", authorizationUpdate: "请先更新这台设备的远程桌面，再使用独立授权入口。", authorizationNotStarted: "申请尚未启动，请稍后重试。",
    permissionHelp: { session_unavailable: "无法定位当前登录的图形桌面。请在这台电脑登录桌面，并从该桌面启动客户端后重新检查。", portal_unavailable: "系统没有可用的桌面共享组件。请检查 xdg-desktop-portal 及与当前桌面匹配的后端是否已安装并可运行。", portal_session_mismatch: "暂时无法连接这台电脑的桌面，会自动重新检查。若持续失败，可点击“重新检查”或提供诊断详情。", portal_input_unavailable: "当前桌面共享组件不支持键盘与鼠标控制，需要支持远程控制的桌面后端。", portal_capture_unavailable: "当前桌面共享组件不支持显示器采集，请检查或更新对应的桌面后端。", portal_cursor_unavailable: "当前桌面共享组件缺少所需的光标支持，请检查或更新对应的桌面后端。", portal_timeout: "桌面共享服务没有及时响应。请重新检查，并确认当前图形会话的桌面共享服务正常运行。", capture_unavailable: "当前图形会话没有可用的屏幕与输入接口。请查看诊断详情，检查会话配置和桌面共享组件。" },
  } : {
    title: "Lazycat Remote", subtitle: "The Lazycat network protects your privacy and data security.", devices: "Online devices", empty: "No online devices",
    help: "Sign in to the Lazycat client on your computer to manage it here.", loading: "Loading devices…", connect: "Connect", consent: "Local consent required",
    failed: "The operation could not be completed. Refresh the device status and try again.", login_required: "Your session expired. Sign in again.", unavailable: "Some desktop sources are unavailable. Other devices remain available.",
    device_unavailable: "Device availability could not be confirmed. Refresh the device list and try again.",
    device_authorization_failed: "The microserver could not get permission to control this device. Try again shortly.",
    control_unavailable: "This device's client is experiencing network issues.",
    control_rejected: "The device control channel did not accept the request. Refresh the device status and try again.",
    invalid_control_response: "The device's response could not be confirmed. Refresh the device status first.",
    invalid_control_origin: "This page cannot send device commands. Reopen the Lazycat Remote management page.",
    control_timeout: "The operation timed out. Refresh the device status to check whether it started.",
    desktop_control_busy: "This device is applying a remote desktop command. Try again shortly.",
    desktop_preference_save_failed: "The microserver could not save this remote desktop setting. Try again.",
    count: n => `${n} devices`, noPreview: "No preview", menu: "Device actions", start: "Enable remote desktop", stop: "Disable remote desktop", uninstall: "Uninstall remote desktop", removing: "Uninstalling remote desktop", working: "Working…", currentDevice: "Current device",
    powerEnable: "Request sleep prevention", powerRestore: "Turn off sleep prevention", powerChecking: "Checking sleep settings…", powerPending: "Confirm the system prompt on the device", power_unknown: "Sleep settings unavailable",
    power_update_required: "Update remote desktop to manage sleep", power_busy: "Another request is in progress on this device. Try again shortly.", power_not_applied: "The sleep setting was not applied. Check approval and power policy on the device.",
    screen: "Screen", input: "Input", granted: "Granted", denied: "Not granted", needs_user_action: "Needs approval", unknown: "Not checked",
    updating: "Updating", disabled: "Disabled", running: "Running", starting: "Starting", locked: "Locked", temporarily_unavailable: "Temporarily unavailable", permission_required: "Needs permission", error: "Failed", offline: "Offline",
    permissions: "Request access", requesting: "Waiting for approval", permissionApproved: "Access approved", permissionUnapproved: "Access not approved", unlockedOnly: "Unlocked desktop only", container: "LightOS instance", instanceManagedFallback: "Version and status managed by LightOS", footnote: "Enable a device to allow connections. Opening its desktop starts the service. The service stops after the last connection closes.",
    enterDesktop: "Open desktop", enterLocked: "Open lock screen",
    lockAccessHelp: "To enable access while locked, select Request access and confirm on the Windows computer.",
    lockAccessPending: "Check the Windows computer for the Lazycat Remote lock-screen access dialog.",
    lockAccessDeferred: "The administrator request was not confirmed. Try again and check the controlled Windows computer's own screen.",
    lockRequestResults: { install_failed: "The lock-screen service installer failed. Request access to try again.", service_not_ready: "The installer finished, but the lock-screen service is not ready. Request access to retry.", declined: "Lock-screen access was declined on the device.", unlock_required: "Unlock the Windows desktop, then request access again.", failed: "Lock-screen setup did not finish. Check administrator approval on the device and try again.", submitted: "Installation was requested. If access remains limited to the unlocked desktop, check setup on the Windows computer." },
    installToEnter: "Install to connect", authorizeToEnter: "Approve access to connect", startingDesktop: "Desktop is starting…",
    stopped: "The desktop process exited. Open the desktop again.", runtime: "Lazycat Remote", not_installed: "Not installed", installedUnknown: "Installed (version unknown)", latest: "Latest", install: "Install remote desktop", update: "Update remote desktop", instanceManaged: "Managed by LightOS",
    client_update_required: "Update and restart the Lazycat PC client on this device before installing or updating remote desktop.",
    interruptNotice: "Updating will disconnect the current remote desktop session. Confirm to stop the service and update it. The service will resume when the update finishes.", cancel: "Cancel", confirm: "Confirm", stopping: "Stopping remote desktop",
    stop_pending: "Disable pending", stopRetrying: "The device has not confirmed shutdown. It will retry automatically while the device is online.",
    preparing: "Preparing", downloading: "Downloading", verifying: "Verifying", installing: "Installing", checking: "Checking the new version", cleaning: "Removing the old version", failedInstall: "Installation / update failed; retry is available", retryRemoval: "Retry uninstall", retryOperation: "Retry",
    virtualAvailable: "A separate virtual desktop is available. Physical access still requires unlocking or consent; switching is never automatic.", physicalLocked: "The physical desktop is locked. Unlock the controlled computer before reconnecting.",
    usable: "Available", details: "View details", permissionTitle: "Desktop access check", diagnosticDetails: "Diagnostic details", recheck: "Refresh status", rechecking: "Checking…", checkRequested: "Device status updates automatically from background checks.", close: "Close",
    connectionHelp: "Request permissions before first use.", enableConnectionHelp: "You can approve permissions before enabling the desktop.", currentConnectionHelp: "Approve permissions here, then connect from another device.",
    authorizeAt: name => `Allow screen and input access in the system dialog on “${name}”. The result will appear here.`, authorizationSuccess: "Permissions approved. You can now connect to the desktop.", authorizationPending: "Waiting for approval on the controlled computer…", authorizationIncomplete: "Approval is not complete. Respond to the system dialog on the controlled computer or request again.", authorizationNotSaved: "The system did not retain this approval. Confirmation is still needed when connecting.", retryAuthorization: "Request again", authorizationUpdate: "Update this device's desktop runtime before requesting permissions here.", authorizationNotStarted: "The request has not started. Try again shortly.",
    permissionHelp: { session_unavailable: "The logged-in graphical desktop could not be located. Sign in to that desktop, launch the client there and check again.", portal_unavailable: "Desktop sharing services are unavailable. Check that xdg-desktop-portal and the backend for this desktop are installed and running.", portal_session_mismatch: "The system desktop service is temporarily unavailable. Checks retry automatically; you can also select Check again.", portal_input_unavailable: "The sharing service does not support keyboard and mouse control. A remote-control-capable desktop backend is required.", portal_capture_unavailable: "The sharing service does not support monitor capture. Check or update the desktop backend.", portal_cursor_unavailable: "The sharing service lacks the required cursor support. Check or update the desktop backend.", portal_timeout: "The sharing service did not respond in time. Check again and verify that the service is running in the selected graphical session.", capture_unavailable: "No usable screen and input interface was found. Review the diagnostic details and check the graphical session and desktop sharing services." },
  };
  const controlFailures = new Set([
    "update_confirmation_required", "client_update_required", "device_unavailable",
    "device_authorization_failed", "control_unavailable", "control_rejected",
    "invalid_control_response", "invalid_control_origin", "desktop_control_busy", "desktop_preference_save_failed", "power_busy",
  ]);
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = copy.title;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const grid = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty");
  const cards = new Map(), pendingActions = new Map(), pendingPermissions = new Set();
  let openMenu = null;
  function closeMenu(restoreFocus = false) {
    if (!openMenu) return;
    const { menu, trigger, read, timer } = openMenu;
    read?.abort();
    clearTimeout(timer);
    menu.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
    if (restoreFocus) trigger.focus();
    openMenu = null;
  }
  function renderPower(scope) {
    const { card, power } = scope;
    power.disabled = !card.target.power_supported || card.parts.toggle.disabled || Boolean(scope.read) || Boolean(card.powerChanging) || typeof scope.enabled !== "boolean";
    power.textContent = !card.target.power_supported ? copy.power_update_required : card.powerChanging ? copy.powerPending
      : scope.enabled === true ? copy.powerRestore : scope.enabled === false ? copy.powerEnable
      : scope.read ? copy.powerChecking : copy.power_unknown;
  }
  async function requestPower(id, action, signal) {
    const response = await fetch(`/api/hosts/${encodeURIComponent(id)}/power-${action}`, {
      method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: "{}", signal,
    });
    if (response.redirected || response.status === 401) throw Error("login_required");
    const body = await response.json();
    if (!response.ok || typeof body.enabled !== "boolean") throw Error(body.error || "power_unknown");
    return body.enabled;
  }
  async function readPower(scope) {
    if (openMenu !== scope || document.hidden || scope.read || scope.card.powerChanging) {
      if (openMenu === scope) renderPower(scope);
      return;
    }
    clearTimeout(scope.timer);
    let nextCheck = 10000;
    if (scope.card.target.power_supported && !scope.card.parts.toggle.disabled) {
      const controller = new AbortController();
      scope.read = controller;
      renderPower(scope);
      try {
        const enabled = await requestPower(scope.card.target.id, "read", controller.signal);
        if (!controller.signal.aborted) scope.enabled = enabled;
      } catch (error) {
        if (!controller.signal.aborted) scope.enabled = undefined;
        if (error.message === "power_busy") nextCheck = 3000;
      } finally {
        if (scope.read === controller) scope.read = null;
      }
    } else { scope.enabled = undefined; }
    if (openMenu !== scope || document.hidden || scope.card.powerChanging) return;
    renderPower(scope);
    scope.timer = setTimeout(() => void readPower(scope), nextCheck);
  }
  async function changePower(card) {
    const scope = openMenu;
    if (scope?.card !== card || scope.power.disabled) return;
    scope.read?.abort();
    clearTimeout(scope.timer);
    const action = scope.enabled ? "restore" : "enable";
    scope.enabled = undefined;
    card.powerChanging = true;
    card.actionError = "";
    updateCard(card, card.target);
    let confirmed;
    try {
      confirmed = await requestPower(card.target.id, action, AbortSignal.timeout(115000));
    } catch (error) {
      card.actionError = copy[error.message] || copy.power_not_applied;
    } finally {
      card.powerChanging = false;
      if (card.isConnected) updateCard(card, card.target);
      const current = openMenu;
      if (current?.card === card && !document.hidden) {
        if (typeof confirmed === "boolean") {
          current.enabled = confirmed;
          renderPower(current);
          current.timer = setTimeout(() => void readPower(current), 10000);
        } else { void readPower(current); }
      }
    }
  }
  document.addEventListener("click", event => {
    if (openMenu && !openMenu.actions.contains(event.target)) closeMenu();
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") closeMenu(true);
  });
  document.addEventListener("focusin", event => {
    if (openMenu && !openMenu.actions.contains(event.target)) closeMenu();
  });
  const permissionDialog = document.getElementById("permission-dialog");
  let permissionDevice = null;
  let permissionMode = "diagnostics", authorizationSubmission = null, authorizationNotice = "";
  const linuxDevice = item => item.kind === "device" && item.platform === "linux";
  const macDevice = item => item?.kind === "device" && ["darwin", "macos"].includes(item.platform);
  function macPermissionsHelp(item) {
    const permissions = item.permissions || {};
    const settings = permissions.settings || [], missing = [];
    if (permissions.screen !== "granted") {
      missing.push(zh ? "录屏与系统录音（或屏幕录制）" : "Screen & System Audio Recording (or Screen Recording)");
    }
    if (permissions.input !== "granted") {
      if (settings.includes("system_app_protection_override")) {
        missing.push(zh ? "系统与 App 保护覆盖" : "System & App Protection Override");
      } else if (settings.includes("device_control_data_access")) {
        missing.push(zh ? "设备控制和数据访问" : "Device Control and Data Access");
      } else if (settings.includes("accessibility")) {
        missing.push(zh ? "辅助功能（或无障碍）" : "Accessibility");
      } else {
        missing.push(zh ? "键鼠控制（辅助功能或设备控制和数据访问）" : "keyboard and mouse control (Accessibility or Device Control and Data Access)");
      }
    }
    const appName = item.controller_protocol >= 2
      ? (zh ? "懒猫微服客户端（LCMD）" : "the Lazycat PC client (LCMD)")
      : copy.runtime;
    return zh
      ? `在被控 Mac 的「隐私与安全性」中，为${appName}允许${missing.map(name => `「${name}」`).join("和")}。`
      : `On the controlled Mac, allow ${missing.join(" and ")} for ${appName} in Privacy & Security.`;
  }
  const connectionConsent = item => linuxDevice(item) && (item.permissions?.backend === "portal" || item.permissions?.request_mode === "on_connection" || item.requires_consent === true) && ![item.permissions?.screen, item.permissions?.input].every(value => value === "granted");
  function permissionHelp(item) {
    if (!linuxDevice(item) || !item.installed) return "";
    if (item.permissions?.virtual_available === true) return copy.virtualAvailable;
    if (item.permissions?.reason_code === "session_locked") return copy.physicalLocked;
    if (connectionConsent(item)) return item.current_device ? copy.currentConnectionHelp : item.access_enabled ? copy.connectionHelp : copy.enableConnectionHelp;
    if (item.permissions?.can_start === false) return copy.permissionHelp[item.permissions.reason_code] || copy.permissionHelp.capture_unavailable;
    return "";
  }
  function updatePermissionDialog(item) {
    if (!permissionDialog.open || item.id !== permissionDevice) return;
    if (permissionMode === "authorization") {
      const p = item.permissions || {}, granted = p.screen === "granted" && p.input === "granted";
      document.getElementById("permission-title").textContent = copy.permissions;
      document.getElementById("permission-summary").textContent = granted ? copy.authorizationSuccess
        : macDevice(item) ? macPermissionsHelp(item) : copy.authorizeAt(item.name);
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
        // The macOS controller acknowledges the request immediately, while
        // the runtime handles consent and the read-only polling reports it.
        if (macDevice(item) || !p || p.requesting || p.screen === "granted" && p.input === "granted") break;
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
  const platformNames = { apple: "macOS", windows: "Windows", linux: "Linux", omarchy: "Omarchy", debian: "Debian", arch: "Arch Linux", ubuntu: "Ubuntu" };
  function platformIcon(item) {
    const platform = String(item.platform || "").trim().toLowerCase();
    if (["darwin", "macos", "mac", "osx"].includes(platform)) return "apple";
    if (["win32", "windows", "win64"].includes(platform)) return "windows";
    const identity = [platform, item.distro, item.os_release?.id, item.os?.id].filter(value => typeof value === "string").join(" ").toLowerCase();
    if (/\bomarchy\b/.test(identity)) return "omarchy";
    if (/\bubuntu\b/.test(identity)) return "ubuntu";
    if (/\bdebian\b/.test(identity)) return "debian";
    if (/\barch(?:\s*linux)?\b/.test(identity)) return "arch";
    return "linux";
  }
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

    const connect = element("a", "preview-link"), connectLabel = element("span", "preview-action", copy.enterDesktop);
    if (!mobile) { connect.target = "_blank"; connect.rel = "noopener noreferrer"; }
    connect.append(connectLabel);
    connect.addEventListener("click", event => { if (connect.getAttribute("aria-disabled") === "true") event.preventDefault(); });
    const notices = element("div", "preview-notices"), badge = element("span", "badge"), error = element("p", "device-error");
    error.id = `device-error-${item.id}`; error.hidden = true; error.tabIndex = 0; error.setAttribute("role", "status");
    notices.append(badge, error);
    const operation = element("div", "deployment"), operationLabel = element("span", "deployment-label"), operationValue = element("span", "deployment-value"), operationHeading = element("div", "deployment-heading"), progress = element("progress", "deployment-progress");
    operation.hidden = true; progress.max = 100; operationHeading.append(operationLabel, operationValue); operation.append(operationHeading, progress);
    cover.append(image, placeholder, connect, notices, operation);

    const body = element("div", "card-body"), heading = element("div", "device-heading"), identity = element("div", "device-identity"), icon = element("img", "platform-icon"), name = element("h2", "device-name"), actions = element("div", "device-actions");
    const trigger = element("button", "menu-trigger"), menu = element("div", "device-menu");
    const toggle = element("button"), power = element("button"), uninstall = element("button");
    menu.hidden = true;
    menu.id = `device-menu-${item.id}`;
    trigger.type = power.type = uninstall.type = "button";
    trigger.textContent = "⋯";
    trigger.setAttribute("aria-expanded", "false");
    trigger.setAttribute("aria-controls", menu.id);
    uninstall.textContent = copy.uninstall;
    menu.append(toggle, power, uninstall);
    actions.append(trigger, menu);
    icon.alt = ""; identity.append(icon, name);
    const platformRow = element("div", "platform-row"), platformInfo = element("div", "platform-info"), platform = element("p", "platform"), permissionSummary = element("span", "permission-summary"), request = element("button", "text-action request-permissions");
    const versionRow = element("div", "version-row"), version = element("p", "runtime-version"), install = element("button", "text-action install-runtime");
    toggle.type = request.type = install.type = "button";
    heading.append(identity, actions); platformInfo.append(platform, permissionSummary); platformRow.append(platformInfo, request); versionRow.append(version, install);
    body.append(heading, platformRow, versionRow); card.append(cover, body);
    card.parts = { version, install, operation, operationLabel, operationValue, progress, image, placeholder, name, icon, badge, platform, permissionSummary, connect, connectLabel, actions, trigger, toggle, uninstall, request, error };
    install.addEventListener("click", async () => {
      const { id, installed, enabled } = card.target;
      if (card.target.removing && card.target.deployment?.stage === "failed") {
        void act(id, "uninstall");
        return;
      }
      if (installed && enabled && !(await confirmUpdate())) return;
      void act(id, installed ? enabled ? "update-confirmed" : "update" : "install");
    });
    trigger.addEventListener("click", () => {
      const opening = menu.hidden;
      closeMenu();
      if (opening) {
        openMenu = { actions, menu, trigger, card, power };
        menu.hidden = false;
        trigger.setAttribute("aria-expanded", "true");
        toggle.focus();
        void readPower(openMenu);
      }
    });
    toggle.addEventListener("click", () => { closeMenu(true); void act(card.target.id, toggleAction(card.target)); });
    power.addEventListener("click", () => void changePower(card));
    uninstall.addEventListener("click", () => { closeMenu(true); void act(card.target.id, "uninstall"); });
    request.addEventListener("click", () => linuxDevice(card.target) ? showPermissionDetails(card.target, card.target.permissions?.request_mode === "system" || connectionConsent(card.target)) : act(card.target.id, "permissions"));
    grid.append(card); return card;
  }
  const toggleAction = item => item.access_enabled ? "stop" : "start";
  function updateCard(card, item) {
    card.target = item;
    const p = card.parts, permissions = item.permissions || {}, deployment = item.deployment || { stage: "idle" };
    const active = !["idle", "completed", "failed"].includes(deployment.stage || "idle"), switching = active;
    const stopping = item.status === "stopping", stopPending = item.status === "stop_pending";
    let pending = pendingActions.get(item.id);
    if (pending && (Date.now() > pending.until || pending.action === "start" && item.access_enabled || pending.action === "stop" && !item.access_enabled)) {
      pendingActions.delete(item.id); pending = undefined;
    }
    const processing = Boolean(pending);
    const linux = linuxDevice(item), deferred = connectionConsent(item), unavailable = linux && item.installed && permissions.can_start === false;
    const physicalUnavailable = linux && permissions.reason_code === "session_locked";
    p.name.textContent = item.name; p.name.title = item.name;
    const icon = platformIcon(item);
    p.icon.dataset.platform = icon; p.icon.alt = platformNames[icon]; p.icon.title = platformNames[icon];
    const iconSource = `./platforms/${icon}.svg`;
    if (p.icon.getAttribute("src") !== iconSource) p.icon.src = iconSource;
    p.platform.textContent = item.kind === "instance" ? copy.container : platforms[item.platform] || item.platform || "";
    const mac = macDevice(item);
    const permissionApproved = permissions.screen === "granted" && permissions.input === "granted";
    const windowsWithoutHelper = icon === "windows" && item.installed && permissions.lock_screen_authorized === false;
    const lockAuthorizationMissing = windowsWithoutHelper && permissions.lock_screen_setup_required !== false;
    const waitForUnlock = windowsWithoutHelper && permissions.lock_screen_setup_required === false && permissions.reason_code === "session_locked";
    const unlockedOnly = lockAuthorizationMissing && (permissionApproved || permissions.reason_code === "session_locked");
    if (!lockAuthorizationMissing || permissions.requesting) card.lockRequestFeedback = "";
    else if (Object.hasOwn(copy.lockRequestResults, permissions.lock_screen_request_result)) {
      card.lockRequestFeedback = copy.lockRequestResults[permissions.lock_screen_request_result];
    }
    const windowsPermissionMessage = lockAuthorizationMissing
      ? permissions.requesting ? copy.lockAccessPending : card.lockRequestFeedback || copy.lockAccessHelp
      : "";
    if (card.actionError === copy.lockAccessDeferred && !lockAuthorizationMissing) card.actionError = "";
    p.permissionSummary.textContent = waitForUnlock ? copy.locked : unlockedOnly ? copy.unlockedOnly : permissionApproved ? copy.permissionApproved : copy.permissionUnapproved;
    p.permissionSummary.hidden = item.kind !== "device";
    const state = item.status || (item.ready ? "running" : "disabled");
    const badge = item.managed && item.installed ? (item.access_enabled ? "running" : "disabled") : state;
    p.badge.textContent = copy[badge] || copy.disabled; p.badge.dataset.state = badge;
    if (item.preview) {
      try {
        const preview = new URL(item.preview, location.href);
        if (preview.origin === location.origin && preview.pathname.startsWith("/api/previews/")) {
          // A hidden lazy image never intersects the viewport to reveal itself.
          p.image.hidden = false;
          if (p.image.src !== preview.href) p.image.src = preview.href;
        }
      } catch {}
    }
    if (item.kind === "instance") {
      p.version.textContent = item.runtime_version ? `${copy.runtime} v${String(item.runtime_version).replace(/^v/, "")} · ${copy.instanceManaged}` : copy.instanceManagedFallback;
    } else {
      p.version.textContent = `${copy.runtime} · ${item.installed ? item.runtime_version || copy.installedUnknown : copy.not_installed}`;
      if ((!item.installed || item.update_available) && item.available_version) p.version.textContent += ` · ${copy.latest} ${item.available_version}`;
    }
    p.version.title = p.version.textContent;
    p.operation.hidden = !active;
    p.operationLabel.textContent = item.removing ? copy.removing : copy[deployment.stage] || copy.working;
    const value = Number.isFinite(deployment.progress) ? Math.max(0, Math.min(100, deployment.progress)) : 0;
    p.progress.value = value; p.progress.setAttribute("aria-label", p.operationLabel.textContent);
    p.operationValue.textContent = `${value}%`;
    if (deployment.stage === "downloading" && deployment.total > 0) p.operationValue.textContent += ` · ${(deployment.received / 1048576).toFixed(1)} / ${(deployment.total / 1048576).toFixed(1)} MiB`;
    p.install.hidden = !item.managed || item.installed && !item.update_available && !active && deployment.stage !== "failed";
    p.install.textContent = item.installed ? copy.update : copy.install;
    if (active) p.install.textContent = copy.working;
    else if (deployment.stage === "failed" && item.removing) p.install.textContent = copy.retryRemoval;
    else if (deployment.stage === "failed" && item.installed) p.install.textContent = copy.retryOperation;
    p.install.disabled = item.online === false || !item.can_install || active || processing || stopping || stopPending;
    const clientUpdateRequired = item.kind === "device" && item.online && !p.install.hidden
      && !item.can_install && !(item.controller_protocol >= 2) && !active && !processing;
    p.install.title = clientUpdateRequired ? copy.client_update_required : item.available_version ? `${copy.latest} ${item.available_version}` : "";
    if (clientUpdateRequired) p.install.setAttribute("aria-describedby", p.error.id);
    else p.install.removeAttribute("aria-describedby");
    p.actions.hidden = !item.managed || !item.installed;
    p.trigger.ariaLabel = `${copy.menu}: ${item.name}`;
    if (p.actions.hidden && openMenu?.trigger === p.trigger) closeMenu();
    const toggleLabel = copy[toggleAction(item)];
    p.toggle.textContent = toggleLabel;
    p.toggle.disabled = item.online === false || processing || switching;
    if (openMenu?.card === card) renderPower(openMenu);
    p.toggle.ariaLabel = `${toggleLabel}: ${item.name}`;
    p.uninstall.disabled = p.toggle.disabled || stopping || stopPending || !item.can_install || !item.can_uninstall;
    p.request.hidden = item.kind !== "device" || waitForUnlock || permissionApproved && !lockAuthorizationMissing;
    p.request.textContent = !mac && (permissions.requesting || pendingPermissions.has(item.id)) ? copy.requesting : copy.permissions;
    p.request.disabled = item.online === false || !item.managed || !item.installed || switching ||
      (!mac && (pendingPermissions.has(item.id) || permissions.requesting === true));
    p.request.title = !item.installed ? copy.not_installed : windowsPermissionMessage;
    if (linux && item.managed && item.installed) {
      const authorize = permissions.request_mode === "system" || deferred;
      p.request.hidden = permissionApproved || !(unavailable || authorize || physicalUnavailable);
      p.request.textContent = permissions.requesting ? copy.requesting : authorize ? copy.permissions : copy.details;
      p.request.title = permissionHelp(item);
    }
    const canEnter = item.managed ? item.access_enabled && item.can_launch === true : item.ready === true;
    const ready = !item.current_device && item.online !== false && canEnter && !switching && !stopping && !stopPending && !processing &&
      (!mac || permissionApproved) && !(deferred && permissions.requesting);
    p.connect.setAttribute("aria-disabled", String(!ready)); p.connect.tabIndex = ready ? 0 : -1;
    if (ready) p.connect.href = item.can_launch ? `/open.html?id=${encodeURIComponent(item.id)}` : item.url; else p.connect.removeAttribute("href");
    const enter = state === "locked" && permissions.reason_code === "lock_screen_ready" ? copy.enterLocked : copy.enterDesktop;
    p.connectLabel.textContent = item.current_device ? copy.currentDevice : ready ? enter
      : item.kind === "device" && !item.installed ? copy.installToEnter
      : state === "locked" ? copy.physicalLocked
      : permissions.requesting || deferred || mac && !permissionApproved || state === "permission_required" ? copy.authorizeToEnter
      : state === "starting" ? copy.startingDesktop : copy.temporarily_unavailable;
    p.connectLabel.hidden = active || stopping || stopPending || !ready && !item.current_device && item.managed && !item.access_enabled;
    p.connect.classList.toggle("show-entry-hint", !p.connectLabel.hidden);
    p.connect.ariaLabel = item.current_device ? copy.currentDevice : `${enter}: ${item.name}`;
    const runtimeMessage = deployment.stage === "failed" ? deployment.error || copy.failedInstall
      : item.runtime_error === "desktop_process_stopped" ? copy.stopped : item.runtime_error || "";
    const macHint = mac && item.managed && item.installed && !permissionApproved;
    if (macHint || lockAuthorizationMissing) p.request.setAttribute("aria-describedby", p.error.id);
    else p.request.removeAttribute("aria-describedby");
    const message = [...new Set([card.actionError || "", stopPending ? copy.stopRetrying : stopping ? copy.stopping : "", runtimeMessage, clientUpdateRequired ? copy.client_update_required : "",
      macHint ? macPermissionsHelp(item) : "", waitForUnlock ? copy.physicalLocked : card.actionError === copy.lockAccessDeferred ? "" : windowsPermissionMessage,
      unavailable ? permissions.reason || permissionHelp(item) : deferred ? permissionHelp(item) : ""].filter(Boolean))].join("\n");
    if (p.error.textContent !== message) p.error.textContent = message;
    p.error.hidden = !message;
    updatePermissionDialog(item);
  }
  function render(targets) {
    const visible = targets.filter(item => item.online !== false);
    const ids = new Set(visible.map(item => item.id));
    if (permissionDevice && !ids.has(permissionDevice)) permissionDialog.close();
    for (const [id, card] of cards) if (!ids.has(id)) { if (openMenu?.trigger === card.parts.trigger) closeMenu(); card.remove(); cards.delete(id); pendingActions.delete(id); }
    for (const item of visible) {
      let card = cards.get(item.id);
      if (!card) { card = createCard(item); cards.set(item.id, card); }
      updateCard(card, item);
    }
    empty.hidden = visible.length > 0; status.textContent = copy.count(visible.length);
  }
  async function act(id, action) {
    const card = cards.get(id);
    const timeout = action === "uninstall" ? 28000 : 16000;
    const repeatable = action === "permissions" && macDevice(card?.target);
    if (action === "permissions") {
      if (!repeatable && pendingPermissions.has(id)) return;
      if (!repeatable) pendingPermissions.add(id);
    } else {
      if (pendingActions.has(id)) return;
      pendingActions.set(id, { action, until: Date.now() + timeout });
    }
    if (card) { card.actionError = ""; updateCard(card, card.target); }
    try {
      const response = await fetch(`/api/hosts/${encodeURIComponent(id)}/${action}`, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: "{}", signal: AbortSignal.timeout(timeout) });
      if (response.redirected || response.status === 401) throw Error("login_required");
      if (!response.ok) {
        const failure = await response.json().catch(() => null);
        if (controlFailures.has(failure?.error)) throw Error(failure.error);
        throw Error(response.status === 403 ? "login_required" : "failed");
      }
      warning.hidden = true;
      const reply = await response.json();
      if (reply?.permission_request_deferred === true && card) card.actionError = copy.lockAccessDeferred;
      if (["install", "update", "update-confirmed", "uninstall"].includes(action)) pendingActions.delete(id);
      const state = reply?.state;
      controlRevision += 1;
      if (typeof reply?.access_enabled === "boolean") {
        pendingActions.delete(id);
        if (card) updateCard(card, { ...card.target, access_enabled: reply.access_enabled });
      } else if (card && state && typeof state.ready === "boolean" && typeof state.enabled === "boolean") {
        updateCard(card, { ...card.target, enabled: state.enabled, ready: state.ready,
          status: state.status, permissions: state.permissions || card.target.permissions,
          deployment: state.deployment || card.target.deployment, runtime_error: state.error });
      }
      await load(true);
      return reply;
    } catch (error) {
      if (action !== "permissions") pendingActions.delete(id);
      if (error.message === "update_confirmation_required") {
        await load();
        if (await confirmUpdate()) await act(id, "update-confirmed");
        return;
      }
      const message = error.name === "TimeoutError" ? copy.control_timeout : copy[error.message] || copy.failed;
      if (card?.isConnected && error.message !== "login_required") card.actionError = message;
      else { warning.textContent = message; warning.hidden = false; }
      if (action === "permissions" && permissionDevice === id) document.getElementById("permission-feedback").textContent = message;
      if (card) updateCard(card, card.target);
    } finally {
      if (action === "permissions" && !repeatable) {
        pendingPermissions.delete(id);
        if (card?.isConnected) updateCard(card, card.target);
      }
    }
  }
  let permissionRefresh;
  function refreshMacPermissions() {
    const scope = permissionRefresh;
    if (!scope || document.hidden || Date.now() < scope.nextCheck) return;
    for (const { target: item } of cards.values()) {
      if (!macDevice(item) || item.online === false || !item.managed || !item.installed || !item.permission_check ||
          !["idle", "completed", "failed"].includes(item.deployment?.stage || "idle") || scope.pending.has(item.id)) continue;
      scope.pending.add(item.id);
      scope.nextCheck = Date.now() + 2000;
      void fetch(`/api/hosts/${encodeURIComponent(item.id)}/permissions-check`, {
        method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: "{}",
        signal: scope.controller.signal,
      }).then(async response => {
        if (!response.ok) return;
        const { state } = await response.json();
        const card = cards.get(item.id);
        if (permissionRefresh !== scope || !card || !state?.permissions ||
            state.version !== card.target.runtime_version || document.hidden) return;
        updateCard(card, { ...card.target, permissions: state.permissions });
      }).catch(() => {}).finally(() => scope.pending.delete(item.id));
    }
  }
  async function load(force = false) {
    if (document.hidden) return;
    if (busy) { reloadRequested ||= force; return; }
    busy = true;
    refreshedAt = Date.now();
    const revision = controlRevision, scope = permissionRefresh;
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
      if (scope !== permissionRefresh || document.hidden) return;
      render(targets); warning.textContent = copy.unavailable; warning.hidden = !data.unavailable?.length;
      refreshMacPermissions();
    } catch (error) {
      if (scope !== permissionRefresh || document.hidden) return;
      status.textContent = ""; warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
      if (error.message === "login_required") render([]);
    } finally {
      clearTimeout(timeout); busy = false; pending = undefined;
      if (reloadRequested) { reloadRequested = false; void load(); }
    }
  }
  function refresh() {
    refreshMacPermissions();
    const transitioning = [...pendingActions.values()].some(p => p.action === "start" || p.action === "stop") || [...cards.values()].some(card => ["starting", "stopping"].includes(card.target.status));
    const active = [...cards.values()].some(card => !["idle", "completed", "failed"].includes(card.target.deployment?.stage || "idle"));
    const requesting = [...cards.values()].some(card => card.target.permissions?.requesting);
    if (Date.now() - refreshedAt >= (transitioning ? 250 : active || requesting ? 1000 : 4000)) void load();
  }
  let timer;
  function startRefresh() {
    if (document.hidden || timer !== undefined) return;
    permissionRefresh = { controller: new AbortController(), pending: new Set(), nextCheck: 0 };
    timer = setInterval(refresh, 250);
    void load(true);
  }
  function stopRefresh() {
    closeMenu();
    pending?.abort();
    permissionRefresh?.controller.abort();
    permissionRefresh = undefined;
    clearInterval(timer);
    timer = undefined;
  }
  document.addEventListener("visibilitychange", () => { if (document.hidden) stopRefresh(); else startRefresh(); });
  window.addEventListener("pagehide", stopRefresh);
  window.addEventListener("pageshow", startRefresh);
  startRefresh();
})();
