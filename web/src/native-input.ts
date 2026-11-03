type CaptureEvent = { type: string; code?: number; pressed?: boolean; reason?: string; message?: string; backend?: string };
/** Optional viewing-device capture supplied by the runtime manager, not the PC shell. */
export function createNativeInput(key: (code: number, pressed: boolean) => void, released: (reason: string) => void) {
  let available = false, socket: WebSocket | null = null, active = false, pending = false, backend = "";
  let heartbeat: ReturnType<typeof setInterval> | undefined, epoch = 0;
  let cancelStart: (() => void) | undefined;
  const endpoint = new URL("./viewer-input", location.href);
  const probe = new URL(endpoint);probe.searchParams.set("capabilities", "true");
  let reason = "Native keyboard capture requires the current device's runtime", lastProbe = 0;
  async function refresh() {
    if (Date.now() - lastProbe < 5000) return available;
    lastProbe = Date.now();
    try {
      const response = await fetch(probe, { credentials: "same-origin", signal: AbortSignal.timeout(3000) });
      const value = response.ok ? await response.json() : null;
      available = value?.available === true;
      if (typeof value?.reason === "string") reason = value.reason;
    } catch { available = false; }
    return available;
  }
  void refresh();
  function stop() {
    epoch++;pending = active = false;backend = "";
    clearInterval(heartbeat);heartbeat = undefined;
    cancelStart?.();cancelStart = undefined;
    const previous = socket;socket = null;
    if (previous) { previous.onmessage = previous.onclose = previous.onerror = null;previous.close(); }
  }
  return {
    get available() { return available; },
    get reason() { return reason; },
    refresh,
    active: () => active,
    acquiring: () => pending,
    ownsWaylandFocus: () => pending || backend === "wayland",
    start(): Promise<{ok:boolean;reason?:string}> {
      if (!available || pending || active) return Promise.resolve({ ok: active });
      const generation = ++epoch;pending = true;
      return new Promise(resolve => {
        const url = new URL(endpoint);url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
        const current = new WebSocket(url);socket = current;let settled = false;
        const finish = (ok: boolean, reason?: string) => {if (!settled) {settled = true;clearTimeout(timer);cancelStart = undefined;resolve({ok,reason});}};
        const timer = setTimeout(() => {finish(false,"Viewing-device keyboard capture timed out");stop();},4000);
        cancelStart = () => finish(false,"Keyboard capture cancelled");
        current.onopen = () => {
          if(generation!==epoch)return current.close();
          current.send(JSON.stringify({type:"capture",title:document.title}));
          heartbeat=setInterval(()=>{if(current.readyState===WebSocket.OPEN)current.send("heartbeat");},250);
        };
        current.onmessage = ({data}) => {
          if(generation!==epoch || typeof data!=="string")return;
          let event:CaptureEvent;try{event=JSON.parse(data);}catch{return;}
          if(event.type==="acquiring")backend=event.backend||"";
          else if(event.type==="ready"){pending=false;active=true;backend=event.backend||"native";finish(true);}
          else if(event.type==="key"&&active&&Number.isInteger(event.code)&&typeof event.pressed==="boolean")key(event.code!,event.pressed);
          else if(event.type==="error"||event.type==="released"){finish(false,event.message);stop();released(event.reason||"capture_ended");}
        };
        current.onerror = () => {if(generation===epoch){finish(false,"Viewing-device keyboard capture is unavailable");stop();released("capture_ended");}};
        current.onclose = () => {if(generation===epoch){finish(false,"Keyboard capture ended");stop();released("capture_ended");}};
      });
    },
    stop,
    destroy: stop,
  };
}
