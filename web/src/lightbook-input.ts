import { createTouchPair, type TouchPairSample } from "./touch-pair";
import { isMobilePlatform } from "./platform";

export function supportsLightBook() {
  if ((navigator.maxTouchPoints ?? 0) < 1) return false;
  return isMobilePlatform();
}

type Point = { x: number; y: number };
interface Options {
  root: HTMLElement;
  active: () => boolean;
  ready: () => boolean;
  key: (code: string, pressed: boolean) => boolean;
  button: (button: number, pressed: boolean) => boolean;
  move: (point: Point) => void;
  position: () => Point;
  dimensions: () => { width: number; height: number };
  scroll: (x: number, y: number) => void;
  zoom: (ratio: number) => void;
  zoomActive?: () => boolean;
  pan?: (x: number, y: number) => void;
  engage?: () => void;
  release?: () => void;
}
const isModifier = (code: string) => /^(Control|Alt|Meta|Shift)(Left|Right)$/.test(code);

export function createLightBookInput(options: Options) {
  const keyboard = options.root.querySelector?.<HTMLElement>("#lightbook-keys");
  const pad = options.root.querySelector?.<HTMLElement>("#lightbook-trackpad");
  if (!keyboard || !pad) return null;
  const lifecycle = new AbortController();
  const keyButtons = Array.from(keyboard.querySelectorAll<HTMLButtonElement>("button"));
  const keyTouches = new Map<number, { button: HTMLButtonElement; code: string; modifier: boolean; used: boolean; latched: boolean }>();
  const down = new Set<string>(), latched = new Set<string>();
  let caps = false;
  const contacts = new Map<number, Point & { start: Point }>();
  let position = {x:.5,y:.5}, last: (Point & {distance:number}) | null = null;
  let origin: (Point & {distance:number}) | null = null;
  let gesture: "single" | "pending" | "scroll" | "pan" | "pinch" | "ignore" = "single";
  let pairFrame: number | null = null;
  let moved = false, dragging = false, began = 0, lastTap = -Infinity;
  let hold: ReturnType<typeof setTimeout> | null = null;
  const live = () => options.active() && options.ready();
  const listen = (target: EventTarget, name: string, callback: (event: any) => void) => target.addEventListener(name, callback, {signal:lifecycle.signal,passive:false});
  // iOS can still show its native loupe on tap-and-hold despite user-select:none
  // and cancelled pointerdown. Claim the trackpad touch stream itself, without
  // suppressing the header buttons' clicks or the real text input's gestures.
  for (const name of ["touchstart", "touchmove", "selectstart"]) {
    listen(pad, name, (event: Event) => { if (options.active() && event.cancelable) event.preventDefault(); });
  }
  const held = (code:string) => [...keyTouches.values()].some(key=>key.code===code);
  function visual() {
    for (const button of keyButtons) {
      const code = button.dataset.lightbookKey;
      const pressed = code === "CapsLock" ? caps : !!code && down.has(code);
      button.classList.toggle("is-pressed", pressed);
      if (button.hasAttribute("aria-pressed")) button.setAttribute("aria-pressed",String(pressed));
    }
    pad!.classList.toggle("is-dragging",dragging);
  }
  function press(code:string) {
    if (!down.has(code) && options.key(code,true)) {
      down.add(code);
      if (code === "CapsLock") caps=!caps;
    }
  }
  function releaseKey(code:string) {
    if (!held(code) && !latched.has(code) && down.delete(code)) options.key(code,false);
  }
  function usedModifiers() { for(const key of keyTouches.values()) if(key.modifier) key.used=true; }
  function consumeLatches() {
    if ([...keyTouches.values()].some(key=>!key.modifier)) return;
    const old=[...latched]; latched.clear();
    for(const code of old) releaseKey(code);
    visual();
  }
  function finishKey(id:number) {
    const key=keyTouches.get(id); if(!key)return;
    keyTouches.delete(id);
    if(key.modifier) {
      if(!key.used && !key.latched)latched.add(key.code);
      else if(!key.used)latched.delete(key.code);
      releaseKey(key.code);
    } else { releaseKey(key.code);consumeLatches(); }
    if(key.button.hasPointerCapture?.(id))key.button.releasePointerCapture(id);
    visual();
  }
  listen(keyboard,"pointerdown",(event:PointerEvent)=>{
    const button=(event.target as HTMLElement).closest<HTMLButtonElement>("[data-lightbook-key]");
    if(!button)return;event.preventDefault();event.stopPropagation();
    if(!live()||button.disabled||event.button!==0||keyTouches.has(event.pointerId))return;
    options.engage?.();
    const code=button.dataset.lightbookKey!;
    const modifier=isModifier(code);
    if(!modifier)usedModifiers();
    keyTouches.set(event.pointerId,{button,code,modifier,used:false,latched:latched.has(code)});
    press(code);
    button.setPointerCapture?.(event.pointerId);visual();
  });
  listen(keyboard,"pointerup",(event:PointerEvent)=>{event.preventDefault();event.stopPropagation();finishKey(event.pointerId);});
  listen(keyboard,"pointercancel",()=>cancel());
  listen(keyboard,"lostpointercapture",(event:PointerEvent)=>{if(keyTouches.has(event.pointerId))cancel();});
  listen(keyboard,"click",(event:MouseEvent)=>{
    event.preventDefault();event.stopPropagation();
    if(event.detail || !live())return;
    const button=(event.target as HTMLElement).closest<HTMLButtonElement>("[data-lightbook-key]");
    if(!button||button.disabled)return;
    const code=button.dataset.lightbookKey!;
    if(isModifier(code)){if(latched.delete(code))releaseKey(code);else{latched.add(code);press(code);}}
    else{usedModifiers();press(code);releaseKey(code);consumeLatches();}
    visual();
  });
  const cancelHold=()=>{if(hold!==null)clearTimeout(hold);hold=null;};
  const centroid=()=>{const[a,b]=[...contacts.values()];return b?{x:(a.x+b.x)/2,y:(a.y+b.y)/2,distance:Math.hypot(a.x-b.x,a.y-b.y)}:null;};
  const markButton=()=>{usedModifiers();};
  function stopDrag(){if(dragging){dragging=false;options.button(0,false);}visual();}
  function movePointer(dx:number,dy:number){
    const size=options.dimensions();if(size.width<=0||size.height<=0)return;
    position={x:Math.max(0,Math.min(1,position.x+dx/size.width)),y:Math.max(0,Math.min(1,position.y+dy/size.height))};
    options.move(position);
  }
  function applyPair(next: TouchPairSample) {
    if (!live() || gesture === "ignore" || !last || !origin) return;
    const from=gesture==="pending"?origin:last;
    if(Math.abs(next.distance-origin.distance)>Math.max(8,origin.distance*.06))gesture="pinch";
    if(gesture==="pending"&&Math.hypot(next.x-origin.x,next.y-origin.y)>4)gesture=options.zoomActive?.()?"pan":"scroll";
    if(gesture==="pinch") {
      moved=true;if(from.distance>0)options.zoom(next.distance/from.distance);
      if(options.zoomActive?.())options.pan?.(next.x-from.x,next.y-from.y);
    } else if(gesture==="pan") { moved=true;options.pan?.(next.x-from.x,next.y-from.y); }
    else if(gesture==="scroll") {moved=true;usedModifiers();options.scroll(from.x-next.x,from.y-next.y);}
    last=next;
  }
  function flushPair() {
    if(pairFrame!==null)cancelAnimationFrame(pairFrame);
    pairFrame=null;
    const sample=contacts.size===2?centroid():null;
    if(sample)applyPair(sample);
  }
  function finishPair() {
    if(gesture==="pending"&&!moved&&performance.now()-began<350){markButton();options.button(2,true);options.button(2,false);}
    if(gesture!=="pan"&&gesture!=="pinch")consumeLatches();
    gesture="single";last=origin=null;moved=false;
  }
  const nativePair=createTouchPair(pad,{
    enabled:live,
    start(sample,first){
      cancelHold();stopDrag();lastTap=-Infinity;
      if(pairFrame!==null)cancelAnimationFrame(pairFrame);pairFrame=null;
      if(first&&!contacts.size){began=performance.now();moved=false;}
      gesture=sample?"pending":"ignore";last=origin=sample;
      if(!sample)moved=true;
    },
    move:applyPair,
    end(cancelled){
      if(cancelled){cancel();return;}
      finishPair();
      const ids=[...contacts.keys()];contacts.clear();
      for(const id of ids)if(pad.hasPointerCapture?.(id))pad.releasePointerCapture(id);
      visual();
    },
  });
  listen(pad,"pointerdown",(event:PointerEvent)=>{
    event.preventDefault();event.stopPropagation();if(!live()||event.button!==0)return;
    options.engage?.();
    contacts.set(event.pointerId,{x:event.clientX,y:event.clientY,start:{x:event.clientX,y:event.clientY}});
    pad.setPointerCapture?.(event.pointerId);
    if(nativePair.active())return;
    if(contacts.size===1){
      began=performance.now();moved=false;gesture="single";position=options.position();
      if(began-lastTap<300){markButton();dragging=options.button(0,true);}
      else hold=setTimeout(()=>{hold=null;if(live()&&contacts.size===1&&!moved){markButton();dragging=options.button(0,true);visual();}},350);
    } else {
      cancelHold();stopDrag();lastTap=-Infinity;
      gesture=contacts.size===2?"pending":"ignore";last=origin=centroid();
    }
    visual();
  });
  listen(pad,"pointermove",(event:PointerEvent)=>{
    const point=contacts.get(event.pointerId);if(!point||!live())return;
    event.preventDefault();event.stopPropagation();
    const dx=event.clientX-point.x,dy=event.clientY-point.y;point.x=event.clientX;point.y=event.clientY;
    if(nativePair.active())return;
    if(gesture==="single"){
      if(Math.hypot(point.x-point.start.x,point.y-point.start.y)>3){moved=true;cancelHold();}
      movePointer(dx,dy);
    } else if(contacts.size===2 && gesture!=="ignore" && last && origin){
      if(pairFrame===null)pairFrame=requestAnimationFrame(flushPair);
    }
  });
  listen(pad,"pointerup",(event:PointerEvent)=>{
    if(!contacts.has(event.pointerId))return;
    event.preventDefault();event.stopPropagation();cancelHold();
    if(nativePair.active()){
      contacts.delete(event.pointerId);
      if(pad.hasPointerCapture?.(event.pointerId))pad.releasePointerCapture(event.pointerId);
      return;
    }
    if(pairFrame!==null)flushPair();
    contacts.delete(event.pointerId);
    if(gesture==="single"){
      if(dragging){stopDrag();lastTap=-Infinity;}
      else if(!moved&&performance.now()-began<350){markButton();options.button(0,true);options.button(0,false);lastTap=performance.now();}
      consumeLatches();
    } else if(!contacts.size){
      finishPair();
    }
    if(pad.hasPointerCapture?.(event.pointerId))pad.releasePointerCapture(event.pointerId);
    if(!contacts.size){gesture="single";last=origin=null;moved=false;}
  });
  listen(pad,"contextmenu",(event:Event)=>event.preventDefault());
  listen(pad,"pointercancel",(event:PointerEvent)=>{if(nativePair.active())contacts.delete(event.pointerId);else cancel();});
  listen(pad,"lostpointercapture",(event:PointerEvent)=>{if(nativePair.active())contacts.delete(event.pointerId);else if(contacts.has(event.pointerId))cancel();});
  listen(pad,"wheel",(event:WheelEvent)=>{if(!live())return;event.preventDefault();if(!event.ctrlKey&&!event.metaKey)options.scroll(event.deltaX,event.deltaY);});
  function reset(){
    nativePair.reset();
    if(pairFrame!==null)cancelAnimationFrame(pairFrame);pairFrame=null;
    if(!keyTouches.size&&!contacts.size&&!down.size&&!latched.size&&!caps&&!dragging&&lastTap===-Infinity)return;
    cancelHold();const oldKeys=[...keyTouches],oldContacts=[...contacts.keys()];
    keyTouches.clear();contacts.clear();down.clear();latched.clear();caps=false;dragging=false;gesture="single";last=origin=null;lastTap=-Infinity;
    for(const[id,key]of oldKeys)if(key.button.hasPointerCapture?.(id))key.button.releasePointerCapture(id);
    for(const id of oldContacts)if(pad!.hasPointerCapture?.(id))pad!.releasePointerCapture(id);
    visual();
  }
  function cancel(){const codes=[...down],button=dragging,locked=caps;reset();for(const code of codes)options.key(code,false);if(button)options.button(0,false);if(locked)options.release?.();}
  listen(window,"blur",cancel);
  listen(document,"visibilitychange",()=>{if(document.hidden)cancel();});
  return {reset,cancel,touching:()=>contacts.size>0||nativePair.active(),dispose(){cancel();nativePair.dispose();lifecycle.abort();}};
}
