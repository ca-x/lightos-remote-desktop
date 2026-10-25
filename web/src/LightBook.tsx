import { Globe2, House, Menu } from "lucide-react";
import { useTranslation } from "react-i18next";

type Key = { code: string; label: string; units?: number; upper?: string; fn?: string; fnLabel?: string };
const key = (code: string, label: string, units = 1, upper?: string, fn?: string, fnLabel?: string): Key => ({ code, label, units, upper, fn, fnLabel });
const letters = (text: string) => Array.from(text, label => key(`Key${label}`, label));
const rows: Key[][] = [
  [key("Escape", "esc", 1.5), ...Array.from({length:12}, (_,i) => key(`F${i+1}`, `F${i+1}`)), key("Delete", "del", 1.5)],
  [key("Backquote", "`", 1, "~"), ...Array.from("1234567890", (n,i) => key(`Digit${n}`, n, 1, "!@#$%^&*()"[i], `F${i+1}`)), key("Minus", "−", 1, "_", "F11"), key("Equal", "=", 1, "+", "F12"), key("Backspace", "⌫", 2, undefined, "Delete", "del")],
  [key("Tab", "tab", 1.5), ...letters("QWERTYUIOP"), key("BracketLeft", "[", 1, "{"), key("BracketRight", "]", 1, "}"), key("Backslash", "\\", 1.5, "|")],
  [key("CapsLock", "caps lock", 1.75), ...letters("ASDFGHJKL"), key("Semicolon", ";", 1, ":"), key("Quote", "'", 1, '"'), key("Enter", "enter", 2.25)],
  [key("ShiftLeft", "shift", 2.25), ...letters("ZXCVBNM"), key("Comma", ",", 1, "<"), key("Period", ".", 1, ">"), key("Slash", "/", 1, "?"), key("ShiftRight", "shift", 2.75)],
];

export function LightBook({ active, ready }: { active: boolean; ready: boolean }) {
  const {t} = useTranslation();
  const renderKey = (item: Key) => <button key={item.code} type="button" className={`lightbook-key ${item.code.length > 3 && !item.code.startsWith("Key") && !item.code.startsWith("Digit") ? "lightbook-key-wide" : ""}`}
    data-lightbook-key={item.code} data-lightbook-fn={item.fn} style={{gridColumn:`span ${(item.units ?? 1)*4}`}}
    disabled={!ready} aria-label={item.code === "Space" ? t("lightbookSpace") : item.label} aria-pressed={/^(Control|Alt|Meta|Shift)|CapsLock/.test(item.code) ? false : undefined}>
    <span className="lightbook-legend">{item.upper && <small>{item.upper}</small>}<span>{item.label}</span></span>
    {item.fn && <span className="lightbook-fn-legend">{item.fnLabel ?? item.fn}</span>}
    {item.code === "CapsLock" && <i className="lightbook-caps-led" aria-hidden="true" />}
  </button>;
  return <section className="lightbook-deck" hidden={!active} aria-label="LightBook">
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
        <button type="button" className="lightbook-key lightbook-fn-key" data-lightbook-action="fn" style={{gridColumn:"span 4"}} aria-pressed={false} disabled={!ready}>fn</button>
        {[key("ControlLeft","Ctrl"),key("MetaLeft","Super",1.5),key("AltLeft","Alt",1.25),key("Space"," ",5.25),key("AltRight","Alt",1.25),key("MetaRight","Super",1.5)].map(renderKey)}
        <div className="lightbook-arrows" style={{gridColumn:"span 9"}}>
          {[["ArrowUp","↑","PageUp","pgup"],["ArrowLeft","←","Home","home"],["ArrowDown","↓","PageDown","pgdn"],["ArrowRight","→","End","end"]].map(([code,label,fn,fnLabel]) => <div key={code} className={`lightbook-arrow-${code.slice(5).toLowerCase()}`}>{renderKey(key(code,label,1,undefined,fn,fnLabel))}</div>)}
        </div>
      </div>
    </div>
    <div className="lightbook-palmrest">
      <div id="lightbook-trackpad" className="lightbook-trackpad" role="application" aria-label={t("lightbookTrackpad")} aria-disabled={!ready} title={t("lightbookTrackpadHelp")} tabIndex={active ? 0 : -1}/>
    </div>
  </section>;
}
