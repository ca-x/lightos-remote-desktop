import { afterEach, expect, it, vi } from "vitest";
import { createScreenRotation } from "../web/src/screen-rotation";
import { pictureClientPoint, pictureNormalizedPoint, pictureScrollDelta } from "../web/src/picture-geometry";

afterEach(()=>vi.useRealTimers());
function setup(orientation?: {lock?: (value:"portrait"|"landscape")=>Promise<void>;unlock?:()=>void}) {
  vi.useFakeTimers();let active=true,landscape=false,rotated=false,pending=false;
  const rotation=createScreenRotation({active:()=>active,landscape:()=>landscape,orientation:()=>orientation,rotated:value=>{rotated=value;},pending:value=>{pending=value;},release:vi.fn()});
  return {rotation,get rotated(){return rotated;},get pending(){return pending;},exit(){active=false;rotation.reset();},landscape(){landscape=true;}};
}
it("locks native orientation in either direction and releases only its own lock on exit", async()=>{
  const lock=vi.fn(async()=>{}),unlock=vi.fn();const app=setup({lock,unlock});
  app.rotation.toggle();expect(lock).toHaveBeenLastCalledWith('landscape');await Promise.resolve();expect(app.rotated).toBe(false);
  app.landscape();app.rotation.toggle();expect(lock).toHaveBeenLastCalledWith('portrait');await Promise.resolve();app.exit();expect(unlock).toHaveBeenCalledTimes(1);
});
it("unavailable or denied native rotation falls back, toggles back and follows a physical device turn", async()=>{
  for(const orientation of [undefined,{lock:async()=>{throw new Error('unsupported');}}]) {
    const app=setup(orientation);app.rotation.toggle();await Promise.resolve();expect(app.rotated).toBe(true);
    app.rotation.toggle();expect(app.rotated).toBe(false);
    app.rotation.toggle();await Promise.resolve();app.rotation.deviceChanged();expect(app.rotated).toBe(false);app.exit();
  }
});
it("bounds a hung orientation request and ignores completion after exiting fullscreen", async()=>{
  let resolve!:()=>void;const unlock=vi.fn();const app=setup({lock:()=>new Promise<void>(r=>{resolve=r;}),unlock});
  app.rotation.toggle();expect(app.pending).toBe(true);vi.advanceTimersByTime(1500);expect(app.rotated).toBe(true);expect(app.pending).toBe(false);expect(unlock).toHaveBeenCalledTimes(1);
  app.exit();resolve();await Promise.resolve();expect(app.rotated).toBe(false);expect(vi.getTimerCount()).toBe(0);
});
it("rotated image points and scrolling retain desktop axes across zoomed rectangles",()=>{
  for(const rect of [{left:0,top:0,width:390,height:844},{left:-80,top:-120,width:780,height:1688}]) {
    expect(pictureNormalizedPoint(rect,{x:rect.left,y:rect.top},true)).toEqual({x:0,y:1});
    expect(pictureNormalizedPoint(rect,{x:rect.left+rect.width,y:rect.top+rect.height},true)).toEqual({x:1,y:0});
    for(const point of [{x:.2,y:.7},{x:.8,y:.1}]) {
      const actual=pictureNormalizedPoint(rect,pictureClientPoint(rect,point,true),true);
      expect(actual.x).toBeCloseTo(point.x);expect(actual.y).toBeCloseTo(point.y);
    }
  }
  expect(pictureScrollDelta(30,10,true)).toEqual({x:10,y:-30});
});
