import { Ampersand, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Asterisk, AtSign, ChevronUp, Delete, DollarSign, Equal, Globe2, Hash, House, Menu, Minus, Percent, Plus } from "lucide-react";
import { useLayoutEffect, useRef, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { createLightBookLegendLayout } from "./lightbook-legend-layout";

type Key = { code: string; label: string; units?: number; upper?: string };
const key = (code: string, label: string, units = 1, upper?: string): Key => ({ code, label, units, upper });
const letters = (text: string) => Array.from(text, label => key(`Key${label}`, label));
// Punctuation has very different font baselines (comma, quote, caret, etc.).
// Give every mark an optically centered drawing inside the same legend slot.
const glyphs = { "^": ChevronUp, "*": Asterisk, "_": Minus, "−": Minus, "+": Plus, "=": Equal, "@": AtSign, "#": Hash, "$": DollarSign, "%": Percent, "&": Ampersand, "⌫": Delete, "↑": ArrowUp, "←": ArrowLeft, "↓": ArrowDown, "→": ArrowRight };
const marks: Record<string, { path?: string; dots?: [number, number][] }> = {
  ",": { path: "M9 6.5c0 2-1 3.5-2.5 4", dots: [[9, 6.5]] },
  ".": { dots: [[8, 8]] },
  ":": { dots: [[8, 5], [8, 11]] },
  ";": { path: "M9 9c0 2-1 3-2.5 3.5", dots: [[9, 4.5], [9, 9]] },
  "'": { path: "M8 5v6" },
  '"': { path: "M5.5 5v6M10.5 5v6" },
  "`": { path: "m6 5 4 6" },
  "~": { path: "M2 8c2-4 4-4 6 0s4 4 6 0" },
  "<": { path: "m11 3-6 5 6 5" },
  ">": { path: "m5 3 6 5-6 5" },
  "/": { path: "m4 13 8-10" },
  "\\": { path: "m4 3 8 10" },
  "|": { path: "M8 2.5v11" },
  "[": { path: "M11 3H5v10h6" },
  "]": { path: "M5 3h6v10H5" },
  "{": { path: "M11 2H9Q7 2 7 4v2q0 2-2 2 2 0 2 2v2q0 2 2 2h2" },
  "}": { path: "M5 2h2q2 0 2 2v2q0 2 2 2-2 0-2 2v2q0 2-2 2H5" },
  "(": { path: "M10 2C4 5 4 11 10 14" },
  ")": { path: "M6 2c6 3 6 9 0 12" },
  "!": { path: "M8 2.5v7", dots: [[8, 12.5]] },
  "?": { path: "M4.5 5a3.5 3.5 0 0 1 7 0c0 2-3.5 2-3.5 5", dots: [[8, 13.5]] },
};
function legend(value: string) {
  const mark = marks[value];
  if (mark) return <svg className="lightbook-key-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {mark.path && <path d={mark.path}/>}{mark.dots?.map(([cx, cy], i) => <circle key={i} cx={cx} cy={cy} r={1} fill="currentColor" stroke="none"/>)}
  </svg>;
  const Glyph = glyphs[value as keyof typeof glyphs];
  return Glyph ? <Glyph className="lightbook-key-glyph" viewBox={"^*_−+=↑←↓→".includes(value) ? "4 4 16 16" : "0 0 24 24"} strokeWidth={1.8} aria-hidden="true" />
    : <span className="lightbook-legend-text">{value}<i className="lightbook-baseline" aria-hidden="true"/></span>;
}
const rows: Key[][] = [
  [key("Escape", "Esc", 1.5), ...Array.from({length:12}, (_,i) => key(`F${i+1}`, `F${i+1}`)), key("Delete", "Del", 1.5)],
  [key("Backquote", "`", 1, "~"), ...Array.from("1234567890", (n,i) => key(`Digit${n}`, n, 1, "!@#$%^&*()"[i])), key("Minus", "−", 1, "_"), key("Equal", "=", 1, "+"), key("Backspace", "⌫", 2)],
  [key("Tab", "Tab", 1.5), ...letters("QWERTYUIOP"), key("BracketLeft", "[", 1, "{"), key("BracketRight", "]", 1, "}"), key("Backslash", "\\", 1.5, "|")],
  [key("CapsLock", "Caps Lock", 1.75), ...letters("ASDFGHJKL"), key("Semicolon", ";", 1, ":"), key("Quote", "'", 1, '"'), key("Enter", "Enter", 2.25)],
  [key("ShiftLeft", "Shift", 2.25), ...letters("ZXCVBNM"), key("Comma", ",", 1, "<"), key("Period", ".", 1, ">"), key("Slash", "/", 1, "?"), key("ShiftRight", "Shift", 2.75)],
];

export function LightBook({ active, ready }: { active: boolean; ready: boolean }) {
  const {t} = useTranslation();
  const deck = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (active && deck.current) return createLightBookLegendLayout(deck.current);
  }, [active]);
  const renderKey = (item: Key) => <button key={item.code} type="button" className={`lightbook-key ${!item.upper && item.code.length > 3 && !item.code.startsWith("Key") && !item.code.startsWith("Digit") ? "lightbook-key-wide" : ""}`}
    data-lightbook-key={item.code} style={{gridColumn:`span ${(item.units ?? 1)*4}`, "--lightbook-key-units":item.units ?? 1, "--lightbook-label-limit":Math.max(1,item.label.length*.6)} as CSSProperties}
    disabled={!ready} aria-label={item.code === "Space" ? t("lightbookSpace") : item.label} aria-pressed={/^(Control|Alt|Meta|Shift)|CapsLock/.test(item.code) ? false : undefined}>
    <span className="lightbook-legend">{item.upper && <small aria-hidden="true">{legend(item.upper)}</small>}<span>{legend(item.label)}</span></span>
    {item.code === "CapsLock" && <i className="lightbook-caps-led" aria-hidden="true" />}
  </button>;
  return <section ref={deck} className="lightbook-deck" hidden={!active} aria-label="LightBook">
    <div className="lightbook-controls">
      <span className="lightbook-brand">LightBook</span>
      <div>
        <button type="button" className="lightbook-tool" data-mobile-action="keyboard" disabled={!ready} aria-label={t("showKeyboard")} title={t("showKeyboard")} aria-pressed={false}><Globe2 size={17}/></button>
        <button type="button" className="lightbook-tool" data-mobile-action="menu" aria-label={t("mobileMenu")} title={t("mobileMenu")}><Menu size={17}/></button>
        <a className="lightbook-tool" href="/" aria-label={t("mobileHome")} title={t("mobileHome")}><House size={17}/></a>
      </div>
    </div>
    <div id="lightbook-keys" className="lightbook-keys" role="group" aria-label={t("lightbookKeyboard")}>
      {rows.map((row,i) => <div className={`lightbook-key-row ${i===0 ? "lightbook-function-row" : ""}`} key={i}>{row.map(renderKey)}</div>)}
      <div className="lightbook-key-row lightbook-bottom-row">
        {[key("ControlLeft","Ctrl"),key("MetaLeft","Super",1.5),key("AltLeft","Alt",1.25),key("Space"," ",5.5),key("AltRight","Alt",1.25),key("MetaRight","Super",1.5)].map(item => renderKey(item))}
        <div className="lightbook-arrows" style={{gridColumn:"span 12"}}>
          {[["ArrowUp","↑"],["ArrowLeft","←"],["ArrowDown","↓"],["ArrowRight","→"]].map(([code,label]) => <div key={code} className={`lightbook-arrow-${code.slice(5).toLowerCase()}`}>{renderKey(key(code,label))}</div>)}
        </div>
      </div>
    </div>
    <div className="lightbook-palmrest">
      <div id="lightbook-trackpad" className="lightbook-trackpad" role="application" aria-label={t("lightbookTrackpad")} aria-disabled={!ready} title={t("lightbookTrackpadHelp")} tabIndex={active ? 0 : -1}>
        <span className="lightbook-trackpad-hint" aria-hidden="true">{t("lightbookTrackpadHint")}</span>
      </div>
    </div>
  </section>;
}
