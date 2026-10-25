import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, Dialog, DialogBackdrop, DialogPanel, DialogTitle, Field, Input, Label, Select, Tab, TabGroup, TabList, TabPanel, TabPanels, Textarea } from "@headlessui/react";
import { Activity, Bot, Cable, Check, ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Clipboard, Expand, House, Info, Keyboard, Menu, Monitor, Palette, Settings2, Shrink, SlidersHorizontal, Volume2, VolumeX, X } from "lucide-react";
import logo from "./assets/logo.webp?inline";
import * as Menubar from "@radix-ui/react-menubar";
import { useTranslation } from "react-i18next";
import { createClient } from "./client";
import { RdpSettings } from "./RdpSettings";
import { McpSettings } from "./McpSettings";
import { MOBILE_ROWS, type MobileShortcut } from "./mobile";
import type { ControlState, SessionView, ToolbarEdge } from "./types";
import i18n, { translateClient } from "./i18n";
import { languageKey, languages, readPreference, resolveLanguage, resolveTheme, savePreference, themeKey, themes, type LanguagePreference, type ThemePreference } from "./preferences";

const initialControls: ControlState = {
  ready: false, wanted: true, audioAvailable: false, audioEnabled: false,
  resizeAvailable: true, desktopMode: "virtual", switchingDesktop: false,
  refreshingDesktops: false, choices: [], outputs: [],
};

type ToolMenuProps = { id: string; label: string; icon: React.ReactNode; edge: ToolbarEdge; children: React.ReactNode };

function ToolMenu({ id, label, icon, edge, children }: ToolMenuProps) {
  const side = edge === "right" ? "left" : edge === "left" ? "right" : edge === "bottom" ? "top" : "bottom";
  return <Menubar.Menu value={id}>
    <Menubar.Trigger id={id} className="toolbar-action menu-trigger" title={label} aria-label={label}>
      {icon}<span>{label}</span><ChevronDown size={12} aria-hidden="true" />
    </Menubar.Trigger>
    <Menubar.Portal>
      <Menubar.Content side={side} align="start" sideOffset={5} collisionPadding={12}
        className="session-menu" aria-label={label}
        onCloseAutoFocus={(event) => { if (document.activeElement?.id === "desktop") event.preventDefault(); }}>
        {children}
      </Menubar.Content>
    </Menubar.Portal>
  </Menubar.Menu>;
}

function Choice({ children, value }: { children: React.ReactNode; value: string }) {
  return <Menubar.RadioItem className="session-menu-item" value={value}><span className="menu-indicator"><Menubar.ItemIndicator><Check size={15} aria-hidden="true" /></Menubar.ItemIndicator></span>{children}</Menubar.RadioItem>;
}

function Submenu({ label, children, disabled = false }: { label: string; children: React.ReactNode; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  return <Menubar.Sub open={open} onOpenChange={setOpen}>
    <Menubar.SubTrigger className="session-menu-item" disabled={disabled}>{label}<ChevronRight className="menu-trailing" size={15} aria-hidden="true" /></Menubar.SubTrigger>
    <Menubar.Portal><Menubar.SubContent className="session-menu" sideOffset={3} collisionPadding={12}>{children}</Menubar.SubContent></Menubar.Portal>
  </Menubar.Sub>;
}

const initialView: SessionView = { mode: "fit", fullscreen: false, autoResolution: true, strategy: "auto", width: 1280, height: 720, keyboardCaptured: false, version: "", output: "" };

function Workspace() {
  const [open, setOpen] = useState(false);
  const [menu, setMenu] = useState("");
  const [utility, setUtility] = useState<"clipboard" | "keyboard" | null>(null);
  const [view, setView] = useState(initialView);
  const [status, setStatus] = useState<Record<string, string>>({});
  const close = (focusable?: HTMLElement) => { setOpen(false); setMenu(""); focusable?.focus({ preventScroll: true }); };
  const { t } = useTranslation();
  const [edge, setEdge] = useState<ToolbarEdge>("right");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState(0);
  const [controls, setControls] = useState(initialControls);
  const [language, setLanguage] = useState<LanguagePreference>(() => readPreference(languageKey, languages, "system"));
  const [theme, setTheme] = useState<ThemePreference>(() => readPreference(themeKey, themes, "system"));
  const lifecycle = useRef<ReturnType<typeof createClient> | null>(null);
  const popover = useRef({ open, close });
  const version = document.querySelector<HTMLMetaElement>('meta[name="lightos-version"]')?.content ?? "";
  useLayoutEffect(() => { popover.current = { open, close }; });

  useEffect(() => {
    let client: ReturnType<typeof createClient> | null = null;
    const initialization = requestAnimationFrame(() => {
      client = createClient({
        translate: translateClient,
        onControls: setControls,
        onSessionView: setView,
        onStatus: (id, text) => setStatus((current) => ({ ...current, [id]: text })),
        onToolbarDock: setEdge,
        isToolsOpen: () => popover.current.open,
        onToolsOpen(wanted, focus) {
          const trigger = document.getElementById("tools-toggle");
          const canvas = document.getElementById("desktop");
          if (wanted && !popover.current.open) trigger?.click();
          else if (!wanted && popover.current.open) popover.current.close(focus ? canvas ?? undefined : undefined);
          if (focus) requestAnimationFrame(() => {
            if (lifecycle.current !== client) return;
            (wanted ? document.getElementById("connection") : canvas)?.focus({ preventScroll: true });
          });
        },
      });
      lifecycle.current = client;
    });
    return () => { cancelAnimationFrame(initialization); lifecycle.current = null; client?.destroy(); };
  }, []);

  useEffect(() => {
    const apply = () => {
      void i18n.changeLanguage(resolveLanguage(language, navigator.languages ?? [navigator.language]));
      document.documentElement.lang = i18n.language === "zh" ? "zh-CN" : "en";
      document.title = `LightOS · ${i18n.t("title")}`;
      lifecycle.current?.refreshLanguage();
    };
    apply();
    window.addEventListener("languagechange", apply);
    return () => window.removeEventListener("languagechange", apply);
  }, [language]);

  useLayoutEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => { document.documentElement.dataset.theme = resolveTheme(theme, media.matches); };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === languageKey || event.key === null) setLanguage(readPreference(languageKey, languages, "system"));
      if (event.key === themeKey || event.key === null) setTheme(readPreference(themeKey, themes, "system"));
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);

  useLayoutEffect(() => { lifecycle.current?.layoutTools(); }, [open, controls, language]);
  useEffect(() => { if (!open) setMenu(""); }, [open]);
  const invoke = (id: string) => document.getElementById(id)?.click();
  const choose = (id: string, value: string) => {
    const node = document.getElementById(id) as HTMLSelectElement;
    node.value = value;
    node.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const showSettings = (tab = settingsTab) => { setMenu(""); setSettingsTab(tab); setSettingsOpen(true); };
  const showUtility = (mode: "clipboard" | "keyboard") => { setMenu(""); setUtility(mode); };
  const CollapseChevron = { top: ChevronUp, bottom: ChevronDown, left: ChevronLeft, right: ChevronRight }[edge];
  const ExpandChevron = { top: ChevronDown, bottom: ChevronUp, left: ChevronRight, right: ChevronLeft }[edge];
  const ToggleChevron = open ? CollapseChevron : ExpandChevron;
  const noResize = !controls.ready || !controls.resizeAvailable;
  const shortcut = (item: MobileShortcut) => {
    const names: Record<string, string> = { ArrowUp: t("arrowUp"), ArrowDown: t("arrowDown"), ArrowLeft: t("arrowLeft"), ArrowRight: t("arrowRight"), Backspace: t("backspace") };
    const label = item.modifier ? t("stickyKey", { key: item.label }) : names[item.key ?? ""] ?? item.label;
    return <button key={item.key ?? item.text} type="button" className="mobile-key" disabled={!controls.ready}
      data-mobile-key={item.key} data-mobile-modifiers={item.modifiers?.join(" ")} data-mobile-text={item.text}
      aria-label={label} title={label} aria-pressed={item.modifier ? false : undefined}>{item.label}</button>;
  };
  return <main id="viewport" className="fit" aria-label={t("viewport")} data-input="pointer"
    onPointerDownCapture={(event) => {
      event.currentTarget.dataset.input = "pointer"; document.documentElement.dataset.input = "pointer";
      if (menu && event.target === document.getElementById("desktop")) {
        event.preventDefault(); event.stopPropagation(); setMenu("");
        document.getElementById("desktop")?.focus({ preventScroll: true });
      }
    }}
    onKeyDownCapture={(event) => { event.currentTarget.dataset.input = "keyboard"; document.documentElement.dataset.input = "keyboard"; }}>
    <canvas id="desktop" width="1280" height="720" tabIndex={0} aria-label={t("canvas")} aria-describedby="help" />
    <img id="remote-cursor" className="remote-cursor" alt="" aria-hidden="true" hidden />
    <div id="desktop-recovery" className="desktop-recovery" role="status" hidden={controls.ready || controls.wanted}>
      <strong>{t("desktopNotConnected")}</strong><p>{t("desktopRecoveryHint")}</p>
    </div>
    <textarea id="mobile-input" className="mobile-input" tabIndex={-1} aria-label={t("mobileInput")}
      autoCapitalize="off" autoCorrect="off" autoComplete="off" spellCheck={false} inputMode="text" enterKeyHint="enter" />
    <nav id="mobile-shortcuts" className="mobile-shortcuts" aria-label={t("mobileShortcuts")}>
      <div className="mobile-shortcut-row" role="toolbar" aria-label={t("mobileRow1")}>
        <button id="mobile-keyboard" className="mobile-key mobile-keyboard" type="button" disabled={!controls.ready} aria-label={t("showKeyboard")} title={t("showKeyboard")} aria-pressed="false"><Keyboard size={18} aria-hidden="true" /></button>
        <div className="mobile-key-scroll">{MOBILE_ROWS[0].map(shortcut)}</div>
      </div>
      <div className="mobile-shortcut-row" role="toolbar" aria-label={t("mobileRow2")}>
        <button id="mobile-menu" className="mobile-key" type="button" data-mobile-action="menu" aria-label={t("mobileMenu")} title={t("mobileMenu")}><Menu size={18} aria-hidden="true" /></button>
        <a id="mobile-home" className="mobile-key mobile-home" href="/" aria-label={t("mobileHome")} title={t("mobileHome")}><House size={18} aria-hidden="true" /></a>
        <div className="mobile-key-scroll">{MOBILE_ROWS[1].map(shortcut)}</div>
      </div>
    </nav>
    <aside id="tools" aria-label={t("title")}>
      <button type="button" id="tools-toggle" className="tool-handle" title={t("drag")} aria-label={t(open ? "close" : "open")} aria-expanded={open} aria-controls="tool-panel" onClick={() => { setOpen(!open); setMenu(""); }}><ToggleChevron size={22} strokeWidth={2.25} aria-hidden="true" /></button>
      <div id="tool-panel" className={`panel remote-toolbar ${open ? "is-open" : ""}`} inert={!open} aria-hidden={!open} aria-label={t("toolbar")}>
        <div className="toolbar-brand" aria-hidden="true"><Monitor size={18} /><span>LightOS</span><span className={controls.ready ? "connection-dot online" : "connection-dot"} /></div>
        <div className="toolbar-actions" role="toolbar" aria-label={t("toolbar")} onKeyDown={(event) => {
          if (!(event.target instanceof Element) || event.target.closest('[role="menubar"]') || !event.target.closest(".toolbar-action") || event.altKey || event.ctrlKey || event.metaKey) return;
          const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(".toolbar-action:not(:disabled)"));
          const current = buttons.indexOf(event.target.closest<HTMLButtonElement>(".toolbar-action")!);
          const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
          if (!delta && event.key !== "Home" && event.key !== "End") return;
          event.preventDefault();
          buttons[event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (current + delta + buttons.length) % buttons.length]?.focus();
        }}>
          <Button id="connection" className="icon-button toolbar-action" title={translateClient(controls.wanted ? "Disconnect" : "Reconnect")} aria-label={translateClient(controls.wanted ? "Disconnect" : "Reconnect")}><Cable size={19} aria-hidden="true" /></Button>
          <Menubar.Root value={open ? menu : ""} onValueChange={setMenu} className="session-menubar" aria-label={t("toolbar")}>
            <ToolMenu edge={edge} id="display-menu" label={t("displayMenu")} icon={<Monitor size={18} aria-hidden="true" />}>
              <Menubar.Label className="session-menu-label">{t("pictureSize")}</Menubar.Label>
              <Menubar.RadioGroup value={view.mode} onValueChange={(value) => invoke(value === "actual" ? "mode-actual" : "mode-fit")}>
                <Choice value="fit">{t("fit")}</Choice><Choice value="actual">{t("actual")}</Choice>
              </Menubar.RadioGroup>
              <Submenu label={`${t("resolution")} · ${view.width} × ${view.height}`} disabled={noResize}>
                <Menubar.CheckboxItem className="session-menu-item" checked={view.autoResolution} onSelect={(event) => event.preventDefault()} onCheckedChange={(checked) => {
                  const node = document.getElementById("auto-resolution") as HTMLInputElement;
                  node.checked = checked === true; node.dispatchEvent(new Event("change", { bubbles: true }));
                }}><span className="menu-indicator"><Menubar.ItemIndicator><Check size={15} aria-hidden="true" /></Menubar.ItemIndicator></span>{t("autoResolution")}</Menubar.CheckboxItem>
                <Menubar.Separator className="session-menu-separator" />
                {["1280×720", "1600×900", "1920×1080", "2560×1440"].map((size) => <Menubar.Item className="session-menu-item" key={size} onSelect={() => {
                  const [width, height] = size.split("×");
                  (document.getElementById("width") as HTMLInputElement).value = width;
                  (document.getElementById("height") as HTMLInputElement).value = height;
                  invoke("apply-size");
                }}>{size}</Menubar.Item>)}
                <Menubar.Separator className="session-menu-separator" />
                <Menubar.Item className="session-menu-item" onSelect={() => showSettings(0)}>{t("customResolution")}</Menubar.Item>
              </Submenu>
              <Menubar.Separator className="session-menu-separator" />
              <Menubar.Label className="session-menu-label">{t("streamStrategy")}</Menubar.Label>
              <Menubar.RadioGroup value={view.strategy} onValueChange={(value) => choose("stream-strategy", value)}>
                <Choice value="auto">{t("automatic")}</Choice><Choice value="low_latency">{t("lowLatency")}</Choice><Choice value="smooth">{t("smooth")}</Choice><Choice value="quality_first">{t("qualityFirst")}</Choice>
              </Menubar.RadioGroup>
              {controls.choices.length > 0 && <><Menubar.Separator className="session-menu-separator" /><Submenu label={t("source")} disabled={controls.switchingDesktop}>
                <Menubar.RadioGroup value={controls.desktopMode} onValueChange={(value) => choose("desktop-source", value)}>
                  {controls.choices.map((choice) => <Menubar.RadioItem className="session-menu-item" key={choice.mode} value={choice.mode} disabled={!choice.available} title={translateClient(choice.reason ?? "")}><span className="menu-indicator"><Menubar.ItemIndicator><Check size={15} aria-hidden="true" /></Menubar.ItemIndicator></span>{translateClient(choice.label)}</Menubar.RadioItem>)}
                </Menubar.RadioGroup>
                <Menubar.Separator className="session-menu-separator" /><Menubar.Item className="session-menu-item" disabled={controls.refreshingDesktops || controls.switchingDesktop} onSelect={() => invoke("refresh-desktops")}>{t("refresh")}</Menubar.Item>
              </Submenu></>}
              {controls.desktopMode === "physical" && controls.outputs.length > 0 && <Submenu label={t("monitor")} disabled={!controls.ready || controls.switchingDesktop}>
                <Menubar.RadioGroup value={view.output} onValueChange={(value) => choose("monitor-source", value)}>{controls.outputs.map((output) => <Choice key={output.name} value={output.name}>{output.name} · {output.width} × {output.height}</Choice>)}</Menubar.RadioGroup>
              </Submenu>}
              <Menubar.Separator className="session-menu-separator" /><Menubar.Item className="session-menu-item" onSelect={() => showSettings(0)}>{t("moreDisplaySettings")}<SlidersHorizontal className="menu-trailing" size={15} aria-hidden="true" /></Menubar.Item>
            </ToolMenu>
            <ToolMenu edge={edge} id="keyboard-toggle" label={t("keyboardMenu")} icon={<Keyboard size={18} aria-hidden="true" />}>
              <Menubar.Item className="session-menu-item" disabled={!controls.ready} onSelect={() => invoke("keyboard-capture")}>{t(view.keyboardCaptured ? "releaseCapture" : "capture")}</Menubar.Item>
              <Menubar.Separator className="session-menu-separator" />
              <Menubar.Item className="session-menu-item" disabled={!controls.ready} onSelect={() => showUtility("keyboard")}>{t("sendChord")}<span className="menu-trailing">…</span></Menubar.Item>
              <Menubar.Item className="session-menu-item" onSelect={() => showUtility("keyboard")}>{t("keyboardHelpTitle")}</Menubar.Item>
            </ToolMenu>
            <ToolMenu edge={edge} id="clipboard-toggle" label={t("clipboardMenu")} icon={<Clipboard size={18} aria-hidden="true" />}>
              <Menubar.Item className="session-menu-item" disabled={!controls.ready} onSelect={() => invoke("paste-local")}>{t("pasteLocal")}</Menubar.Item>
              <Menubar.Item className="session-menu-item" disabled={!controls.ready} onSelect={() => { invoke("clipboard-get"); showUtility("clipboard"); }}>{t("getClipboard")}</Menubar.Item>
              <Menubar.Separator className="session-menu-separator" />
              <Menubar.Item className="session-menu-item" onSelect={() => showUtility("clipboard")}>{t("input")}<span className="menu-trailing">…</span></Menubar.Item>
            </ToolMenu>
            <ToolMenu edge={edge} id="info-menu" label={t("connectionInfo")} icon={<Activity size={18} aria-hidden="true" />}>
              <Menubar.Label className="session-menu-label connection-state"><span className={controls.ready ? "connection-dot online" : "connection-dot"} aria-hidden="true" /><span aria-live="polite">{status.state}</span></Menubar.Label>
              <div className="session-menu-status"><p title={t("metrics")}>{status.performance}</p><p>{status["video-details"]}</p><p role="status">{status["desktop-status"]}</p><p role="status" aria-live="polite">{status.message}</p></div>
              <Menubar.Separator className="session-menu-separator" /><Menubar.Item className="session-menu-item" onSelect={() => showSettings(4)}>{t("about")}</Menubar.Item>
            </ToolMenu>
          </Menubar.Root>
          <Button id="audio" className="icon-button toolbar-action" title={t(controls.audioEnabled ? "audioOff" : "audioOn")} aria-label={t(controls.audioEnabled ? "audioOff" : "audioOn")} aria-pressed={controls.audioEnabled} disabled={!controls.ready || !controls.audioAvailable}>{controls.audioEnabled ? <Volume2 size={19} aria-hidden="true" /> : <VolumeX size={19} aria-hidden="true" />}</Button>
          <div className="toolbar-end">
            <Button id="settings-toggle" className="icon-button toolbar-action" title={t("settings")} aria-label={t("settings")} onClick={() => showSettings()}><Settings2 size={19} aria-hidden="true" /></Button>
            <Button id="fullscreen" className="icon-button toolbar-action" title={t(view.fullscreen ? "exitFullscreen" : "fullscreen")} aria-label={t(view.fullscreen ? "exitFullscreen" : "fullscreen")} aria-pressed={view.fullscreen}>{view.fullscreen ? <Shrink size={19} aria-hidden="true" /> : <Expand size={19} aria-hidden="true" />}</Button>
            <Button id="tools-close" className="icon-button toolbar-action" title={t("close")} aria-label={t("close")}><CollapseChevron size={18} aria-hidden="true" /></Button>
          </div>
        </div>
      </div>
    </aside>
    <div hidden aria-hidden="true">
      <Button id="mode-fit" /><Button id="mode-actual" />
      <p id="state" /><p id="performance" /><p id="video-details" /><p id="message" />
      <p id="display-mode" />
      <section id="desktop-controls"><Select id="desktop-source" /><Button id="refresh-desktops" /><div id="monitor-controls"><Select id="monitor-source" /></div><p id="desktop-status" /></section>
    </div>
    <Dialog static open={utility !== null} onClose={() => setUtility(null)} className={`settings-shell ${utility ? "is-open" : ""}`} inert={utility === null} aria-hidden={utility === null}>
      <DialogBackdrop className="settings-backdrop" /><div className="settings-positioner"><DialogPanel className="settings-dialog utility-dialog">
        <header className="settings-heading"><DialogTitle>{t(utility === "keyboard" ? "keyboard" : "clipboard")}</DialogTitle><Button className="icon-button" aria-label={t("closeSettings")} onClick={() => setUtility(null)}><X size={20} aria-hidden="true" /></Button></header>
        <div className="settings-content">
          <section id="keyboard-section" hidden={utility !== "keyboard"} className="settings-section">
            <Button id="keyboard-capture" onClick={() => setUtility(null)} type="button" aria-pressed="false" disabled={!controls.ready} />
            <p id="keyboard-state" /><p>{t("keyboardHelp")}</p>
            <fieldset id="shortcut-modifiers"><legend>{t("chord")}</legend><label><input id="shortcut-ctrl" type="checkbox" /> Ctrl</label><label><input id="shortcut-alt" type="checkbox" /> Alt</label><label><input id="shortcut-shift" type="checkbox" /> Shift</label><label><input id="shortcut-super" type="checkbox" /> Super</label></fieldset>
            <Field><Label htmlFor="shortcut-key">{t("mainKey")}</Label><Select id="shortcut-key" aria-label={t("mainKey")} /></Field>
            <Button id="send-shortcut" type="button" disabled={!controls.ready}>{t("sendChord")}</Button>
          </section>
          <section id="clipboard-section" hidden={utility !== "clipboard"} className="settings-section">
            <Field className="text-controls"><Label htmlFor="text">{t("input")}</Label><Textarea id="text" rows={5} spellCheck={false} autoComplete="off" aria-describedby="help" />
              <div className="clipboard-actions"><Button id="send-text" type="button" disabled={!controls.ready}>{t("sendText")}</Button><Button id="clipboard-get" type="button" disabled={!controls.ready}>{t("getClipboard")}</Button><Button id="clipboard-set" type="button" disabled={!controls.ready}>{t("setClipboard")}</Button><Button id="paste-local" type="button" disabled={!controls.ready}>{t("pasteLocal")}</Button></div>
            </Field>
          </section>
        </div>
      </DialogPanel></div>
    </Dialog>
    <Dialog static open={settingsOpen} onClose={() => setSettingsOpen(false)} className={`settings-shell ${settingsOpen ? "is-open" : ""}`} inert={!settingsOpen} aria-hidden={!settingsOpen}>
      <DialogBackdrop className="settings-backdrop" />
      <div className="settings-positioner">
        <DialogPanel className="settings-dialog">
          <header className="settings-heading"><div><span className="settings-eyebrow">LightOS Remote Desktop</span><DialogTitle>{t("settings")}</DialogTitle></div><Button type="button" className="icon-button" aria-label={t("closeSettings")} onClick={() => setSettingsOpen(false)} data-autofocus><X size={20} aria-hidden="true" /></Button></header>
          <TabGroup className="settings-body" selectedIndex={settingsTab} onChange={setSettingsTab}>
            <TabList className="settings-tabs"><Tab><SlidersHorizontal size={16} aria-hidden="true" />{t("displayMenu")}</Tab><Tab><Palette size={16} aria-hidden="true" />{t("preferences")}</Tab><Tab><Bot size={16} aria-hidden="true" />MCP</Tab><Tab>RDP</Tab><Tab><Info size={16} aria-hidden="true" />{t("about")}</Tab></TabList>
            <TabPanels className="settings-content">
              <TabPanel unmount={false}>
        <section id="settings-section" className="settings-section"><h3>{t("resolution")}</h3>
          <label className="fit-control"><input id="auto-resolution" type="checkbox" defaultChecked disabled={noResize} />{t("autoResolution")}</label>
          {!controls.resizeAvailable && <p>{t("physicalResize")}</p>}
          <form id="resize" className="resolution-grid" aria-label={t("resolution")}>
            <Field><Label>{t("width")}</Label><Input id="width" type="number" min="320" max="3840" step="1" defaultValue="1280" required disabled={noResize} /></Field>
            <Field><Label>{t("height")}</Label><Input id="height" type="number" min="200" max="2160" step="1" defaultValue="720" required disabled={noResize} /></Field>
            <Button id="apply-size" type="submit" disabled={noResize}>{t("resize")}</Button>
          </form>
        </section>
        <section id="stream-section" className="settings-section"><h3>{t("streamSettings")}</h3>
          <Field className="preference-field"><Label htmlFor="video-codec">{t("videoCodec")}</Label><Select id="video-codec" defaultValue="auto"><option value="auto">{t("automatic")}</option><option value="h264">H.264</option><option value="jpeg">JPEG</option></Select></Field>
          <Field className="preference-field"><Label htmlFor="encoder-backend">{t("encoderBackend")}</Label><Select id="encoder-backend" defaultValue="auto"><option value="auto">{t("automatic")}</option><option value="software">{t("softwareEncoding")}</option><option value="vaapi">VA-API</option><option value="nvenc">NVENC</option><option value="amf">AMF</option><option value="videotoolbox">VideoToolbox</option></Select></Field>
          <p id="encoder-status" role="status" />
          <Field className="preference-field"><Label htmlFor="stream-strategy">{t("streamStrategy")}</Label><Select id="stream-strategy" defaultValue="auto"><option value="auto">{t("automatic")}</option><option value="low_latency">{t("lowLatency")}</option><option value="smooth">{t("smooth")}</option><option value="quality_first">{t("qualityFirst")}</option></Select></Field>
          <p>{t("streamHelp")}</p>
        </section>

              </TabPanel>
              <TabPanel unmount={false}>
        <section id="preferences-section" className="settings-section"><h3>{t("preferences")}</h3>
          <Field className="preference-field"><Label htmlFor="language">{t("language")}</Label><Select id="language" value={language} onChange={(event) => { const value = event.target.value as LanguagePreference; savePreference(languageKey, value); setLanguage(value); }}><option value="system">{t("browser")}</option><option value="zh">简体中文</option><option value="en">English</option></Select></Field>
          <Field className="preference-field"><Label htmlFor="theme">{t("theme")}</Label><Select id="theme" value={theme} onChange={(event) => { const value = event.target.value as ThemePreference; savePreference(themeKey, value); setTheme(value); }}><option value="system">{t("system")}</option><option value="light">{t("light")}</option><option value="dark">{t("dark")}</option></Select></Field>
        </section>

              </TabPanel>
              <TabPanel unmount={false}><McpSettings visible={settingsOpen && settingsTab === 2} embedded /></TabPanel>
              <TabPanel unmount={false}><RdpSettings visible={settingsOpen && settingsTab === 3} /></TabPanel>
              <TabPanel unmount={false}>
                <section className="settings-section about-section" aria-label={t("about")}>
                  <div className="about-brand"><img src={logo} width="48" height="48" alt="" /><div><h3>LightOS Remote Desktop</h3><p>{t("aboutDescription")}</p></div></div>
                  <dl className="about-details"><div><dt>{t("version")}</dt><dd id="software-version">{view.version || (version && version !== "__LIGHTOS_VERSION__" ? `v${version}` : t("developmentBuild"))}</dd></div><div><dt>{t("license")}</dt><dd>MIT</dd></div></dl>
                  <div className="about-links"><a href="https://github.com/ca-x/lightos-remote-desktop" target="_blank" rel="noreferrer">{t("projectRepository")}</a><a href="https://github.com/ca-x/lightos-remote-desktop/releases" target="_blank" rel="noreferrer">{t("releaseNotes")}</a><a href="https://github.com/ca-x/lightos-remote-desktop/issues" target="_blank" rel="noreferrer">{t("reportIssue")}</a><a href="https://github.com/ca-x/lightos-remote-desktop/blob/main/LICENSE" target="_blank" rel="noreferrer">{t("license")}</a></div>
                  <p id="help">{t("help")}</p><p>{t("metrics")}</p><p className="mobile-help">{t("touchHelp")}</p>
                </section>
              </TabPanel>
            </TabPanels>
          </TabGroup>
          <footer className="settings-footer"><span>{t("settingsSaved")}</span><Button type="button" onClick={() => setSettingsOpen(false)}>{t("done")}</Button></footer>
        </DialogPanel>
      </div>
    </Dialog>
  </main>;
}

export default function App() {
  return <Workspace />;
}
