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

beforeEach(() => {
  vi.useFakeTimers();
  document.body.innerHTML = `<main id="viewport"><canvas></canvas><textarea></textarea><nav><button id="keyboard"></button><button data-mobile-key="ControlLeft"></button><button data-mobile-key="ArrowUp"></button><button data-mobile-text="/"></button></nav></main>`;
  viewport = document.querySelector("main")!;
  input = document.querySelector("textarea")!;
  toggle = document.querySelector("#keyboard")!;
  commands = []; texts = []; chords = [];
  visual = Object.assign(new EventTarget(), { width: 390, height: 844, offsetTop: 0, offsetLeft: 0 });
  vi.stubGlobal("visualViewport", visual);
  vi.stubGlobal("innerHeight", 844);
  vi.stubGlobal("matchMedia", () => Object.assign(new EventTarget(), { matches: true }));
  controls = createMobileControls({
    canvas: document.querySelector("canvas")!, viewport, input, bar: document.querySelector("nav")!, toggle,
    ready: () => true, send: command => { commands.push(command); return true; },
    text: value => texts.push(value), chord: value => chords.push(value), move: () => {}, tools: () => {}, layout: () => {}, translate: value => value,
  });
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
  expect(commands.filter(v=>v.type==="scroll")).toHaveLength(2);
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
