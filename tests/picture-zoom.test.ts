import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPictureZoom } from "../web/src/picture-zoom";

let zoom: ReturnType<typeof createPictureZoom>;
let canvas: HTMLCanvasElement;
let enabled: boolean;
let touching: boolean;
const area = {left:0,top:0,right:640,bottom:400};
beforeEach(() => {
  Object.assign(area,{left:0,top:0,right:640,bottom:400});
  document.body.innerHTML='<main><canvas></canvas></main><nav><button>Home</button></nav>';
  canvas=document.querySelector('canvas')!;
  enabled=false; touching=false;
  canvas.getBoundingClientRect=()=>{
    const match=canvas.style.transform.match(/translate\(([-\d.e]+)px, ([-\d.e]+)px\) scale\(([-\d.e]+)\)/);
    const x=Number(match?.[1]||0), y=Number(match?.[2]||0), scale=Number(match?.[3]||1);
    return {left:x,top:20+y,right:x+640*scale,bottom:20+y+360*scale,width:640*scale,height:360*scale} as DOMRect;
  };
  zoom=createPictureZoom({canvas,viewport:document.querySelector('main')!,enabled:()=>enabled,
    bounds:()=>area,touching:()=>touching,changed:()=>{}});
});

it('covers the mobile fullscreen area without distortion and permits manual zoom back out',()=>{
  enabled=true;zoom.setCover(true);
  const rect=canvas.getBoundingClientRect();
  expect(rect.left).toBeLessThanOrEqual(area.left);expect(rect.top).toBeLessThanOrEqual(area.top);
  expect(rect.right).toBeGreaterThanOrEqual(area.right);expect(rect.bottom).toBeGreaterThanOrEqual(area.bottom);
  expect(rect.width/rect.height).toBeCloseTo(640/360);
  zoom.gesture({x:320,y:200},{x:320,y:200},.1);zoom.layout();expect(zoom.active()).toBe(false);
  zoom.setCover(false);expect(canvas.style.transform).toBe('');
});

it('retains a cover scale above four when panning a very different aspect ratio',()=>{
  enabled=true;area.bottom=2000;zoom.setCover(true);
  const before=canvas.getBoundingClientRect().height;
  zoom.gesture({x:320,y:1000},{x:310,y:1000},1);
  expect(canvas.getBoundingClientRect().height).toBeCloseTo(before);expect(before).toBeGreaterThanOrEqual(2000);
});
afterEach(()=>{zoom.dispose();document.body.innerHTML='';vi.restoreAllMocks();});
it('blocks page zoom shortcuts and trackpad events while leaving virtual pictures unchanged',()=>{
  for(const code of ['Equal','Minus','Digit0','NumpadAdd']){
    const event=new KeyboardEvent('keydown',{code,ctrlKey:true,cancelable:true,bubbles:true});
    document.dispatchEvent(event);expect(event.defaultPrevented).toBe(true);
  }
  const pinch=new WheelEvent('wheel',{ctrlKey:true,deltaY:-40,clientX:320,clientY:200,cancelable:true,bubbles:true});
  canvas.dispatchEvent(pinch);expect(pinch.defaultPrevented).toBe(true);expect(zoom.active()).toBe(false);
  expect(canvas.style.transform).toBe('');
  const wheel=new WheelEvent('wheel',{deltaY:40,cancelable:true,bubbles:true});
  canvas.dispatchEvent(wheel);expect(wheel.defaultPrevented).toBe(false);
});
it('keeps the image point beneath the pinch center and bounds panning without changing the dock',()=>{
  enabled=true;
  const dock=document.querySelector('nav')!;const previous=dock.outerHTML;
  zoom.gesture({x:320,y:200},{x:320,y:200},2);
  let rect=canvas.getBoundingClientRect();
  expect((320-rect.left)/rect.width).toBe(.5);expect((200-rect.top)/rect.height).toBe(.5);
  expect(rect.width).toBe(1280);expect(canvas.style.clipPath).toContain('inset(');
  zoom.gesture({x:320,y:200},{x:350,y:225},1);
  rect=canvas.getBoundingClientRect();
  expect((350-rect.left)/rect.width).toBe(.5);expect((225-rect.top)/rect.height).toBe(.5);
  zoom.gesture({x:350,y:225},{x:10000,y:10000},1);
  rect=canvas.getBoundingClientRect();expect(rect.left).toBe(0);expect(rect.top).toBe(0);
  expect(dock.outerHTML).toBe(previous);
  zoom.gesture({x:320,y:200},{x:320,y:200},.1);
  expect(canvas.style.transform).toBe('');expect(canvas.style.clipPath).toBe('');
});
it('does not zoom from toolbar gestures and resets when the selected desktop changes',()=>{
  enabled=true;
  const wheel=new WheelEvent('wheel',{ctrlKey:true,deltaY:-60,cancelable:true,bubbles:true});
  document.querySelector('button')!.dispatchEvent(wheel);
  expect(wheel.defaultPrevented).toBe(true);expect(zoom.active()).toBe(false);
  zoom.gesture({x:320,y:200},{x:320,y:200},100);
  expect(canvas.getBoundingClientRect().width).toBe(2560);
  enabled=false;zoom.layout();expect(zoom.active()).toBe(false);expect(canvas.style.transform).toBe('');
});
it('supports Safari trackpad gestures without applying touchscreen pinches twice',()=>{
  enabled=true;
  const gesture=(type:string,scale:number)=>{
    const event=new Event(type,{bubbles:true,cancelable:true});
    Object.assign(event,{scale,clientX:320,clientY:200});canvas.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  };
  gesture('gesturestart',1);gesture('gesturechange',1.5);gesture('gesturechange',2);gesture('gestureend',2);
  expect(canvas.getBoundingClientRect().width).toBe(1280);
  zoom.reset();touching=true;
  gesture('gesturestart',1);gesture('gesturechange',2);gesture('gestureend',2);
  expect(zoom.active()).toBe(false);
});
