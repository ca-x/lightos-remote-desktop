import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createCursorLayer } from "../web/src/cursor";

describe("independent cursor", () => {
  let sockets: Socket[];
  let dimensions: [number, number];
  let decode: () => Promise<void>;
  let cleanup: (() => void) | undefined;
  class Socket {
    onmessage?: (event: { data: string }) => void;
    onclose?: () => void;
    onerror?: () => void;
    close = vi.fn();
    constructor(public url: URL) { sockets.push(this); }
    message(value: unknown) { this.onmessage?.({ data: JSON.stringify(value) }); }
  }
  beforeEach(() => {
    sockets = []; dimensions = [48,48]; decode = () => Promise.resolve();
    document.body.innerHTML = '<canvas></canvas><img />';
    vi.stubGlobal("WebSocket", Socket);
    vi.stubGlobal("Image", class {
      src = "";
      naturalWidth = dimensions[0]; naturalHeight = dimensions[1];
      decode() { return decode(); }
    });
    vi.stubGlobal("CSS", { supports: () => true });
    let id = 0;
    vi.stubGlobal("URL", class extends URL {
      static createObjectURL = vi.fn(() => `blob:cursor-${++id}`);
      static revokeObjectURL = vi.fn();
    });
  });
  afterEach(() => { cleanup?.(); cleanup = undefined; vi.unstubAllGlobals(); });
  const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
  function setup() {
    const canvas = document.querySelector("canvas")!;
    const image = document.querySelector("img")!;
    canvas.getBoundingClientRect = () => ({ left:10,top:20,width:640,height:400,right:650,bottom:420 } as DOMRect);
    vi.spyOn(canvas, "matches").mockReturnValue(true);
    const layer = createCursorLayer(canvas,image);
    cleanup = layer.dispose;
    layer.connect("a".repeat(32));
    return { canvas,image,layer,socket:sockets[0] };
  }
  function packet(extra = {}) {
    return {type:"cursor",visible:true,id:"1",x:.2,y:.3,width:48,height:48,display_width:24,display_height:24,
      hotspot_x:7,hotspot_y:9,units_width:640,units_height:400,png:btoa(String.fromCharCode(137,80,78,71,13,10,26,10)),...extra};
  }
  it("waits for a cursor-free video frame, then updates the real image and hotspot without another video frame", async () => {
    const app = setup();
    app.socket.message(packet()); await flush();
    expect(app.canvas.style.cursor).toBe("none");
    app.layer.present(false);
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
    expect(app.canvas.style.cursor).toContain("2.00000x");
    expect(app.canvas.style.cursor).toContain("7.00 9.00");
    expect(app.image.hidden).toBe(true);
    let finish!: () => void;
    decode = () => new Promise(resolve => { finish = resolve; }); dimensions = [24,48];
    app.socket.message(packet({id:"2",width:24,display_width:12,hotspot_x:6,hotspot_y:12}));
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
    finish(); await flush();
    expect(app.canvas.style.cursor).toContain("blob:cursor-2");
    expect(app.canvas.style.cursor).toContain("6.00 12.00");
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:cursor-1");
    app.layer.present(true);
    expect(app.canvas.style.cursor).toBe("none");
    expect(app.image.hidden).toBe(true);
  });
  it("discards a late cursor image after switching sessions", async () => {
    const app = setup(); let finish!: () => void;
    decode = () => new Promise(resolve => { finish = resolve; });
    app.socket.message(packet());
    app.layer.connect("b".repeat(32)); finish(); await flush();
    expect(app.canvas.style.cursor).toBe("none");
    expect(app.image.hidden).toBe(true);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:cursor-1");
    expect(app.socket.close).toHaveBeenCalled();
  });
  it("applies hidden cursor state and predicts touch position before any echo", async () => {
    const app = setup(); app.socket.message(packet()); await flush(); app.layer.present(false);
    app.layer.move(330,220,"touch");
    expect(app.image.hidden).toBe(false);
    expect(app.image.style.transform).toBe("translate3d(323px, 211px, 0)");
    app.socket.message(packet({visible:false,png:undefined})); await flush();
    expect(app.canvas.style.cursor).toBe("none"); expect(app.image.hidden).toBe(true);
  });
});
