import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import App from "../web/src/App";
import { createClient } from "../web/src/client";
import i18n from "../web/src/i18n";

vi.mock("../web/src/client", () => ({ createClient: vi.fn() }));
const destroy = vi.fn();
const refreshLanguage = vi.fn();
const layoutTools = vi.fn();

beforeEach(async () => {
  vi.clearAllMocks();
  localStorage.clear();
  await i18n.changeLanguage("en");
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
  vi.mocked(createClient).mockImplementation(() => {
    expect(document.getElementById("tools-toggle")).not.toBeNull();
    expect(document.getElementById("tool-panel")).not.toBeNull();
    return { destroy, refreshLanguage, layoutTools, refreshDesktops: async () => {}, setLightBook: vi.fn() };
  });
});

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("React shell, menus and dialog lifecycle", () => {
  it("waits for persistent client controls and creates exactly one client during effect replay", async () => {
    const view = render(<StrictMode><App /></StrictMode>);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(screen.queryByRole("menuitemradio", { name: "Fit window" })).toBeNull();
    fireEvent.pointerDown(screen.getByRole("menuitem", { name: "Display" }), { button: 0, ctrlKey: false });
    expect(await screen.findByRole("menuitemradio", { name: "Fit window" })).toBeDefined();
    view.unmount();
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it("cancels initialization when unmounted before the next frame", async () => {
    const view = render(<App />);
    view.unmount();
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(createClient).not.toHaveBeenCalled();
  });

  it("changes language and theme without restarting the remote client", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(settings).getByRole("tab", { name: "Appearance & language" }));
    fireEvent.change(document.getElementById("language")!, { target: { value: "zh" } });
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
    expect(screen.getByRole("dialog", { name: "设置" })).toBeDefined();
    fireEvent.change(document.getElementById("theme")!, { target: { value: "dark" } });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("lightos-desktop-language")).toBe("zh");
    expect(localStorage.getItem("lightos-desktop-theme")).toBe("dark");
    expect(createClient).toHaveBeenCalledTimes(1);
    expect(destroy).not.toHaveBeenCalled();
  });
  it("keeps client controls mounted while settings are closed and preserves values across tabs", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    for (const id of ["video-codec", "encoder-backend", "width", "height", "text", "shortcut-key"]) expect(document.getElementById(id)).not.toBeNull();
    const width = document.getElementById("width") as HTMLInputElement;
    width.value = "1920";
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(settings).getByRole("tab", { name: "Appearance & language" }));
    fireEvent.click(within(settings).getByRole("tab", { name: "Display" }));
    expect(document.getElementById("width")).toBe(width);
    expect(width.value).toBe("1920");
    fireEvent.click(within(settings).getByRole("button", { name: "Close settings" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull());
    expect(document.getElementById("width")).toBe(width);
    expect(createClient).toHaveBeenCalledTimes(1);
  });

  it("navigates the menu strip with arrows and Home", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    const toolbar = screen.getByRole("toolbar", { name: "Session toolbar" });
    const clipboard = within(toolbar).getByRole("menuitem", { name: "Clipboard" });
    clipboard.focus();
    fireEvent.keyDown(clipboard, { key: "ArrowRight" });
    await waitFor(() => expect(document.activeElement).toBe(within(toolbar).getByRole("menuitem", { name: "Connection information" })));
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    await waitFor(() => expect(document.activeElement).toBe(within(toolbar).getByRole("menuitem", { name: "Display" })));
  });

  it("clears one-time MCP tokens when switching settings tabs or closing the dialog", async () => {
    const token = "lrd_test_one_time_secret";
    vi.stubGlobal("fetch", vi.fn(async (_url, options) => options?.method === "POST"
      ? Response.json({ token, info: { id: "one", name: "Agent", scope: "observe", created_at: 1 } })
      : Response.json({ enabled: false, tokens: [] })));
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(settings).getByRole("tab", { name: "MCP" }));
    await within(settings).findByLabelText("Token name");
    fireEvent.change(screen.getByLabelText("Token name"), { target: { value: "Agent" } });
    fireEvent.submit(screen.getByRole("form", { name: "Create an MCP token" }));
    await screen.findByDisplayValue(token);
    fireEvent.click(within(settings).getByRole("tab", { name: "Display" }));
    await waitFor(() => expect(screen.queryByDisplayValue(token)).toBeNull());
    fireEvent.click(within(settings).getByRole("tab", { name: "MCP" }));
    expect(screen.queryByDisplayValue(token)).toBeNull();
    fireEvent.click(within(settings).getByRole("button", { name: "Close settings" }));
    expect(localStorage.length).toBe(0);
    expect(createClient).toHaveBeenCalledTimes(1);
  });

  it("puts RDP only in Settings and cancels its request when switching tabs", async () => {
    const fetchRdp = vi.fn((_url: unknown, _options?: RequestInit) => new Promise<Response>(() => {}));
    vi.stubGlobal("fetch", fetchRdp);
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(within(screen.getByRole("toolbar", { name: "Session toolbar" })).queryByText("RDP")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(settings).getByRole("tab", { name: "RDP" }));
    await waitFor(() => expect(fetchRdp).toHaveBeenCalledOnce());
    const signal = fetchRdp.mock.calls[0][1]?.signal;
    expect(within(settings).getByRole("checkbox", { name: "Enable RDP" })).toBeTruthy();
    expect(within(settings).queryByRole("combobox")).toBeNull();
    fireEvent.click(within(settings).getByRole("tab", { name: "Display" }));
    expect(signal?.aborted).toBe(true);
    expect(createClient).toHaveBeenCalledTimes(1);
  });

  it("dismisses portalled menus when collapsing the parent toolbar", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    fireEvent.pointerDown(screen.getByRole("menuitem", { name: "Display" }), { button: 0, ctrlKey: false });
    await screen.findByRole("menu", { name: "Display" });
    fireEvent.click(document.getElementById("tools-toggle")!);
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Display" })).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(screen.queryByRole("menu", { name: "Display" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Display" }).getAttribute("aria-expanded")).toBe("false");
  });

  it.each([
    ["top", "right"], ["bottom", "right"],
    ["left", "right"], ["right", "left"],
  ] as const)("keeps the same directional trigger mounted when toggling at the %s edge", async (edge, expand) => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    act(() => {
      vi.mocked(createClient).mock.calls[0][0]?.onToolbarDock?.(edge);
      vi.mocked(createClient).mock.calls[0][0]?.onToolbarDirection?.(expand);
    });
    const toggle = screen.getByRole("button", { name: "Open desktop tools" });
    expect(toggle.querySelector(".lucide-chevron-right")).not.toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);
    expect(toggle.querySelector(".lucide-chevron-right")).not.toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(document.getElementById("tools-close")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.querySelector(".lucide-chevron-right")).not.toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(createClient).toHaveBeenCalledTimes(1);
  });

  it("keeps fullscreen directly accessible and About last in Settings with a compact logo", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(screen.getByRole("button", { name: "Fullscreen" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    const tabs = within(settings).getAllByRole("tab");
    expect(tabs.at(-1)?.textContent).toBe("About");
    fireEvent.click(tabs.at(-1)!);
    expect(within(settings).getByRole("heading", { name: "LightOS Remote Desktop" })).toBeTruthy();
    const image = settings.querySelector(".about-brand img")!;
    expect(image.getAttribute("width")).toBe("48");
    expect(image.getAttribute("height")).toBe("48");
    expect(within(settings).getByRole("link", { name: "Release notes" }).getAttribute("href")).toContain("/releases");
    expect(within(settings).getByText("PolyForm Noncommercial License 1.0.0")).toBeTruthy();
  });

});

it("places full keyboard mode below theme, off by default, and only enables it for mobile touch devices", async () => {
  vi.stubGlobal("navigator", { maxTouchPoints: 5, userAgent: "Android", platform: "Linux", languages: ["en-US"], language: "en-US" });
  render(<App />);
  await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  const dialog = await screen.findByRole("dialog", { name: "Settings" });
  fireEvent.click(within(dialog).getByRole("tab", { name: "Appearance & language" }));
  const toggle = within(dialog).getByRole("switch", { name: "Full keyboard mode" }) as HTMLInputElement;
  expect(toggle.checked).toBe(false);expect(toggle.disabled).toBe(false);
  expect(document.getElementById("theme")!.compareDocumentPosition(toggle) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  fireEvent.click(toggle);
  expect(localStorage.getItem("lightos-desktop-lightbook")).toBe("on");
  expect(document.querySelector(".lightbook-brand")!.textContent).toBe("LightOS");
  expect(document.querySelector(".lightbook-deck")!.hasAttribute("hidden")).toBe(false);
  fireEvent.click(within(dialog).getByRole("button", { name: "Close settings" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull());
  expect(within(document.querySelector(".lightbook-deck") as HTMLElement).getAllByRole("button",{name:"Super"})).toHaveLength(2);
});


it("places synchronized audio and fullscreen shortcuts before the full keyboard IME entry", async () => {
  vi.stubGlobal("navigator", { maxTouchPoints:5,userAgent:"Android",platform:"Linux",languages:["en-US"],language:"en-US" });
  localStorage.setItem("lightos-desktop-lightbook","on");
  render(<App/>);await waitFor(()=>expect(createClient).toHaveBeenCalledTimes(1));
  const actions=Array.from(document.querySelectorAll<HTMLButtonElement>('.lightbook-controls [data-mobile-action]'));
  expect(actions.map(button=>button.dataset.mobileAction)).toEqual(["audio","fullscreen","keyboard","menu"]);
  expect(actions[0].disabled).toBe(true);expect(actions[1].disabled).toBe(false);
  const callbacks=vi.mocked(createClient).mock.calls[0][0]!;
  await act(async()=>callbacks.onControls?.({ready:true,wanted:true,audioAvailable:true,audioEnabled:true,resizeAvailable:true,desktopMode:"virtual",switchingDesktop:false,refreshingDesktops:false,choices:[],outputs:[]}));
  expect(actions[0].disabled).toBe(false);expect(actions[0].getAttribute('aria-pressed')).toBe('true');expect(actions[0].getAttribute('aria-label')).toBe('Mute audio');
});
