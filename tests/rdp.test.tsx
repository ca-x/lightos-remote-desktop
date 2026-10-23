import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RdpSettings } from "../web/src/RdpSettings";
import i18n from "../web/src/i18n";

let status: { supported: boolean; available: boolean; enabled: boolean; port: number; endpoint: string; loopback: boolean; fingerprint: string | null; error: boolean; unavailable_reason: string | null };
let failure: number | null;
const fetchMock = vi.fn(async (_input: URL | RequestInfo, options?: RequestInit) => {
  if (failure) return new Response("failed", { status: failure });
  if (options?.method === "PUT") {
    status = { ...status, ...JSON.parse(options.body as string) };
    status.endpoint = `127.0.0.1:${status.port}`;
  }
  return Response.json(status);
});
beforeEach(async () => {
  status = { supported: true, available: true, enabled: false, port: 3389, endpoint: "127.0.0.1:3389", loopback: true, fingerprint: null, error: false, unavailable_reason: null };
  failure = null;
  vi.clearAllMocks();
  await i18n.changeLanguage("en");
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

async function ready() {
  const toggle = screen.getByRole("checkbox", { name: "Enable RDP" }) as HTMLInputElement;
  await waitFor(() => expect(toggle.disabled).toBe(false));
  return toggle;
}

describe("RDP settings", () => {
  it("loads only when visible and offers enable plus a strict port without quality controls", async () => {
    const view = render(<RdpSettings visible={false} />);
    expect(fetchMock).not.toHaveBeenCalled();
    view.rerender(<RdpSettings visible />);
    const toggle = await ready();
    expect(screen.getAllByRole("spinbutton")).toHaveLength(1);
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByText(/forward this port over SSH/)).toBeTruthy();
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(true));
    const port = screen.getByLabelText("RDP port");
    fireEvent.change(port, { target: { value: "3390" } });
    fireEvent.blur(port);
    await waitFor(() => expect(status.port).toBe(3390));
    await ready();
    fireEvent.change(port, { target: { value: "65536" } });
    fireEvent.blur(port);
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Enter a whole port number from 1 to 65535.");
    expect(status.port).toBe(3390);
    for (const [url, options] of fetchMock.mock.calls) {
      expect(new URL(String(url)).pathname).toMatch(/\/api\/rdp$/);
      expect(options?.credentials).toBe("same-origin");
      expect(options?.headers).toHaveProperty("X-LightOS-RDP", "1");
      if (options?.body) expect(Object.keys(JSON.parse(options.body as string)).sort()).toEqual(["enabled", "port"]);
    }
  });

  it("combines an edited port and an immediate toggle in one request", async () => {
    render(<RdpSettings visible />);
    const toggle = await ready();
    const port = screen.getByLabelText("RDP port");
    fireEvent.focus(port);
    fireEvent.change(port, { target: { value: "3391" } });
    fireEvent.pointerDown(toggle);
    fireEvent.blur(port, { relatedTarget: toggle });
    fireEvent.pointerUp(toggle);
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(true));
    const updates = fetchMock.mock.calls.filter(([, options]) => options?.method === "PUT");
    expect(updates).toHaveLength(1);
    expect(JSON.parse(updates[0][1]?.body as string)).toEqual({ enabled: true, port: 3391 });
    expect(status.port).toBe(3391);
  });

  it("keeps the applied switch and endpoint when a save fails", async () => {
    render(<RdpSettings visible />);
    const toggle = await ready();
    failure = 503;
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", expect.stringContaining("Could not apply RDP settings"));
    expect(toggle.checked).toBe(false);
    expect((screen.getByLabelText("Server bind address") as HTMLInputElement).value).toBe("127.0.0.1:3389");
  });

  it.each([
    [false, true, null, "This server build does not include RDP support."],
    [true, false, "attached", "RDP is available only for the owned virtual desktop, not an attached physical desktop."],
    [true, false, "private_state", "RDP settings are unavailable. Check the server’s private state directory and its permissions."],
  ])("explains unavailable settings", async (supported, available, reason, message) => {
    status = { ...status, supported: supported as boolean, available: available as boolean, unavailable_reason: reason as string | null };
    render(<RdpSettings visible />);
    expect(await screen.findByText(message as string)).toBeTruthy();
    expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
  });

  it("aborts on close/tab change and ignores a late response, then refreshes on return", async () => {
    let complete!: (response: Response) => void;
    fetchMock.mockImplementationOnce(() => new Promise<Response>((resolve) => { complete = resolve; }));
    const view = render(<RdpSettings visible />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const signal = fetchMock.mock.calls[0][1]?.signal;
    view.rerender(<RdpSettings visible={false} />);
    expect(signal?.aborted).toBe(true);
    complete(Response.json({ ...status, enabled: true, port: 9999 }));
    await Promise.resolve();
    view.rerender(<RdpSettings visible />);
    const toggle = await ready();
    expect(toggle.checked).toBe(false);
    expect((screen.getByLabelText("RDP port") as HTMLInputElement).value).toBe("3389");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("shows the actual endpoint and fingerprint as read-only", async () => {
    status = { ...status, enabled: true, endpoint: "0.0.0.0:3389", loopback: false, fingerprint: "AA:BB:CC" };
    render(<RdpSettings visible />);
    await ready();
    expect((screen.getByLabelText("Server bind address") as HTMLInputElement).value).toBe("0.0.0.0:3389");
    const fingerprint = screen.getByLabelText("TLS certificate SHA-256 fingerprint") as HTMLInputElement;
    expect(fingerprint.readOnly).toBe(true);
    expect(fingerprint.value).toBe("AA:BB:CC");
    expect(screen.getByText(/wildcard bind address is not a client destination/)).toBeTruthy();
  });
});
