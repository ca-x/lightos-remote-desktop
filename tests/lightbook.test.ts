import {beforeEach,afterEach,it,expect,vi} from "vitest";
import {createLightBookInput,supportsLightBook} from "../web/src/lightbook-input";

let input: NonNullable<ReturnType<typeof createLightBookInput>>;
let keys: [string,boolean][], buttons: [number,boolean][], moves: {x:number;y:number}[], scrolls: number[][];
let active: boolean, ready: boolean;
let releases: number;
let zoomed: boolean;
let pans: number[][];
let pad: HTMLElement;
function pointer(target:Element,type:string,id:number,x=0,y=0){
 const e=new Event(type,{bubbles:true,cancelable:true});Object.assign(e,{pointerId:id,button:0,clientX:x,clientY:y});target.dispatchEvent(e);
}
const key=(name:string)=>document.querySelector(`[data-lightbook-key="${name}"]`)!;
beforeEach(()=>{
 vi.useFakeTimers();
 document.body.innerHTML=`<nav><div id="lightbook-keys"><button data-lightbook-key="MetaLeft" aria-pressed="false">Super</button><button data-lightbook-key="ShiftLeft">Shift</button><button data-lightbook-key="CapsLock" aria-pressed="false">Caps</button><button data-lightbook-key="KeyA">A</button><button data-lightbook-key="Space">Space</button><button data-lightbook-key="ArrowLeft">←</button></div><div id="lightbook-trackpad"></div></nav>`;
 keys=[];buttons=[];moves=[];scrolls=[];releases=0;zoomed=false;pans=[];active=ready=true;pad=document.querySelector('#lightbook-trackpad')!;
 input=createLightBookInput({root:document.querySelector('nav')!,active:()=>active,ready:()=>ready,
  key:(code,pressed)=>{keys.push([code,pressed]);return true;},button:(b,p)=>{buttons.push([b,p]);return true;},move:p=>moves.push(p),position:()=>({x:.5,y:.5}),dimensions:()=>({width:400,height:200}),scroll:(x,y)=>scrolls.push([x,y]),zoom:()=>{},zoomActive:()=>zoomed,pan:(x,y)=>pans.push([x,y]),release:()=>{releases++;},})!;
});
afterEach(()=>{input.dispose();vi.useRealTimers();vi.unstubAllGlobals();});
it('gates by mobile touch capability, retaining tablets with desktop browser identity',()=>{
 vi.stubGlobal('matchMedia',()=>({matches:false}));
 vi.stubGlobal('navigator',{maxTouchPoints:0,userAgent:'Desktop',platform:'Linux x86_64'});expect(supportsLightBook()).toBe(false);
 vi.stubGlobal('navigator',{maxTouchPoints:5,userAgent:'Android Tablet',platform:'Linux'});expect(supportsLightBook()).toBe(true);
 vi.stubGlobal('navigator',{maxTouchPoints:5,userAgent:'Macintosh',platform:'MacIntel'});expect(supportsLightBook()).toBe(true);
 vi.stubGlobal('navigator',{maxTouchPoints:5,userAgent:'Windows',platform:'Win32'});expect(supportsLightBook()).toBe(false);
 vi.stubGlobal('matchMedia',()=>({matches:true}));expect(supportsLightBook()).toBe(true);
});
it('supports held Super+Space and one-shot tapped modifiers without stuck keys',()=>{
 pointer(key('MetaLeft'),'pointerdown',1);pointer(key('Space'),'pointerdown',2);pointer(key('Space'),'pointerup',2);pointer(key('MetaLeft'),'pointerup',1);
 expect(keys).toEqual([['MetaLeft',true],['Space',true],['Space',false],['MetaLeft',false]]);
 keys=[];pointer(key('MetaLeft'),'pointerdown',3);pointer(key('MetaLeft'),'pointerup',3);
 expect(key('MetaLeft').getAttribute('aria-pressed')).toBe('true');
 pointer(key('KeyA'),'pointerdown',4);pointer(key('KeyA'),'pointerup',4);
 expect(keys).toEqual([['MetaLeft',true],['KeyA',true],['KeyA',false],['MetaLeft',false]]);
 expect(key('MetaLeft').getAttribute('aria-pressed')).toBe('false');
});
it('releases a held arrow independently when its modifier is released first',()=>{
 pointer(key('ShiftLeft'),'pointerdown',1);pointer(key('ArrowLeft'),'pointerdown',2);pointer(key('ShiftLeft'),'pointerup',1);pointer(key('ArrowLeft'),'pointerup',2);
 expect(keys).toEqual([['ShiftLeft',true],['ArrowLeft',true],['ShiftLeft',false],['ArrowLeft',false]]);
});
it('moves relative to the current pointer without dragging, taps left, and taps with two fingers right',()=>{
 pointer(pad,'pointerdown',1,10,10);pointer(pad,'pointermove',1,50,30);pointer(pad,'pointerup',1,50,30);
 expect(moves.at(-1)).toEqual({x:.6,y:.6});expect(buttons).toEqual([]);
 pointer(pad,'pointerdown',2,10,10);pointer(pad,'pointerup',2,10,10);expect(buttons).toEqual([[0,true],[0,false]]);
 vi.advanceTimersByTime(400);buttons=[];
 pointer(pad,'pointerdown',3,10,10);pointer(pad,'pointerdown',4,50,10);pointer(pad,'pointerup',3,10,10);pointer(pad,'pointerup',4,50,10);
 expect(buttons).toEqual([[2,true],[2,false]]);
});
it('scrolls with two fingers without a trailing click and cancels a held drag on blur',()=>{
 pointer(pad,'pointerdown',1,10,100);pointer(pad,'pointerdown',2,100,100);pointer(pad,'pointermove',1,10,80);pointer(pad,'pointermove',2,100,80);pointer(pad,'pointerup',1,10,80);pointer(pad,'pointerup',2,100,80);
 expect(scrolls.reduce((sum,s)=>sum+s[1],0)).toBe(20);expect(buttons).toEqual([]);
 pointer(pad,'pointerdown',3,10,10);vi.advanceTimersByTime(360);expect(buttons).toEqual([[0,true]]);
 pointer(key('ShiftLeft'),'pointerdown',4);
 window.dispatchEvent(new Event('blur'));
 expect(buttons).toEqual([[0,true],[0,false]]);expect(keys).toEqual([['ShiftLeft',true],['ShiftLeft',false]]);
 pointer(pad,'pointerup',3);pointer(key('ShiftLeft'),'pointerup',4);expect(buttons).toHaveLength(2);expect(keys).toHaveLength(2);
});
it('does not emit input while hidden or disconnected',()=>{
 active=false;pointer(key('KeyA'),'pointerdown',1);pointer(pad,'pointerdown',2);pointer(pad,'pointerup',2);
 active=true;ready=false;pointer(key('KeyA'),'pointerdown',3);pointer(pad,'pointerdown',4);pointer(pad,'pointerup',4);
 expect(keys).toEqual([]);expect(buttons).toEqual([]);expect(moves).toEqual([]);
});
it('clears the remote Caps lock along with its local indicator when leaving the keyboard',()=>{
 pointer(key('CapsLock'),'pointerdown',1);pointer(key('CapsLock'),'pointerup',1);
 expect(key('CapsLock').getAttribute('aria-pressed')).toBe('true');
 input.cancel();
 expect(releases).toBe(1);expect(key('CapsLock').getAttribute('aria-pressed')).toBe('false');
 input.cancel();expect(releases).toBe(1);
});

it('suppresses native touch selection on the trackpad without losing tap-hold dragging',()=>{
 const touch=()=>{const e=new Event('touchstart',{bubbles:true,cancelable:true});Object.assign(e,{targetTouches:[{identifier:1,clientX:20,clientY:20}]});pad.dispatchEvent(e);return e;};
 expect(touch().defaultPrevented).toBe(true);
 pointer(pad,'pointerdown',1,20,20);pointer(pad,'pointerup',1,20,20);
 expect(touch().defaultPrevented).toBe(true);
 pointer(pad,'pointerdown',2,20,20);vi.advanceTimersByTime(500);pointer(pad,'pointermove',2,50,40);pointer(pad,'pointerup',2,50,40);
 expect(buttons).toEqual([[0,true],[0,false],[0,true],[0,false]]);
 expect(moves.at(-1)).toEqual({x:.575,y:.6});
 const outside=new Event('touchstart',{bubbles:true,cancelable:true});document.body.dispatchEvent(outside);expect(outside.defaultPrevented).toBe(false);
 input.dispose();expect(touch().defaultPrevented).toBe(false);
});


it('pans a magnified picture from the trackpad without scrolling remote content or clicking',()=>{
 zoomed=true;
 pointer(pad,'pointerdown',1,50,100);pointer(pad,'pointerdown',2,150,100);
 pointer(pad,'pointermove',1,70,115);pointer(pad,'pointermove',2,170,115);
 pointer(pad,'pointerup',1,70,115);pointer(pad,'pointerup',2,170,115);
 expect(pans.reduce((sum,p)=>sum+p[0],0)).toBe(20);expect(pans.reduce((sum,p)=>sum+p[1],0)).toBe(15);
 expect(scrolls).toEqual([]);expect(buttons).toEqual([]);
});


it('keeps native multi-touch pan after pointer cancellation and ends without a phantom click',()=>{
 zoomed=true;
 const touch=(type:string,points:number[][])=>{const e=new Event(type,{bubbles:true,cancelable:true});Object.assign(e,{targetTouches:points.map(([identifier,clientX,clientY])=>({identifier,clientX,clientY}))});pad.dispatchEvent(e);};
 pointer(pad,'pointerdown',1,50,100);pointer(pad,'pointerdown',2,150,100);
 touch('touchstart',[[11,50,100],[22,150,100]]);
 pointer(pad,'pointercancel',1);pointer(pad,'pointercancel',2);
 touch('touchmove',[[11,70,115],[22,170,115]]);touch('touchend',[[22,170,115]]);touch('touchend',[]);
 expect(pans).toEqual([[20,15]]);expect(scrolls).toEqual([]);expect(buttons).toEqual([]);expect(input.touching()).toBe(false);
});

it('native two-finger taps right-click once and cancellation drops queued gestures',()=>{
 const touch=(type:string,points:number[][])=>{const e=new Event(type,{bubbles:true,cancelable:true});Object.assign(e,{targetTouches:points.map(([identifier,clientX,clientY])=>({identifier,clientX,clientY}))});pad.dispatchEvent(e);};
 pointer(pad,'pointerdown',1,50,100);pointer(pad,'pointerdown',2,150,100);touch('touchstart',[[11,50,100],[22,150,100]]);
 pointer(pad,'pointerup',1,50,100);touch('touchend',[[22,150,100]]);pointer(pad,'pointerup',2,150,100);touch('touchend',[]);
 expect(buttons).toEqual([[2,true],[2,false]]);buttons=[];
 touch('touchstart',[[11,50,100],[22,150,100]]);touch('touchmove',[[11,70,115],[22,170,115]]);touch('touchcancel',[]);
 vi.advanceTimersByTime(32);expect(scrolls).toEqual([]);expect(buttons).toEqual([]);expect(input.touching()).toBe(false);
});
