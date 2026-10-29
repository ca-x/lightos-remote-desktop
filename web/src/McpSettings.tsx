import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button, Field, Input, Label } from "@headlessui/react";
import { useTranslation } from "react-i18next";
import { useAutoRefresh } from "./use-auto-refresh";
import { Select } from "./Select";
import { ScrollTextarea } from "./ScrollArea";

type Scope = "observe" | "control";
type Token = { id: string; name: string; scope: Scope; created_at: number };
type Status = { enabled: boolean; tokens: Token[] };

export function McpSettings({ visible, embedded = false }: { visible: boolean; embedded?: boolean }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(embedded);
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [name, setName] = useState("");
  const [scope, setScope] = useState<Scope>("observe");
  const [fresh, setFresh] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const request = useRef<AbortController | null>(null);
  const base = new URL(".", `${window.location.origin}${window.location.pathname}`);
  const endpoint = new URL("mcp", base).href;

  useEffect(() => {
    if (!visible || !expanded) {
      request.current?.abort();
      request.current = null;
      setFresh(null);
      setCopied(false);
      setBusy(false);
    }
  }, [visible, expanded]);
  useEffect(() => () => request.current?.abort(), []);
  useAutoRefresh(visible && expanded, refresh);

  async function api(path: string, method: string, body?: object) {
    if (method === "GET" && request.current) return undefined;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    if (method !== "GET" || !status) setBusy(true);
    if (method !== "GET") setError("");
    try {
      const response = await fetch(new URL(path, base), {
        method, credentials: "same-origin", cache: "no-store", redirect: "error",
        signal: method === "GET" ? AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]) : controller.signal,
        headers: { "X-LightOS-MCP": "1", ...(body ? { "Content-Type": "application/json" } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {}),
      });
      if (!response.ok) throw new Error(String(response.status));
      const result: unknown = response.status === 204 ? null : await response.json();
      if (controller.signal.aborted) return undefined;
      setError((current) => current === t("mcpError") ? "" : current);
      return result;
    } catch {
      if (!controller.signal.aborted) setError(t("mcpError"));
      return undefined;
    } finally {
      if (request.current === controller) { request.current = null; setBusy(false); }
    }
  }

  async function refresh() {
    const result = await api("api/mcp", "GET");
    if (result) setStatus(result as Status);
  }
  async function toggle(enabled: boolean) {
    setFresh(null);
    const result = await api("api/mcp", "PUT", { enabled });
    if (result) setStatus(result as Status);
  }
  async function create(event: FormEvent) {
    event.preventDefault();
    setFresh(null);
    setCopied(false);
    const result = await api("api/mcp/tokens", "POST", { name: name.trim(), scope }) as { token: string; info: Token } | undefined;
    if (result && status) {
      setStatus({ ...status, tokens: [...status.tokens, result.info] });
      setFresh(result.token);
      setName("");
      await refresh();
    }
  }
  async function revoke(id: string) {
    const result = await api(`api/mcp/tokens/${encodeURIComponent(id)}`, "DELETE");
    if (result === null && status) {
      setStatus({ ...status, tokens: status.tokens.filter((token) => token.id !== id) });
      setFresh(null);
      await refresh();
    }
  }
  async function copy() {
    if (!fresh) return;
    try { await navigator.clipboard.writeText(fresh); setCopied(true); }
    catch { setError(t("mcpCopyError")); }
  }

  return <details id="mcp-section" className={embedded ? "embedded-mcp" : undefined} open={embedded ? true : undefined} onToggle={(event) => {
    const opened = event.currentTarget.open;
    setExpanded(opened);
  }}>
    <summary hidden={embedded}>{t("mcpTitle")}</summary>
    {embedded && <h3>{t("mcpTitle")}</h3>}
    <p>{t("mcpHelp")}</p>
    <div className="mcp-heading">
      <label className="fit-control"><input type="checkbox" checked={status?.enabled ?? false} disabled={!status || busy} onChange={(event) => { void toggle(event.target.checked); }} />{t("mcpEnabled")}</label>
    </div>
    <Field className="preference-field"><Label htmlFor="mcp-endpoint">{t("mcpEndpoint")}</Label><Input id="mcp-endpoint" readOnly value={endpoint} onFocus={(event) => event.target.select()} /></Field>
    {error && <p role="alert">{error}</p>}
    {status && <>
      <form className="mcp-create" onSubmit={(event) => { void create(event); }} aria-label={t("mcpCreateForm")}>
        <Field><Label htmlFor="mcp-token-name">{t("mcpTokenName")}</Label><Input id="mcp-token-name" value={name} maxLength={80} autoComplete="off" disabled={busy} onChange={(event) => setName(event.target.value)} required /></Field>
        <Field><Label htmlFor="mcp-token-scope-trigger">{t("mcpScope")}</Label><Select id="mcp-token-scope" aria-label={t("mcpScope")} value={scope} disabled={busy} onChange={(event) => setScope(event.target.value as Scope)}><option value="observe">{t("mcpObserve")}</option><option value="control">{t("mcpControl")}</option></Select></Field>
        <Button type="submit" disabled={busy || !name.trim() || status.tokens.length >= 32}>{t("mcpCreate")}</Button>
      </form>
      {fresh && <div className="mcp-secret" role="region" aria-label={t("mcpNewToken")}>
        <p>{t("mcpOnce")}</p>
        <ScrollTextarea aria-label={t("mcpNewToken")} readOnly rows={3} value={fresh} spellCheck={false} autoComplete="off" onFocus={(event) => event.target.select()} />
        <div className="toolbar"><Button type="button" onClick={() => { void copy(); }}>{t(copied ? "mcpCopied" : "mcpCopy")}</Button><Button type="button" onClick={() => setFresh(null)}>{t("mcpDone")}</Button></div>
      </div>}
      <ul className="mcp-tokens" aria-label={t("mcpTokens")}>
        {status.tokens.map((token) => <li key={token.id}>
          <div><strong>{token.name}</strong><span>{t(token.scope === "control" ? "mcpControl" : "mcpObserve")} · {new Date(token.created_at * 1000).toLocaleDateString()}</span></div>
          <Button type="button" className="quiet-button" disabled={busy} aria-label={t("mcpRevokeNamed", { name: token.name })} onClick={() => { void revoke(token.id); }}>{t("mcpRevoke")}</Button>
        </li>)}
      </ul>
      {!status.tokens.length && <p>{t("mcpNoTokens")}</p>}
    </>}
  </details>;
}
