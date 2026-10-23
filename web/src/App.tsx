import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, Dialog, DialogBackdrop, DialogPanel, DialogTitle, Field, Input, Label, Popover, PopoverButton, PopoverGroup, PopoverPanel, Select, Tab, TabGroup, TabList, TabPanel, TabPanels, Textarea } from "@headlessui/react";
import { Activity, Bot, Cable, ChevronDown, Clipboard, Expand, Grip, Keyboard, Maximize, Menu, Minimize, Monitor, Palette, RefreshCw, Settings2, SlidersHorizontal, Volume2, VolumeX, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { createClient } from "./client";
import { RdpSettings } from "./RdpSettings";
import { McpSettings } from "./McpSettings";
import { MOBILE_ROWS, type MobileShortcut } from "./mobile";
import type { ControlState, ToolbarEdge } from "./types";
import i18n, { translateClient } from "./i18n";
import { languageKey, languages, readPreference, resolveLanguage, resolveTheme, savePreference, themeKey, themes, type LanguagePreference, type ThemePreference } from "./preferences";

const initialControls: ControlState = {
  ready: false, wanted: true, audioAvailable: false, audioEnabled: false,
  resizeAvailable: true, desktopMode: "virtual", switchingDesktop: false,
  refreshingDesktops: false, choices: [], outputs: [],
};

type ToolMenuProps = { id: string; label: string; icon: React.ReactNode; edge: ToolbarEdge; toolbarOpen: boolean; children: React.ReactNode };

function ToolMenu(props: ToolMenuProps) {
  return <Popover className="toolbar-menu">{({ open, close }) => <ToolMenuContents {...props} open={open} close={close} />}</Popover>;
}

function ToolMenuContents({ id, label, icon, edge, toolbarOpen, children, open, close }: ToolMenuProps & { open: boolean; close: (focusable?: HTMLElement) => void }) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!toolbarOpen && open) {
      const active = document.activeElement;
      close(active instanceof HTMLElement && !panel.current?.contains(active) ? active : document.getElementById("desktop") ?? undefined);
    }
  }, [toolbarOpen, open, close]);
  const side = edge === "right" ? "left" : edge === "left" ? "right" : edge === "bottom" ? "top" : "bottom";
  const visible = toolbarOpen && open;
  return <>
    <PopoverButton id={id} className="icon-button toolbar-action menu-trigger" title={label} aria-label={label}>
      {icon}<ChevronDown className="menu-chevron" size={10} aria-hidden="true" />
    </PopoverButton>
    <PopoverPanel ref={panel} static focus portal anchor={{ to: `${side} start`, gap: 10, padding: 12 }}
      style={{ transformOrigin: side === "left" ? "right top" : side === "right" ? "left top" : side === "top" ? "left bottom" : "left top" }}
      className={`submenu ${visible ? "is-open" : ""}`} inert={!visible} aria-hidden={!visible} role="dialog" aria-label={label}>
      <header className="submenu-heading"><h2>{label}</h2><PopoverButton className="submenu-close" aria-label={i18n.t("dismissMenu")}><X size={16} aria-hidden="true" /></PopoverButton></header>
      {children}
    </PopoverPanel>
  </>;
}

function Workspace({ open, close }: { open: boolean; close: (focusable?: HTMLElement) => void }) {
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
  const noResize = !controls.ready || !controls.resizeAvailable;
  const shortcut = (item: MobileShortcut) => {
    const names: Record<string, string> = { ArrowUp: t("arrowUp"), ArrowDown: t("arrowDown"), ArrowLeft: t("arrowLeft"), ArrowRight: t("arrowRight"), Backspace: t("backspace") };
    const label = item.modifier ? t("stickyKey", { key: item.label }) : names[item.key ?? ""] ?? item.label;
    return <button key={item.key ?? item.text} type="button" className="mobile-key" disabled={!controls.ready}
      data-mobile-key={item.key} data-mobile-modifiers={item.modifiers?.join(" ")} data-mobile-text={item.text}
      aria-label={label} title={label} aria-pressed={item.modifier ? false : undefined}>{item.label}</button>;
  };
  return <main id="viewport" className="fit" aria-label={t("viewport")} data-input="pointer"
    onPointerDownCapture={(event) => { event.currentTarget.dataset.input = "pointer"; document.documentElement.dataset.input = "pointer"; }}
    onKeyDownCapture={(event) => { event.currentTarget.dataset.input = "keyboard"; document.documentElement.dataset.input = "keyboard"; }}>
    <canvas id="desktop" width="1280" height="720" tabIndex={0} aria-label={t("canvas")} aria-describedby="help" />
    <textarea id="mobile-input" className="mobile-input" tabIndex={-1} aria-label={t("mobileInput")}
      autoCapitalize="off" autoCorrect="off" autoComplete="off" spellCheck={false} inputMode="text" enterKeyHint="enter" />
    <nav id="mobile-shortcuts" className="mobile-shortcuts" aria-label={t("mobileShortcuts")}>
      <div className="mobile-shortcut-row" role="toolbar" aria-label={t("mobileRow1")}>
        <button id="mobile-keyboard" className="mobile-key mobile-keyboard" type="button" disabled={!controls.ready} aria-label={t("showKeyboard")} title={t("showKeyboard")} aria-pressed="false"><Keyboard size={21} aria-hidden="true" /></button>
        <div className="mobile-key-scroll">{MOBILE_ROWS[0].map(shortcut)}</div>
      </div>
      <div className="mobile-shortcut-row" role="toolbar" aria-label={t("mobileRow2")}>
        <button id="mobile-menu" className="mobile-key" type="button" data-mobile-action="menu" disabled={!controls.ready} aria-label={t("mobileMenu")} title={t("mobileMenu")}><Menu size={19} aria-hidden="true" /></button>
        <div className="mobile-key-scroll">{MOBILE_ROWS[1].map(shortcut)}</div>
      </div>
    </nav>
    <aside id="tools" aria-label={t("title")}>
      <PopoverButton id="tools-toggle" className="tool-handle" title={t("drag")} aria-label={t(open ? "close" : "open")}><Grip size={20} aria-hidden="true" /></PopoverButton>
      <PopoverPanel static id="tool-panel" className={`panel remote-toolbar ${open ? "is-open" : ""}`} inert={!open} aria-hidden={!open} aria-label={t("toolbar")}>
        <div className="toolbar-brand" aria-hidden="true"><Monitor size={18} /><span>LightOS</span><span className={controls.ready ? "connection-dot online" : "connection-dot"} /></div>
        <PopoverGroup className="toolbar-actions" role="toolbar" aria-label={t("toolbar")} onKeyDown={(event) => {
          if (!(event.target instanceof Element) || !event.target.closest(".toolbar-action") || event.altKey || event.ctrlKey || event.metaKey) return;
          const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(".toolbar-action:not(:disabled)"));
          const current = buttons.indexOf(event.target.closest<HTMLButtonElement>(".toolbar-action")!);
          const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
          if (!delta && event.key !== "Home" && event.key !== "End") return;
          event.preventDefault();
          buttons[event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (current + delta + buttons.length) % buttons.length]?.focus();
        }}>
          <Button id="connection" className="icon-button toolbar-action" title={translateClient(controls.wanted ? "Disconnect" : "Reconnect")} aria-label={translateClient(controls.wanted ? "Disconnect" : "Reconnect")}><Cable size={20} aria-hidden="true" /></Button>
          <ToolMenu toolbarOpen={open} edge={edge} id="display-menu" label={t("displayMenu")} icon={<Monitor size={20} aria-hidden="true" />}>
            <div className="view-modes" role="group" aria-label={t("displayMenu")}>
              <Button id="mode-fit" className="view-option" aria-label={t("fit")}><Minimize size={18} aria-hidden="true" /><span>{t("fit")}</span></Button>
              <Button id="mode-actual" className="view-option" aria-label={t("actual")}><Maximize size={18} aria-hidden="true" /><span>{t("actual")}</span></Button>
              <Button id="fullscreen" className="view-option" aria-label={t("fullscreen")}><Expand size={18} aria-hidden="true" /><span>{t("fullscreen")}</span></Button>
            </div>
            <p id="display-mode" />
        <section id="desktop-controls" className="desktop-controls" hidden={!controls.choices.length} aria-label={t("source")}>
          <div className="desktop-heading"><label htmlFor="desktop-source">{t("source")}</label><Button id="refresh-desktops" className="quiet-button" type="button" title={t("refreshTip")} disabled={controls.refreshingDesktops || controls.switchingDesktop}><RefreshCw size={14} aria-hidden="true" />{t("refresh")}</Button></div>
          <Select id="desktop-source" aria-label={t("source")} aria-describedby="desktop-status" disabled={!controls.choices.length} />
          <div id="monitor-controls" hidden={controls.desktopMode !== "physical"}><label htmlFor="monitor-source">{t("monitor")}</label><Select id="monitor-source" aria-label={t("monitor")} aria-describedby="desktop-status" disabled={!controls.ready || controls.switchingDesktop || !controls.outputs.length} /></div>
          <p id="desktop-status" role="status" />
        </section>

            <Button type="button" className="menu-row" onClick={() => { setSettingsTab(0); setSettingsOpen(true); }}><SlidersHorizontal size={16} aria-hidden="true" />{t("resolution")}</Button>
          </ToolMenu>
          <ToolMenu toolbarOpen={open} edge={edge} id="keyboard-toggle" label={t("keyboard")} icon={<Keyboard size={20} aria-hidden="true" />}>
        <section id="keyboard-section">
          <Button id="keyboard-capture" type="button" aria-pressed="false" disabled={!controls.ready} />
          <p id="keyboard-state" />
          <p>{t("keyboardHelp")}</p>
          <fieldset id="shortcut-modifiers"><legend>{t("chord")}</legend>
            <label><input id="shortcut-ctrl" type="checkbox" /> Ctrl</label><label><input id="shortcut-alt" type="checkbox" /> Alt</label><label><input id="shortcut-shift" type="checkbox" /> Shift</label><label><input id="shortcut-super" type="checkbox" /> Super</label>
          </fieldset>
          <Field><Label htmlFor="shortcut-key">{t("mainKey")}</Label><Select id="shortcut-key" aria-label={t("mainKey")} /></Field>
          <Button id="send-shortcut" type="button" disabled={!controls.ready}>{t("sendChord")}</Button>
        </section>

          </ToolMenu>
          <ToolMenu toolbarOpen={open} edge={edge} id="clipboard-toggle" label={t("clipboard")} icon={<Clipboard size={20} aria-hidden="true" />}>
        <section id="clipboard-section">
          <Field className="text-controls"><Label htmlFor="text">{t("input")}</Label><Textarea id="text" rows={3} spellCheck={false} autoComplete="off" aria-describedby="help" />
            <div className="clipboard-actions"><Button id="send-text" type="button" disabled={!controls.ready}>{t("sendText")}</Button><Button id="clipboard-get" type="button" disabled={!controls.ready}>{t("getClipboard")}</Button><Button id="clipboard-set" type="button" disabled={!controls.ready}>{t("setClipboard")}</Button><Button id="paste-local" type="button" disabled={!controls.ready}>{t("pasteLocal")}</Button></div>
          </Field>
        </section>

          </ToolMenu>
          <Button id="audio" className="icon-button toolbar-action" title={t(controls.audioEnabled ? "audioOff" : "audioOn")} aria-label={t(controls.audioEnabled ? "audioOff" : "audioOn")} aria-pressed={controls.audioEnabled} disabled={!controls.ready || !controls.audioAvailable}>{controls.audioEnabled ? <Volume2 size={20} aria-hidden="true" /> : <VolumeX size={20} aria-hidden="true" />}</Button>
          <ToolMenu toolbarOpen={open} edge={edge} id="info-menu" label={t("connectionInfo")} icon={<Activity size={20} aria-hidden="true" />}>
            <div className="connection-state"><span className={controls.ready ? "connection-dot online" : "connection-dot"} aria-hidden="true" /><span id="state" aria-live="polite" /></div>
            <p id="message" role="status" aria-live="polite" />
            <div className="telemetry"><p id="performance" title={t("metrics")} /><p id="video-details" /></div>
            <p>{t("metrics")}</p><p id="help">{t("help")}</p><p className="mobile-help">{t("touchHelp")}</p>
            <footer><span>LightOS Remote Desktop</span><span id="software-version" title={t("version")}>v{version}</span></footer>
          </ToolMenu>
          <Button id="settings-toggle" className="icon-button toolbar-action" title={t("settings")} aria-label={t("settings")} onClick={() => setSettingsOpen(true)}><Settings2 size={20} aria-hidden="true" /></Button>
          <Button id="tools-close" className="icon-button toolbar-action collapse-toolbar" title={t("close")} aria-label={t("close")}><ChevronDown size={18} aria-hidden="true" /></Button>
        </PopoverGroup>
      </PopoverPanel>
    </aside>
    <Dialog static open={settingsOpen} onClose={() => setSettingsOpen(false)} className={`settings-shell ${settingsOpen ? "is-open" : ""}`} inert={!settingsOpen} aria-hidden={!settingsOpen}>
      <DialogBackdrop className="settings-backdrop" />
      <div className="settings-positioner">
        <DialogPanel className="settings-dialog">
          <header className="settings-heading"><div><span className="settings-eyebrow">LightOS Remote Desktop</span><DialogTitle>{t("settings")}</DialogTitle></div><Button type="button" className="icon-button" aria-label={t("closeSettings")} onClick={() => setSettingsOpen(false)} data-autofocus><X size={20} aria-hidden="true" /></Button></header>
          <TabGroup className="settings-body" selectedIndex={settingsTab} onChange={setSettingsTab}>
            <TabList className="settings-tabs"><Tab><SlidersHorizontal size={16} aria-hidden="true" />{t("displayMenu")}</Tab><Tab><Palette size={16} aria-hidden="true" />{t("preferences")}</Tab><Tab><Bot size={16} aria-hidden="true" />MCP</Tab><Tab>RDP</Tab></TabList>
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
            </TabPanels>
          </TabGroup>
          <footer className="settings-footer"><span>{t("settingsSaved")}</span><Button type="button" onClick={() => setSettingsOpen(false)}>{t("done")}</Button></footer>
        </DialogPanel>
      </div>
    </Dialog>
  </main>;
}

export default function App() {
  return <Popover as="div">{({ open, close }) => <Workspace open={open} close={close} />}</Popover>;
}
