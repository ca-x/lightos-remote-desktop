import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createMobileControls, mobileResolution } from "../web/src/mobile";

let controls: ReturnType<typeof createMobileControls>;
let input: HTMLTextAreaElement;
let toggle: HTMLButtonElement;
let viewport: HTMLElement;
let visual: EventTarget & { width: number; height: number; offsetTop: number; offsetLeft: number };
let commands: Record<string, unknown>[];
let texts: string[];
let chords: string[][];
let ready: boolean;
let menuOpen: boolean;
let audioAvailable: boolean;
let audioToggles: number;
let fullscreenToggles: number;
let rotations: number;
let zoomed: boolean;
let gestures: { ratio: number; from: {x:number;y:number}; to: {x:number;y:number} }[];

beforeEach(() => {
  vi.useFakeTimers();
  document.body.innerHTML = `<main id="viewport"><canvas></canvas><textarea></textarea><button id="fullscreen-exit" hidden></button><button id="fullscreen-rotate" hidden></button><nav><button id="keyboard"></button><button data-mobile-key="ControlLeft"></button><button data-mobile-key="ArrowUp"></button><button data-mobile-text="/"></button><button data-mobile-action="audio"></button><button data-mobile-action="fullscreen"></button><button data-mobile-action="menu"></button></nav></main>`;
  viewport = document.querySelector("main")!;
  input = document.querySelector("textarea")!;
  toggle = document.querySelector("#keyboard")!;
  audioAvailable = true; audioToggles = fullscreenToggles = rotations = 0;
  commands = []; texts = []; chords = []; zoomed = false; gestures = []; ready = true; menuOpen = false;
  visual = Object.assign(new EventTarget(), { width: 390, height: 844, offsetTop: 0, offsetLeft: 0 });
  vi.stubGlobal("visualViewport", visual);
  vi.stubGlobal("innerHeight", 844);
  vi.stubGlobal("matchMedia", () => Object.assign(new EventTarget(), { matches: true }));
  controls = createMobileControls({
    canvas: document.querySelector("canvas")!, viewport, input, bar: document.querySelector("nav")!, toggle,
    fullscreenExit: document.querySelector('#fullscreen-exit')!, exitFullscreen: () => controls.setFullscreen(false),
    fullscreenRotate: document.querySelector('#fullscreen-rotate')!, rotateScreen: () => { rotations++; },
    audioAvailable: () => audioAvailable, toggleAudio: () => { audioToggles++; }, toggleFullscreen: () => { fullscreenToggles++; },
    zoom: { active: () => zoomed, gesture: (from, to, ratio) => { gestures.push({from, to, ratio}); zoomed = true; } },
    ready: () => ready, send: command => { commands.push(command); return true; },
    text: value => texts.push(value), chord: value => chords.push(value), move: () => {}, tools: (open) => { menuOpen = open; }, layout: () => {}, translate: value => value,
  });
});

it("fullscreen closes the keyboard and reveals its floating toggle for two seconds per tap",()=>{
  toggle.click();expect(document.activeElement).toBe(input);
  controls.setFullscreen(true);
  const button=document.querySelector<HTMLButtonElement>('#fullscreen-exit')!;
  expect(document.activeElement).not.toBe(input);expect(controls.keyboardActive()).toBe(false);
  expect(document.querySelector('nav')!.hidden).toBe(true);expect(controls.bounds()!.bar).toBe(0);
  expect(button.hidden).toBe(false);
  vi.advanceTimersByTime(1999);expect(button.hidden).toBe(false);
  const touch=new Event('pointerdown',{bubbles:true,cancelable:true});document.dispatchEvent(touch);
  expect(touch.defaultPrevented).toBe(false);
  vi.advanceTimersByTime(1999);expect(button.hidden).toBe(false);
  vi.advanceTimersByTime(1);expect(button.hidden).toBe(true);
  document.dispatchEvent(new Event('pointerdown'));expect(button.hidden).toBe(false);
  controls.setFullscreen(false);expect(button.hidden).toBe(true);expect(document.querySelector('nav')!.hidden).toBe(false);
  vi.advanceTimersByTime(2500);expect(button.hidden).toBe(true);
});

it("fullscreen exit restores controls without opening the IME, even when disconnected",()=>{
  for (const connected of [true, false]) {
    ready=connected; controls.setFullscreen(true); controls.update();
    const button=document.querySelector<HTMLButtonElement>('#fullscreen-exit')!;
    expect(button.disabled).toBe(false);
    button.click();
    expect(controls.isFullscreen()).toBe(false);
    expect(document.activeElement).not.toBe(input);
    expect(document.querySelector('nav')!.hidden).toBe(false);
    const menu=document.querySelector<HTMLButtonElement>('[data-mobile-action=menu]')!;
    expect(menu.disabled).toBe(false);menu.click();expect(menuOpen).toBe(true);
  }
});

afterEach(() => { controls.dispose(); vi.useRealTimers(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

function pointer(id: number, x: number, y: number) {
  return { pointerId: id, pointerType: "touch", clientX: x, clientY: y, preventDefault() {} } as PointerEvent;
}

function inputEvent(type: string, data: string | null, composing = false) {
  input.dispatchEvent(new InputEvent("input", { inputType: type, data, isComposing: composing, bubbles: true }));
}

it("uses high density capture without shrinking desktop logical size or exceeding encoding limits", () => {
  expect(mobileResolution(390,739,2)).toEqual({width:780,height:1476,scale:2});
  expect(mobileResolution(390,739,3)).toEqual({width:970,height:1840,scale:2.5});
  expect(mobileResolution(360,535,3)).toEqual({width:1080,height:1602,scale:3});
  expect(mobileResolution(390,739,1.5)).toEqual({width:582,height:1104,scale:1.5});
  expect(mobileResolution(1000,1400,2)).toEqual({width:1500,height:2100,scale:1.5});
  expect(mobileResolution(390,739,1)).toEqual({width:390,height:738,scale:1});
  for(const ratio of [1,1.5,2,3]) {
    const result=mobileResolution(390,739,ratio);
    expect(result.width%2).toBe(0);expect(result.height%2).toBe(0);
    expect(result.width/result.scale).toBeGreaterThanOrEqual(388);
    expect(result.height/result.scale).toBeGreaterThanOrEqual(736);
  }
});

it("toggles the IME input and docks the bar using visual viewport height and pan", () => {
  toggle.click();
  expect(document.activeElement).toBe(input);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  expect(controls.keyboardActive()).toBe(true);
  vi.advanceTimersByTime(20);
  expect(viewport.classList.contains("software-keyboard-visible")).toBe(false);
  visual.height = 500;
  visual.offsetTop = 40;
  visual.dispatchEvent(new Event("resize"));
  vi.advanceTimersByTime(20);
  expect(viewport.style.getPropertyValue("--keyboard-inset")).toBe("304px");
  expect(viewport.style.getPropertyValue("--mobile-height")).toBe("500px");
  expect(viewport.classList.contains("software-keyboard-visible")).toBe(true);
  toggle.click();
  expect(document.activeElement).not.toBe(input);
  expect(controls.keyboardActive()).toBe(true);
  vi.advanceTimersByTime(360);
  expect(controls.keyboardActive()).toBe(false);
});

it("sends committed Chinese exactly once across compositionend and following input", () => {
  toggle.click();
  input.dispatchEvent(new CompositionEvent("compositionstart", { data: "" }));
  input.value = "\u200b中文";
  inputEvent("insertCompositionText", "中文", true);
  expect(texts).toEqual([]);
  input.dispatchEvent(new CompositionEvent("compositionend", { data: "中文" }));
  input.value = "\u200b中文";
  vi.advanceTimersByTime(50);
  inputEvent("insertText", "中文");
  expect(texts).toEqual(["中文"]);
  vi.advanceTimersByTime(1);
  input.value = "\u200b🙂abc";
  inputEvent("insertText", "🙂abc");
  expect(texts).toEqual(["中文", "🙂abc"]);
});

it("consumes sticky modifiers once and preserves keyboard focus while touching shortcuts", () => {
  toggle.click();
  const ctrl = document.querySelector<HTMLButtonElement>('[data-mobile-key="ControlLeft"]')!;
  ctrl.click();
  expect(ctrl.getAttribute("aria-pressed")).toBe("true");
  document.querySelector<HTMLButtonElement>('[data-mobile-text="/"]')!.click();
  expect(chords).toEqual([["ControlLeft", "Slash"]]);
  expect(ctrl.getAttribute("aria-pressed")).toBe("false");
  expect(document.activeElement).toBe(input);
  const arrow = document.querySelector<HTMLButtonElement>('[data-mobile-key="ArrowUp"]')!;
  const dispatch = (name: string) => {
    const event = new Event(name, { bubbles: true, cancelable: true });
    Object.assign(event, { pointerId: 1, pointerType: "touch", clientX: 10, clientY: 10 });
    arrow.dispatchEvent(event);
  };
  dispatch("pointerdown"); dispatch("pointerup");
  arrow.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
  expect(chords).toEqual([["ControlLeft", "Slash"], ["ArrowUp"]]);
  expect(document.activeElement).toBe(input);
});

it("opens the menu after the pointer gesture so the popover does not close on the same click", () => {
  const button = document.createElement("button");
  button.dataset.mobileAction = "menu";
  document.querySelector("nav")!.appendChild(button);
  toggle.click();
  const event = (name: string) => {
    const e = new Event(name, { bubbles: true, cancelable: true });
    Object.assign(e, { pointerId: 1, clientX: 0, clientY: 0 });
    button.dispatchEvent(e);
  };
  event("pointerdown");event("pointerup");
  expect(document.activeElement).toBe(input);
  button.click();
  expect(document.activeElement).not.toBe(input);
});

it("maps deletion and enter from software keyboard beforeinput without inserting sentinel text", () => {
  toggle.click();
  for (const inputType of ["deleteContentBackward", "insertLineBreak"]) {
    const event = new InputEvent("beforeinput", { inputType, bubbles: true, cancelable: true });
    input.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  }
  expect(chords).toEqual([["Backspace"], ["Enter"]]);
  expect(texts).toEqual([]);
});

it("distinguishes tap, drag, and hold without opening the keyboard", () => {
  controls.handlePointer(pointer(1, 10, 10), "down");
  controls.handlePointer(pointer(1, 10, 10), "up");
  expect(commands).toEqual([{ type: "button", button: 0, pressed: true }, { type: "button", button: 0, pressed: false }]);
  commands.length = 0;
  controls.handlePointer(pointer(2, 10, 10), "down");
  controls.handlePointer(pointer(2, 30, 40), "move");
  controls.handlePointer(pointer(2, 30, 40), "cancel");
  expect(commands).toEqual([{ type: "button", button: 0, pressed: true }, { type: "button", button: 0, pressed: false }]);
  commands.length = 0;
  controls.handlePointer(pointer(3, 10, 10), "down");
  vi.advanceTimersByTime(510);
  controls.handlePointer(pointer(3, 10, 10), "up");
  expect(commands).toEqual([{ type: "button", button: 2, pressed: true }, { type: "button", button: 2, pressed: false }]);
  expect(document.activeElement).not.toBe(input);
});

it("scrolls with two fingers and never turns the last lifted finger into a click", () => {
  controls.handlePointer(pointer(1, 10, 100), "down");
  controls.handlePointer(pointer(2, 100, 100), "down");
  controls.handlePointer(pointer(1, 10, 90), "move");
  controls.handlePointer(pointer(2, 100, 90), "move");
  controls.handlePointer(pointer(1, 10, 90), "up");
  controls.handlePointer(pointer(2, 100, 90), "up");
  expect(commands.filter(v=>v.type==="scroll").reduce((sum,v)=>sum + Number(v.y),0)).toBe(10);
  expect(commands.filter(v=>v.type==="button")).toEqual([]);
});

it("reset releases a drag, clears modifiers and prevents delayed hold/repeat actions", () => {
  controls.handlePointer(pointer(1, 10, 10), "down");
  controls.handlePointer(pointer(1, 40, 10), "move");
  document.querySelector<HTMLButtonElement>('[data-mobile-key="ControlLeft"]')!.click();
  controls.reset();
  const count = commands.length;
  vi.advanceTimersByTime(1000);
  expect(commands.length).toBe(count);
  expect(commands.at(-1)).toEqual({ type: "button", button: 0, pressed: false });
  expect(document.querySelector('[data-mobile-key="ControlLeft"]')!.getAttribute("aria-pressed")).toBe("false");
});

it("excludes host safe areas from the area used for capture and toolbar placement", () => {
  viewport.style.padding = "47px 20px 0 16px";
  vi.spyOn(document.querySelector("nav")!, "getBoundingClientRect").mockReturnValue({ height: 111 } as DOMRect);
  expect(controls.bounds()).toEqual({ top: 47, left: 16, width: 354, height: 797, bar: 111 });
  visual.width = 844; visual.height = 390;
  viewport.style.padding = "0px 47px 0 47px";
  expect(controls.bounds()).toEqual({ top: 0, left: 47, width: 750, height: 390, bar: 111 });
});

it("pinches and pans only the local picture without scrolling or changing keyboard and sticky keys", () => {
  toggle.click();
  const ctrl = document.querySelector<HTMLButtonElement>('[data-mobile-key="ControlLeft"]')!;
  ctrl.click();
  controls.handlePointer(pointer(1,100,100),"down");
  controls.handlePointer(pointer(2,200,100),"down");
  // Accumulate small motion until it is distinguishable from a scroll.
  for (let x=98;x>=88;x-=2) controls.handlePointer(pointer(1,x,100),"move");
  controls.handlePointer(pointer(2,220,100),"move");
  controls.handlePointer(pointer(1,88,100),"up");
  controls.handlePointer(pointer(2,220,100),"up");
  expect(gestures.length).toBeGreaterThan(0);
  expect(gestures.reduce((scale,g)=>scale*g.ratio,1)).toBeCloseTo(1.32);
  expect(commands.filter(c=>c.type==="scroll"||c.type==="button")).toEqual([]);
  expect(document.activeElement).toBe(input);
  expect(ctrl.getAttribute("aria-pressed")).toBe("true");
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  gestures=[];
  controls.handlePointer(pointer(3,100,100),"down");
  controls.handlePointer(pointer(4,200,100),"down");
  controls.handlePointer(pointer(3,100,110),"move");
  controls.handlePointer(pointer(4,200,110),"move");
  controls.handlePointer(pointer(3,100,110),"up");
  controls.handlePointer(pointer(4,200,110),"up");
  expect(gestures.every(g=>g.ratio===1)).toBe(true);
  expect(commands.filter(c=>c.type==="scroll"||c.type==="button")).toEqual([]);
});

it("keeps the menu usable when a desktop has not become ready", () => {
  const menu = document.querySelector<HTMLButtonElement>('[data-mobile-action="menu"]')!;
  ready = false;
  controls.update();
  menu.click();
  expect(menuOpen).toBe(true);
  expect(menu.disabled).toBe(false);
  expect(toggle.disabled).toBe(true);
  document.querySelector<HTMLButtonElement>('[data-mobile-key="ArrowUp"]')!.click();
  expect(commands).toEqual([]);
  expect(chords).toEqual([]);
});

it("samples both fingers together so horizontal fullscreen panning does not pinch",()=>{
  zoomed=true;
  controls.handlePointer(pointer(1,100,100),'down');controls.handlePointer(pointer(2,200,100),'down');
  controls.handlePointer(pointer(2,230,100),'move');controls.handlePointer(pointer(1,130,100),'move');
  controls.handlePointer(pointer(1,130,100),'up');controls.handlePointer(pointer(2,230,100),'up');
  expect(gestures).toHaveLength(1);expect(gestures[0].ratio).toBe(1);
  expect(gestures[0].to.x-gestures[0].from.x).toBe(30);
  expect(commands.filter(c=>c.type==='scroll'||c.type==='button')).toEqual([]);
});


it("keeps rotation beside the exit lifecycle and offers direct audio/fullscreen actions",()=>{
  const rotate=document.querySelector<HTMLButtonElement>('#fullscreen-rotate')!;
  controls.setFullscreen(true);expect(rotate.hidden).toBe(false);rotate.click();expect(rotations).toBe(1);
  vi.advanceTimersByTime(2000);expect(rotate.hidden).toBe(true);
  document.dispatchEvent(new Event('pointerdown'));expect(rotate.hidden).toBe(false);
  controls.setFullscreen(false);expect(rotate.hidden).toBe(true);
  const audio=document.querySelector<HTMLButtonElement>('[data-mobile-action=audio]')!;
  const fullscreen=document.querySelector<HTMLButtonElement>('[data-mobile-action=fullscreen]')!;
  audio.click();fullscreen.click();expect(audioToggles).toBe(1);expect(fullscreenToggles).toBe(1);expect(menuOpen).toBe(false);
  audioAvailable=false;controls.update();expect(audio.disabled).toBe(true);
  ready=false;controls.update();expect(fullscreen.disabled).toBe(false);fullscreen.click();expect(fullscreenToggles).toBe(2);
});


it("keeps direct picture panning when native touch events continue after pointer cancellation",()=>{
 zoomed=true;const canvas=document.querySelector('canvas')!;
 const touch=(type:string,points:{identifier:number;clientX:number;clientY:number}[])=>{
   const e=new Event(type,{bubbles:true,cancelable:true});Object.assign(e,{targetTouches:points,touches:points});canvas.dispatchEvent(e);
 };
 controls.handlePointer(pointer(1,100,100),'down');controls.handlePointer(pointer(2,200,100),'down');
 touch('touchstart',[{identifier:11,clientX:100,clientY:100},{identifier:22,clientX:200,clientY:100}]);
 controls.handlePointer(pointer(1,100,100),'cancel');controls.handlePointer(pointer(2,200,100),'cancel');
 touch('touchmove',[{identifier:11,clientX:130,clientY:115},{identifier:22,clientX:230,clientY:115}]);
 touch('touchend',[]);
 expect(gestures).toHaveLength(1);expect(gestures[0].ratio).toBe(1);
 expect(gestures[0].to.x-gestures[0].from.x).toBe(30);expect(gestures[0].to.y-gestures[0].from.y).toBe(15);
 expect(commands.filter(c=>c.type==='scroll'||c.type==='button')).toEqual([]);expect(controls.touching()).toBe(false);
});
