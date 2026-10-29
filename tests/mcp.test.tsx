import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { McpSettings } from "../web/src/McpSettings";
import i18n from "../web/src/i18n";

const secret = "lrd_mcp_once_only_test_credential";
const token = { id: "test-token-id", name: "Agent one", scope: "observe", created_at: 1_792_656_000 };
let enabled: boolean;
let tokens: typeof token[];
let fail: boolean;
const fetchMock = vi.fn(async (input: URL | RequestInfo, options?: RequestInit) => {
  if (fail) return new Response("Unavailable", { status: 503 });
  const path = new URL(String(input)).pathname;
  const method = options?.method;
  if (method === "PUT") enabled = JSON.parse(options?.body as string).enabled;
  if (method === "POST") {
    const body = JSON.parse(options?.body as string);
    const info = { ...token, name: body.name, scope: body.scope };
    tokens = [...tokens, info];
    return Response.json({ token: secret, info }, { status: 201 });
  }
  if (method === "DELETE") {
    tokens = tokens.filter((entry) => !path.endsWith(entry.id));
    return new Response(null, { status: 204 });
  }
  return Response.json({ enabled, tokens });
});
function expand() {
  const details = screen.getByText("MCP agent access").closest("details")!;
  details.open = true;
  fireEvent(details, new Event("toggle"));
}

beforeEach(async () => {
  enabled = false; tokens = []; fail = false;
  vi.clearAllMocks(); localStorage.clear();
  await i18n.changeLanguage("en");
  vi.stubGlobal("fetch", fetchMock);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("MCP settings", () => {
  it("loads on demand, toggles enabled state, and uses same-origin management requests", async () => {
    render(<McpSettings visible />);
    expect(fetchMock).not.toHaveBeenCalled();
    expand();
    const toggle = screen.getByRole("checkbox", { name: "Enable MCP" }) as HTMLInputElement;
    await waitFor(() => expect(toggle.disabled).toBe(false));
    expect(toggle.checked).toBe(false);
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(true));
    const endpoint = screen.getByLabelText("Streamable HTTP endpoint") as HTMLInputElement;
    expect(endpoint.value).toMatch(/\/mcp$/);
    for (const [input, options] of fetchMock.mock.calls) {
      expect(String(input)).not.toContain(secret);
      expect(options?.credentials).toBe("same-origin");
      expect(options?.cache).toBe("no-store");
      expect(options?.redirect).toBe("error");
      expect(options?.headers).toMatchObject({ "X-LightOS-MCP": "1" });
      expect(options?.headers).not.toHaveProperty("Authorization");
    }
  });

  it("shows a new token once, copies explicitly, and revokes only by public ID", async () => {
    const view = render(<McpSettings visible />);
    expand();
    await screen.findByLabelText("Token name");
    fireEvent.change(screen.getByLabelText("Token name"), { target: { value: "Agent one" } });
    fireEvent.change(document.getElementById("mcp-token-scope")!, { target: { value: "control" } });
    fireEvent.submit(screen.getByRole("form", { name: "Create an MCP token" }));
    const field = await screen.findByRole("textbox", { name: "New MCP token" }) as HTMLTextAreaElement;
    expect(field.value).toBe(secret);
    expect(localStorage.length).toBe(0);
    fireEvent.click(screen.getByRole("button", { name: "Copy token" }));
    await screen.findByRole("button", { name: "Copied" });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(secret);
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("textbox", { name: "New MCP token" })).toBeNull();
    fireEvent.focus(window);
    await waitFor(() => expect((screen.getByRole("button", { name: "Revoke Agent one" }) as HTMLButtonElement).disabled).toBe(false));
    expect(screen.queryByDisplayValue(secret)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Revoke Agent one" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Revoke Agent one" })).toBeNull());
    const deletion = fetchMock.mock.calls.find(([, options]) => options?.method === "DELETE");
    expect(String(deletion?.[0])).toMatch(/api\/mcp\/tokens\/test-token-id$/);
    expect(String(deletion?.[0])).not.toContain(secret);
    view.unmount();
  });

  it("clears plaintext on panel close and reports persistence errors without changing the toggle", async () => {
    const view = render(<McpSettings visible />);
    expand();
    await screen.findByLabelText("Token name");
    fireEvent.change(screen.getByLabelText("Token name"), { target: { value: "Agent one" } });
    fireEvent.submit(screen.getByRole("form", { name: "Create an MCP token" }));
    await screen.findByDisplayValue(secret);
    view.rerender(<McpSettings visible={false} />);
    await waitFor(() => expect(screen.queryByDisplayValue(secret)).toBeNull());
    view.rerender(<McpSettings visible />);
    fail = true;
    fireEvent.click(screen.getByRole("checkbox", { name: "Enable MCP" }));
    await screen.findByRole("alert");
    expect((screen.getByRole("checkbox", { name: "Enable MCP" }) as HTMLInputElement).checked).toBe(false);
    expect(localStorage.length).toBe(0);
  });

  it("does not display a token from a request that finishes after the settings close", async () => {
    const view = render(<McpSettings visible />);
    expand();
    await screen.findByLabelText("Token name");
    let finish!: (value: Response) => void;
    fetchMock.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    fireEvent.change(screen.getByLabelText("Token name"), { target: { value: "Agent one" } });
    fireEvent.submit(screen.getByRole("form", { name: "Create an MCP token" }));
    view.rerender(<McpSettings visible={false} />);
    finish(Response.json({ token: secret, info: token }, { status: 201 }));
    await waitFor(() => expect(screen.queryByDisplayValue(secret)).toBeNull());
  });
});
