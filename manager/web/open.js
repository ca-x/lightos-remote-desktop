(async () => {
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const query = new URLSearchParams(location.search);
  const id = query.get("id");
  const message = document.querySelector("[role=status]");
  const back = document.querySelector("a");
  document.documentElement.lang = zh ? "zh-CN" : "en";
  message.textContent = zh ? "正在连接远程桌面…" : "Connecting to remote desktop…";
  try {
    if (!/^[0-9a-f]{32}$/.test(id)) throw Error();
    const response = await fetch(`/api/hosts/${id}/connect`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: "{}",
      signal: AbortSignal.timeout(16000),
    });
    if (!response.ok || response.redirected) throw Error();
    const deadline = Date.now() + 35000;
    while (Date.now() < deadline) {
      const state = await fetch("/api/targets?kind=device", { signal: AbortSignal.timeout(6000) });
      if (!state.ok) throw Error();
      const item = (await state.json()).targets.find(target => target.id === id);
      if (!item || !item.online || !item.access_enabled || item.current_device || !item.installed) throw Error();
      if (item.ready && item.can_connect) {
        location.replace(`/desktop/${id}/${query.get("from") === "lightos" ? "?from=lightos" : ""}`);
        return;
      }
      if (item.status === "error" || item.status === "permission_required") throw Error();
      await new Promise(resolve => setTimeout(resolve, 500));
    }
  } catch {}
  message.textContent = zh ? "桌面未就绪，请返回设备卡片查看状态和权限。" : "Desktop unavailable. Check the status and permissions on the device card.";
  back.textContent = zh ? "返回设备列表" : "Back to devices";
  back.hidden = false;
})();
