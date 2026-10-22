import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    expect(screen.getByRole("button", { name: "Fit window" })).toBeDefined();
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
    fireEvent.change(document.getElementById("language")!, { target: { value: "zh" } });
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
    expect(screen.getByRole("button", { name: "适应窗口" })).toBeDefined();
    fireEvent.change(document.getElementById("theme")!, { target: { value: "dark" } });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("lightos-desktop-language")).toBe("zh");
    expect(localStorage.getItem("lightos-desktop-theme")).toBe("dark");
    expect(createClient).toHaveBeenCalledTimes(1);
    expect(destroy).not.toHaveBeenCalled();
  });
});
