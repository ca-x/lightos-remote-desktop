(() => {
  "use strict";
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const copy = zh ? {
    title: "远程桌面", subtitle: "连接你的容器实例和电脑。", refresh: "刷新", all: "全部", container: "容器实例", physical: "物理机",
    empty: "暂无可连接的桌面", help: "容器实例：在 LightOS 实例设置中安装并开启远程桌面。物理机：在客户端设置中开启“接入 LightOS”和“远程桌面”。",
    lightos: "打开 LightOS", scope: "仅显示当前账号有权访问且已就绪的桌面。", loading: "正在检查桌面…", connect: "进入桌面", search: "搜索桌面", failed: "加载失败，请检查登录状态、LightOS 权限和版本后重试。",
    consent: "需本机授权", unavailable: "以下桌面或设备暂不可用：", count: n => `${n} 个可用桌面`, filtered: "没有匹配的桌面",
  } : {
    title: "Remote desktops", subtitle: "Connect to your containers and computers.", refresh: "Refresh", all: "All", container: "Containers", physical: "Computers",
    empty: "No desktops available", help: "Containers: install and enable remote desktop in the LightOS instance settings. Computers: enable Connect to LightOS and Remote desktop in the client settings.",
    lightos: "Open LightOS", scope: "Only authorized, ready desktops are shown.", loading: "Checking desktops…", connect: "Connect", search: "Search desktops", failed: "Unable to load desktops. Check your login, LightOS access and version, then retry.",
    consent: "Local consent required", unavailable: "These desktops or devices are unavailable: ", count: n => `${n} desktops available`, filtered: "No matching desktops",
  };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  document.title = `LightOS · ${copy.title}`;
  document.querySelectorAll("[data-text]").forEach(node => { node.textContent = copy[node.dataset.text]; });
  const search = document.getElementById("search");
  search.placeholder = search.ariaLabel = copy.search;
  const list = document.getElementById("desktops"), status = document.getElementById("status"), warning = document.getElementById("warning"), empty = document.getElementById("empty"), refresh = document.getElementById("refresh");
  let targets = [], filter = "all", busy = false, pending;
  function render() {
    const query = search.value.trim().toLocaleLowerCase();
    const visible = targets.filter(item => (filter === "all" || item.kind === filter) && `${item.name} ${item.platform}`.toLocaleLowerCase().includes(query));
    const fragment = document.createDocumentFragment();
    for (const item of visible) {
      const card = document.createElement("article"), icon = document.createElement("div"), detail = document.createElement("div"), name = document.createElement("h2"), description = document.createElement("p"), link = document.createElement("a");
      icon.className = "desktop-icon"; icon.textContent = item.kind === "physical" ? "▣" : "◈"; icon.ariaHidden = "true";
      name.textContent = item.name; description.textContent = `${copy[item.kind] || item.kind} · ${item.platform || "LightOS"}${item.requires_consent ? " · " + copy.consent : ""}`;
      link.className = "connect"; link.textContent = copy.connect; link.href = item.url; link.target = "_blank"; link.rel = "noopener noreferrer"; link.ariaLabel = `${copy.connect}: ${item.name}`;
      detail.append(name, description); card.append(icon, detail, link); fragment.append(card);
    }
    list.replaceChildren(fragment);
    empty.hidden = visible.length > 0 || busy;
    empty.querySelector("h2").textContent = targets.length > 0 ? copy.filtered : copy.empty;
    if (!busy) status.textContent = copy.count(targets.length);
  }
  async function load() {
    if (busy || document.hidden) return;
    busy = true; refresh.disabled = true; status.textContent = copy.loading;
    pending = new AbortController();
    const timeout = setTimeout(() => pending?.abort(), 28000);
    try {
      const response = await fetch("/api/remote-desktop/targets", { credentials: "same-origin", cache: "no-store", signal: pending.signal });
      if (!response.ok) throw new Error("request rejected");
      const data = await response.json();
      if (!Array.isArray(data.targets)) throw new Error("invalid response");
      targets = data.targets.filter(item => {
        const url = new URL(item.url, location.href);
        return ["https:", "http:"].includes(url.protocol) && url.pathname.startsWith("/remote-desktop/") && !url.username && !url.password;
      });
      const missing = Array.isArray(data.unavailable) ? data.unavailable : [];
      warning.textContent = missing.length ? copy.unavailable + missing.join("、") : ""; warning.hidden = !missing.length;
      if (data.lightos_url) {
        const url = new URL(data.lightos_url);
        if (url.protocol === "https:" && !url.username && !url.password) document.getElementById("lightos").href = url.href;
      }
    } catch {
      targets = []; warning.textContent = copy.failed; warning.hidden = false;
    } finally { clearTimeout(timeout); busy = false; refresh.disabled = false; pending = undefined; render(); }
  }
  document.querySelectorAll("[data-filter]").forEach(button => button.addEventListener("click", () => {
    filter = button.dataset.filter;
    document.querySelectorAll("[data-filter]").forEach(node => node.setAttribute("aria-pressed", String(node === button)));
    render();
  }));
  search.addEventListener("input", render);
  refresh.addEventListener("click", load);
  document.addEventListener("visibilitychange", () => { if (document.hidden) pending?.abort(); else void load(); });
  window.addEventListener("pagehide", () => pending?.abort());
  setInterval(load, 30000);
  void load();
})();
