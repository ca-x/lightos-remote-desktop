// Older LightOS desktops need no upgrade: observe only frames actually drawn
// in this authorized, same-origin connection, then save the last valid cover.
(() => {
  const id = /^\/desktop\/([a-f0-9]{32})\//.exec(location.pathname)?.[1];
  if (!id) return;
  let canvas, last = 0, timer, latest;
  function capture(keepalive = false) {
    if (!canvas || document.hidden && !keepalive) return;
    const cover = document.createElement("canvas");
    const scale = Math.min(1, 640 / canvas.width, 480 / canvas.height);
    cover.width = Math.max(1, Math.round(canvas.width * scale));
    cover.height = Math.max(1, Math.round(canvas.height * scale));
    cover.getContext("2d").drawImage(canvas, 0, 0, cover.width, cover.height);
    let data = cover.toDataURL("image/jpeg", 0.65).split(",")[1];
    if (data.length > 80000) data = cover.toDataURL("image/jpeg", 0.4).split(",")[1];
    const bytes = Uint8Array.from(atob(data), byte => byte.charCodeAt(0));
    if (bytes.length > 60000) return;
    latest = new Blob([bytes], { type: "image/jpeg" });
    last = Date.now();
    void fetch(`/api/previews/${id}`, { method: "POST", body: latest, credentials: "same-origin", keepalive, headers: { "Content-Type": "image/jpeg" } }).catch(() => {});
  }
  function drawn(target) {
    if (target.id !== "desktop" || !target.width || !target.height) return;
    canvas = target;
    if (!timer) timer = setTimeout(() => {
      timer = undefined;
      try { capture(); } catch {}
    }, Math.max(30, 5000 - (Date.now() - last)));
  }
  for (const method of ["drawImage", "putImageData"]) {
    const original = CanvasRenderingContext2D.prototype[method];
    CanvasRenderingContext2D.prototype[method] = function (...args) {
      const result = Reflect.apply(original, this, args);
      drawn(this.canvas);
      return result;
    };
  }
  window.addEventListener("pagehide", () => {
    clearTimeout(timer); timer = undefined;
    try { capture(true); } catch {}
  });
})();
