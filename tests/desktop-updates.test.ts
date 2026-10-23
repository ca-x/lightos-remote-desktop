import { afterEach, expect, it, vi } from "vitest";
import { decodeDesktopTiles, parseDesktopUpdate } from "../web/src/desktop-updates";

function packet(x=3,y=4,width=2,height=2) {
  const data=new ArrayBuffer(41),view=new DataView(data);view.setUint8(0,4);
  for(const [offset,value] of [[1,1280],[5,720],[9,1],[13,x],[17,y],[21,width],[25,height],[29,8]])view.setUint32(offset,value,true);
  new Uint8Array(data,33).set([137,80,78,71,13,10,26,10]);return data;
}
afterEach(()=>vi.unstubAllGlobals());
it("validates bounded PNG damage and rejects out-of-bounds or incomplete regions",()=>{
  const update=parseDesktopUpdate(packet());expect(update.width).toBe(1280);expect(update.tiles[0]).toMatchObject({x:3,y:4,width:2,height:2});
  expect(()=>parseDesktopUpdate(packet(1279))).toThrow();expect(()=>parseDesktopUpdate(packet(0,0,65))).toThrow();
  expect(()=>parseDesktopUpdate(packet().slice(0,40))).toThrow();
  const bad=packet();new DataView(bad).setUint32(9,100,true);expect(()=>parseDesktopUpdate(bad)).toThrow();
});
it("closes every bitmap when one of the parallel tile decoders fails",async()=>{
  const image={width:2,height:2,close:vi.fn()};let finish!:(image:unknown)=>void;
  vi.stubGlobal("createImageBitmap",vi.fn().mockImplementationOnce(()=>Promise.reject(Error("decode"))).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;})));
  const tiles=parseDesktopUpdate(packet()).tiles;
  const decoding=decodeDesktopTiles([...tiles,...tiles]);
  finish(image);await expect(decoding).rejects.toThrow("decode");expect(image.close).toHaveBeenCalledOnce();
});
