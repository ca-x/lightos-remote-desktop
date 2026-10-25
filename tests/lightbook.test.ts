import {beforeEach,afterEach,it,expect,vi} from "vitest";
import {createLightBookInput,supportsLightBook} from "../web/src/lightbook-input";

let input: NonNullable<ReturnType<typeof createLightBookInput>>;
let keys: [string,boolean][], buttons: [number,boolean][], moves: {x:number;y:number}[], scrolls: number[][];
let active: boolean, ready: boolean;
let releases: number;
let pad: HTMLElement;
function pointer(target:Element,type:string,id:number,x=0,y=0){
 const e=new Event(type,{bubbles:true,cancelable:true});Object.assign(e,{pointerId:id,button:0,clientX:x,clientY:y});target.dispatchEvent(e);
}
const key=(name:string)=>document.querySelector(`[data-lightbook-key="${name}"]`)!;
beforeEach(()=>{
 vi.useFakeTimers();
 document.body.innerHTML=`<nav><div id="lightbook-keys"><button data-lightbook-key="MetaLeft" aria-pressed="false">Super</button><button data-lightbook-key="ShiftLeft">Shift</button><button data-lightbook-key="CapsLock" aria-pressed="false">Caps</button><button data-lightbook-key="KeyA">A</button><button data-lightbook-key="Space">Space</button><button data-lightbook-key="ArrowLeft" data-lightbook-fn="Home">←</button><button data-lightbook-action="fn" aria-pressed="false">fn</button></div><div id="lightbook-trackpad"></div></nav>`;
 keys=[];buttons=[];moves=[];scrolls=[];releases=0;active=ready=true;pad=document.querySelector('#lightbook-trackpad')!;
 input=createLightBookInput({root:document.querySelector('nav')!,active:()=>active,ready:()=>ready,
  key:(code,pressed)=>{keys.push([code,pressed]);return true;},button:(b,p)=>{buttons.push([b,p]);return true;},move:p=>moves.push(p),position:()=>({x:.5,y:.5}),dimensions:()=>({width:400,height:200}),scroll:(x,y)=>scrolls.push([x,y]),zoom:()=>{},release:()=>{releases++;},})!;
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
it('pairs Fn navigation key releases with the actual resolved key',()=>{
 const fn=document.querySelector('[data-lightbook-action=fn]')!;
 pointer(fn,'pointerdown',1);pointer(key('ArrowLeft'),'pointerdown',2);pointer(fn,'pointerup',1);pointer(key('ArrowLeft'),'pointerup',2);
 expect(keys).toEqual([['Home',true],['Home',false]]);
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
