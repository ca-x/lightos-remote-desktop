/** Persist only pixels the user has actually viewed in a managed desktop session. */
export function createSessionPreview(canvas: HTMLCanvasElement) {
  const id = /^\/desktop\/([a-f0-9]{32})\//.exec(location.pathname)?.[1];
  let latest: Blob | undefined;
  let lastCapture = 0;
  let busy = false;
  let active = true;
  const save = (keepalive = false) => {
    if (!id || !latest) return;
    void fetch(`/api/previews/${id}`, {
      method: "POST", body: latest, credentials: "same-origin", keepalive,
      headers: { "Content-Type": "image/jpeg" },
    }).catch(() => {});
  };
  return {
    frame() {
      if (!id || !active || busy || Date.now() - lastCapture < 5000 || !canvas.width || !canvas.height) return;
      lastCapture = Date.now();
      busy = true;
      const cover = document.createElement("canvas");
      const scale = Math.min(1, 640 / canvas.width, 480 / canvas.height);
      cover.width = Math.max(1, Math.round(canvas.width * scale));
      cover.height = Math.max(1, Math.round(canvas.height * scale));
      cover.getContext("2d")?.drawImage(canvas, 0, 0, cover.width, cover.height);
      cover.toBlob(blob => {
        busy = false;
        if (!active || !blob || blob.size > 60000) return;
        latest = blob;
        save();
      }, "image/jpeg", 0.65);
    },
    flush() {
      // pagehide cannot await an encode; retain the latest valid session frame.
      save(true);
    },
    destroy() { save(true); active = false; latest = undefined; },
  };
}
