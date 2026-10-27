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
  afterEach(() => { cleanup?.(); cleanup = undefined; vi.useRealTimers(); vi.unstubAllGlobals(); });
  const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
  function setup(rotated = false) {
    const canvas = document.querySelector("canvas")!;
    const image = document.querySelector("img")!;
    canvas.getBoundingClientRect = () => ({ left:10,top:20,width:640,height:400,right:650,bottom:420 } as DOMRect);
    vi.spyOn(canvas, "matches").mockReturnValue(true);
    if (rotated) canvas.getBoundingClientRect = () => ({left:10,top:20,width:400,height:640,right:410,bottom:660} as DOMRect);
    const layer = createCursorLayer(canvas,image,undefined,undefined,()=>rotated);
    cleanup = layer.dispose;
    layer.connect("a".repeat(32));
    return { canvas,image,layer,socket:sockets[0] };
  }
  function packet(extra = {}) {
    return {type:"cursor",visible:true,id:"1",x:.2,y:.3,width:48,height:48,display_width:24,display_height:24,
      hotspot_x:7,hotspot_y:9,units_width:640,units_height:400,png:btoa(String.fromCharCode(137,80,78,71,13,10,26,10)),...extra};
  }
  it("updates the real cursor and hotspot independently of video frame delivery", async () => {
    const app = setup();
    app.socket.message(packet()); await flush();
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
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
    expect(app.canvas.style.cursor).toContain("blob:cursor-2");
    expect(app.image.hidden).toBe(true);
  });
  it("rotates cursor shape and hotspot with the picture while keeping pointer coordinates exact", async () => {
    const app = setup(true);app.socket.message(packet());await flush();app.layer.present(false);
    app.layer.move(110,180,"touch");
    expect(app.layer.position()).toEqual({x:.25,y:.75});
    expect(app.image.style.transform).toBe("translate3d(119px, 173px, 0) rotate(90deg)");
    app.layer.moveNormalized(.75,.25);
    expect(app.layer.position()).toEqual({x:.75,y:.25});
    expect(app.image.style.transform).toBe("translate3d(319px, 493px, 0) rotate(90deg)");
  });
  it("discards a late cursor image after switching sessions", async () => {
    const app = setup(); let finish!: () => void;
    decode = () => new Promise(resolve => { finish = resolve; });
    app.socket.message(packet());
    app.layer.connect("b".repeat(32)); finish(); await flush();
    expect(app.canvas.style.cursor).toBe("default");
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
  it("keeps the displayed touch position across delayed echoes and the next trackpad stroke", async () => {
    vi.useFakeTimers();
    const app = setup(); app.socket.message(packet()); await flush(); app.layer.present(false);
    app.layer.moveNormalized(.4,.4);
    app.layer.moveNormalized(.6,.5);
    vi.advanceTimersByTime(350);
    app.socket.message(packet({x:.4,y:.4,png:undefined})); await flush();
    expect(app.layer.position()).toEqual({x:.6,y:.5});
    expect(app.image.style.transform).toBe("translate3d(387px, 211px, 0)");
    app.layer.present(false); app.layer.layout();
    expect(app.image.style.transform).toBe("translate3d(387px, 211px, 0)");
    const position = app.layer.position(); app.layer.moveNormalized(position.x+.05,position.y);
    expect(app.layer.position()).toEqual({x:.65,y:.5});
    app.socket.message(packet({x:.6,y:.5,png:undefined})); await flush();
    expect(app.layer.position()).toEqual({x:.65,y:.5});
    app.socket.message(packet({x:.65,y:.5,png:undefined})); await flush();
    app.socket.message(packet({x:.3,y:.2,png:undefined})); await flush();
    expect(app.layer.position()).toEqual({x:.3,y:.2});
  });
  it("updates the shape and visibility during prediction, and accepts an unacknowledged remote warp after the grace period", async () => {
    vi.useFakeTimers();
    const app = setup(); app.socket.message(packet()); await flush(); app.layer.present(false);
    app.layer.moveNormalized(.6,.5); vi.advanceTimersByTime(350);
    app.socket.message(packet({id:"2",x:.4,y:.4,hotspot_x:5})); await flush();
    expect(app.image.src).toContain("blob:cursor-2");
    expect(app.image.style.transform).toBe("translate3d(389px, 211px, 0)");
    app.socket.message(packet({id:"2",x:.4,y:.4,visible:false,png:undefined})); await flush();
    expect(app.image.hidden).toBe(true);
    vi.advanceTimersByTime(3000);
    app.socket.message(packet({id:"2",x:.1,y:.2,png:undefined})); await flush();
    expect(app.image.hidden).toBe(false);
    expect(app.layer.position()).toEqual({x:.1,y:.2});
  });
  it("keeps the native mouse cursor through a delayed echo without reinstalling the same cursor on each move", async () => {
    vi.useFakeTimers();
    const app = setup(); app.socket.message(packet()); await flush(); app.layer.present(false);
    app.layer.move(330,220);
    const writes = vi.spyOn(app.canvas.style,"setProperty");
    app.layer.move(340,220); app.layer.move(350,220);
    vi.advanceTimersByTime(350);
    app.socket.message(packet({x:.5,y:.5,png:undefined})); await flush();
    expect(app.image.hidden).toBe(true);
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
    expect(writes.mock.calls.filter(([name])=>name==="cursor")).toHaveLength(0);
    vi.advanceTimersByTime(3000);
    app.socket.message(packet({x:.1,y:.2,png:undefined})); await flush();
    expect(app.image.hidden).toBe(false);
    expect(app.canvas.style.cursor).toBe("none");
    app.layer.move(360,220);
    expect(app.image.hidden).toBe(true);
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
  });
  it("recovers a lost cursor channel without dropping touch prediction or enabling a duplicate cursor", async () => {
    vi.useFakeTimers();
    const app = setup(); app.socket.message(packet()); await flush(); app.layer.present(false);
    app.layer.moveNormalized(.6,.5);
    app.socket.onclose?.();
    app.layer.present(true);
    expect(app.image.hidden).toBe(false);
    vi.advanceTimersByTime(250);
    expect(sockets).toHaveLength(2);
    expect(sockets[1].url.href).toBe(app.socket.url.href);
    sockets[1].message(packet({id:"2",x:.4,y:.4,hotspot_x:5})); await flush();
    expect(app.image.hidden).toBe(false);
    app.layer.present(false);
    expect(app.image.hidden).toBe(false);
    expect(app.image.src).toContain("blob:cursor-2");
    expect(app.layer.position()).toEqual({x:.6,y:.5});
    app.socket.message(packet({visible:false})); await flush();
    expect(app.image.hidden).toBe(false);
  });
  it("retains the independent cursor when a legacy frame requests an embedded cursor", async () => {
    const app = setup();app.socket.message(packet());await flush();app.layer.present(false);
    app.canvas.style.setProperty("cursor","default");
    app.layer.present(true);
    expect(app.canvas.style.cursor).toContain("blob:cursor-1");
    expect(app.image.hidden).toBe(true);
  });
  it("backs off failed cursor connections and cancels retries and late image decodes on session changes", async () => {
    vi.useFakeTimers();
    const app = setup(); let finish!: () => void;
    decode = () => new Promise(resolve => { finish = resolve; });
    app.socket.message(packet());
    app.socket.onclose?.();
    finish(); await flush();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:cursor-1");
    vi.advanceTimersByTime(250);
    sockets[1].onclose?.();
    vi.advanceTimersByTime(250);
    expect(sockets).toHaveLength(2);
    app.layer.connect("b".repeat(32));
    vi.advanceTimersByTime(5000);
    expect(sockets).toHaveLength(3);
    expect(sockets[2].url.searchParams.get("session")).toBe("b".repeat(32));
    sockets[2].onclose?.(); app.layer.reset(); vi.advanceTimersByTime(5000);
    expect(sockets).toHaveLength(3);
  });
});
