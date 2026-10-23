import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
    return { destroy, refreshLanguage, layoutTools };
  });
});

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("React shell and Headless UI lifecycle", () => {
  it("waits for Headless UI IDs and creates exactly one client during effect replay", async () => {
    const view = render(<StrictMode><App /></StrictMode>);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(screen.queryByRole("button", { name: "Fit window" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Display" }));
    expect(await screen.findByRole("button", { name: "Fit window" })).toBeDefined();
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
    fireEvent.click(within(settings).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull());
    expect(document.getElementById("width")).toBe(width);
    expect(createClient).toHaveBeenCalledTimes(1);
  });

  it("navigates toolbar actions with arrows and skips disabled audio", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    const toolbar = screen.getByRole("toolbar", { name: "Session toolbar" });
    const clipboard = within(toolbar).getByRole("button", { name: "Text & clipboard" });
    clipboard.focus();
    fireEvent.keyDown(clipboard, { key: "ArrowRight" });
    expect(document.activeElement).toBe(within(toolbar).getByRole("button", { name: "Connection information" }));
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    expect(document.activeElement).toBe(within(toolbar).getByRole("button", { name: "Disconnect" }));
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

  it("dismisses portalled menus when collapsing the parent toolbar", async () => {
    render(<App />);
    await waitFor(() => expect(createClient).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    fireEvent.click(screen.getByRole("button", { name: "Display" }));
    await screen.findByRole("dialog", { name: "Display" });
    fireEvent.click(document.getElementById("tools-toggle")!);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Display" })).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Open desktop tools" }));
    expect(screen.queryByRole("dialog", { name: "Display" })).toBeNull();
    expect(screen.getByRole("button", { name: "Display" }).getAttribute("aria-expanded")).toBe("false");
  });

});
