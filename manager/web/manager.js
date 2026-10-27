(() => {
  "use strict";
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "远程桌面", subtitle: "选择一台设备，开始连接。", refresh: "刷新", empty: "暂无可连接的远程桌面",
    help: "请确认设备在线，并已开启远程桌面。", loading: "正在获取可用桌面…", connect: "连接", consent: "需本机授权",
    failed: "无法获取桌面，请稍后重试。", login_required: "登录已失效，请重新登录后刷新。", unavailable: "部分桌面来源暂不可用，请确认访问授权后刷新。", count: n => `${n} 个可连接的远程桌面`,
  } : {
    title: "Remote Desktop", subtitle: "Choose a device to connect.", refresh: "Refresh", empty: "No remote desktops available",
    help: "Make sure your devices are online and remote desktop is enabled.", loading: "Finding available desktops…", connect: "Connect", consent: "Local consent required",
    failed: "Unable to load desktops. Please try again.", login_required: "Your session has expired. Sign in again and refresh.", unavailable: "Some desktop sources are unavailable. Check access authorization and refresh.", count: n => `${n} remote desktops available`,
  };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = copy.title;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const list = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty"), refresh = document.getElementById("refresh");
  let busy = false, pending;
  function render(targets) {
    const fragment = document.createDocumentFragment();
    for (const item of targets) {
      const card = document.createElement("article"), icon = document.createElement("img"), detail = document.createElement("div"), name = document.createElement("h2"), description = document.createElement("p"), link = document.createElement("a");
      icon.className = "desktop-icon"; icon.src = "./icon.png"; icon.alt = "";
      name.textContent = item.name; description.textContent = [item.platform, item.requires_consent ? copy.consent : ""].filter(Boolean).join(" · ");
      link.className = "connect"; link.textContent = copy.connect; link.href = item.url; link.target = "_blank"; link.rel = "noopener noreferrer"; link.ariaLabel = `${copy.connect}: ${item.name}`;
      detail.append(name, description); card.append(icon, detail, link); fragment.append(card);
    }
    list.replaceChildren(fragment); empty.hidden = targets.length > 0; status.textContent = copy.count(targets.length);
  }
  async function load() {
    if (busy || document.hidden) return;
    busy = true; refresh.disabled = true; status.textContent = copy.loading;
    const controller = new AbortController(); pending = controller;
    const timeout = setTimeout(() => controller.abort(), 20000);
    try {
      const response = await fetch("/api/targets", { credentials: "same-origin", cache: "no-store", signal: controller.signal });
      if (response.redirected || response.status === 401 || response.status === 403) throw new Error("login_required");
      if (!response.ok) throw new Error("failed");
      const data = await response.json();
      if (!Array.isArray(data.targets)) throw new Error("failed");
      const targets = data.targets.filter(item => {
        if (!item || typeof item.url !== "string" || typeof item.name !== "string") return false;
        try { const url = new URL(item.url, location.href); return url.protocol === "https:" && !url.username && !url.password || url.origin === location.origin && url.pathname.startsWith("/desktop/"); } catch { return false; }
      });
      render(targets); warning.textContent = copy.unavailable; warning.hidden = !data.unavailable?.length;
    } catch (error) {
      if (document.hidden && controller.signal.aborted) return;
      render([]); warning.textContent = copy[error.message] || copy.failed; warning.hidden = false;
    } finally { clearTimeout(timeout); busy = false; refresh.disabled = false; pending = undefined; }
  }
  refresh.addEventListener("click", load);
  document.addEventListener("visibilitychange", () => { if (document.hidden) pending?.abort(); else void load(); });
  const timer = setInterval(load, 30000);
  window.addEventListener("pagehide", () => { pending?.abort(); clearInterval(timer); });
  window.addEventListener("pageshow", event => { if (event.persisted) void load(); });
  void load();
})();
