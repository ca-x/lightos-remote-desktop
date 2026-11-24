import { createTouchPair, type TouchPairSample } from "./touch-pair";
import { isMobilePlatform } from "./platform";

export function supportsLightBook() {
  if ((navigator.maxTouchPoints ?? 0) < 1) return false;
  return isMobilePlatform();
}

type Point = { x: number; y: number };
interface TrackpadOptions {
  active: () => boolean;
  ready: () => boolean;
  touchOnly?: boolean;
  rotated?: () => boolean;
  actionStarted?: () => void;
  actionEnded?: () => void;
  button: (button: number, pressed: boolean) => boolean;
  move: (point: Point) => void;
  position: () => Point;
  dimensions: () => { width: number; height: number };
  scroll: (x: number, y: number) => void;
  zoom?: (steps: number) => void;
  picture?: { active: () => boolean; gesture: (from: Point, to: Point, ratio: number) => void };
  clickFeedback?: () => void;
  engage?: () => void;
}
interface Options extends TrackpadOptions {
  root: HTMLElement;
  key: (code: string, pressed: boolean) => boolean;
  release?: () => void;
}
const zoomStep = Math.log(1.12);
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
  const live = () => options.active() && options.ready();
  const listen = (target: EventTarget, name: string, callback: (event: any) => void) => target.addEventListener(name, callback, {signal:lifecycle.signal,passive:false});
  const trackpad = createTrackpadInput(pad, { ...options, actionStarted: usedModifiers, actionEnded: consumeLatches });
  const held = (code:string) => [...keyTouches.values()].some(key=>key.code===code);
  function visual() {
    for (const button of keyButtons) {
      const code = button.dataset.lightbookKey;
      const pressed = code === "CapsLock" ? caps : !!code && down.has(code);
      button.classList.toggle("is-pressed", pressed);
      if (button.hasAttribute("aria-pressed")) button.setAttribute("aria-pressed",String(pressed));
    }
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
  function reset(){
    trackpad.reset();
    if(!keyTouches.size&&!down.size&&!latched.size&&!caps)return;
    const oldKeys=[...keyTouches];
    keyTouches.clear();down.clear();latched.clear();caps=false;
    for(const[id,key]of oldKeys)if(key.button.hasPointerCapture?.(id))key.button.releasePointerCapture(id);
    visual();
  }
  function cancel(){const codes=[...down],locked=caps;trackpad.cancel();reset();for(const code of codes)options.key(code,false);if(locked)options.release?.();}
  listen(window,"blur",cancel);
  listen(document,"visibilitychange",()=>{if(document.hidden)cancel();});
  return {reset,cancel,usedModifiers,consumeLatches,touching:trackpad.touching,dispose(){cancel();trackpad.dispose();lifecycle.abort();}};
}

export function createTrackpadInput(pad: HTMLElement, options: TrackpadOptions) {
  const lifecycle = new AbortController();
  const contacts = new Map<number, Point & { start: Point }>();
  let position = {x:.5,y:.5}, last: TouchPairSample | null = null;
  let origin: TouchPairSample | null = null;
  let zoomRemainder = 0;
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
  const cancelHold=()=>{if(hold!==null)clearTimeout(hold);hold=null;};
  const centroid = (): TouchPairSample | null => {
    const [a, b] = [...contacts.values()];
    return b ? { x: (a.x+b.x)/2, y: (a.y+b.y)/2, distance: Math.hypot(a.x-b.x, a.y-b.y), angle: Math.atan2(b.y-a.y,b.x-a.x) } : null;
  };
  const visual = () => pad.classList.toggle("is-dragging", dragging);
  function pressButton(button: number) {
    options.actionStarted?.();
    if (!options.button(button, true)) return false;
    options.clickFeedback?.();
    return true;
  }
  function tapButton(button: number) {
    if (pressButton(button)) options.button(button, false);
  }
  function stopDrag(){if(dragging){dragging=false;options.button(0,false);}visual();}
  function movePointer(dx:number,dy:number){
    const size=options.dimensions();if(size.width<=0||size.height<=0)return;
    const rotated = options.rotated?.();
    const x = rotated ? dy / size.height : dx / size.width;
    const y = rotated ? -dx / size.width : dy / size.height;
    const next = { x: Math.max(0, Math.min(1, position.x + x)), y: Math.max(0, Math.min(1, position.y + y)) };
    if (next.x === position.x && next.y === position.y) return;
    position = next;
    options.move(position);
  }
  function applyPair(next: TouchPairSample) {
    if (!live() || gesture === "ignore" || !last || !origin) return;
    const from = gesture === "pending" ? origin : last;
    const distanceChanged = Math.abs(next.distance - origin.distance) > Math.max(8, origin.distance * .06);
    if ((gesture === "pending" || gesture === "pan") && distanceChanged) gesture = "pinch";
    if (gesture === "pending" && Math.hypot(next.x - origin.x, next.y - origin.y) > 4) {
      gesture = options.picture?.active() ? "pan" : "scroll";
    }
    if (gesture === "pinch" && from.distance > 0 && next.distance > 0) {
      moved = true;
      const ratio = next.distance / from.distance;
      if (options.picture) {
        options.picture.gesture(from, next, ratio);
      } else {
        zoomRemainder += Math.log(ratio);
        const steps = Math.trunc(zoomRemainder / zoomStep);
        if (steps) {
          zoomRemainder -= steps * zoomStep;
          options.actionStarted?.();
          options.zoom?.(Math.max(-6, Math.min(6, steps)));
        }
      }
    } else if (gesture === "pan") {
      moved = true;
      options.picture?.gesture(from, next, 1);
    } else if (gesture === "scroll") {
      moved = true;
      options.actionStarted?.();
      options.scroll(from.x - next.x, from.y - next.y);
    } else if (next.angle !== undefined && origin.angle !== undefined) {
      // Rotation moves the fingers even if the centroid and distance stay put.
      if (Math.abs(Math.sin((next.angle - origin.angle) / 2)) * origin.distance > 4) moved = true;
    }
    last = next;
  }

  function beginPair(sample: TouchPairSample | null) {
    gesture = sample ? "pending" : "ignore";
    last = origin = sample;
    zoomRemainder = 0;
    if (!sample) moved = true;
  }
  function flushPair() {
    if(pairFrame!==null)cancelAnimationFrame(pairFrame);
    pairFrame=null;
    const sample=contacts.size===2?centroid():null;
    if(sample)applyPair(sample);
  }
  function finishPair() {
    if (gesture === "pending" && !moved && performance.now()-began < 350) tapButton(2);
    if (gesture !== "pan" && (gesture !== "pinch" || !options.picture)) options.actionEnded?.();
    gesture="single";last=origin=null;moved=false;
  }
  const nativePair=createTouchPair(pad,{
    enabled:live,
    start(sample,first){
      cancelHold();stopDrag();lastTap=-Infinity;
      if(pairFrame!==null)cancelAnimationFrame(pairFrame);pairFrame=null;
      if(first&&!contacts.size){began=performance.now();moved=false;}
      beginPair(sample);
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
    if (!options.active() || (options.touchOnly && event.pointerType !== "touch")) return;
    event.preventDefault();event.stopPropagation();if(!live()||event.button!==0)return;
    options.engage?.();
    contacts.set(event.pointerId,{x:event.clientX,y:event.clientY,start:{x:event.clientX,y:event.clientY}});
    pad.setPointerCapture?.(event.pointerId);
    if(nativePair.active())return;
    if(contacts.size===1){
      began=performance.now();moved=false;gesture="single";position=options.position();
      if (began-lastTap < 300) dragging = pressButton(0);
      else hold = setTimeout(() => {
        hold = null;
        if (live() && contacts.size === 1 && !moved) { dragging = pressButton(0); visual(); }
      }, 350);
    } else {
      cancelHold();stopDrag();lastTap=-Infinity;
      beginPair(contacts.size === 2 ? centroid() : null);
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
      else if(!moved&&performance.now()-began<350){tapButton(0);lastTap=performance.now();}
      options.actionEnded?.();
    } else if(!contacts.size){
      finishPair();
    }
    if(pad.hasPointerCapture?.(event.pointerId))pad.releasePointerCapture(event.pointerId);
    if(!contacts.size){gesture="single";last=origin=null;moved=false;}
  });
  listen(pad,"contextmenu",(event:Event)=>event.preventDefault());
  listen(pad,"pointercancel",(event:PointerEvent)=>{if(nativePair.active())contacts.delete(event.pointerId);else cancel();});
  listen(pad,"lostpointercapture",(event:PointerEvent)=>{if(nativePair.active())contacts.delete(event.pointerId);else if(contacts.has(event.pointerId))cancel();});
  listen(pad,"wheel",(event:WheelEvent)=>{if(options.touchOnly||!live())return;event.preventDefault();if(!event.ctrlKey&&!event.metaKey)options.scroll(event.deltaX,event.deltaY);});
  function reset(){
    nativePair.reset();
    if(pairFrame!==null)cancelAnimationFrame(pairFrame);pairFrame=null;
    cancelHold();
    const ids=[...contacts.keys()];contacts.clear();
    dragging=false;gesture="single";last=origin=null;lastTap=-Infinity;
    for(const id of ids)if(pad.hasPointerCapture?.(id))pad.releasePointerCapture(id);
    visual();
  }
  function cancel() {
    const button = dragging;
    const remoteGesture = gesture === "scroll" || (gesture === "pinch" && !options.picture);
    reset();
    if (button) options.button(0, false);
    if (button || remoteGesture) options.actionEnded?.();
  }
  listen(window,"blur",cancel);
  listen(document,"visibilitychange",()=>{if(document.hidden)cancel();});
  return {reset,cancel,touching:()=>contacts.size>0||nativePair.active(),dispose(){cancel();nativePair.dispose();lifecycle.abort();}};
}
