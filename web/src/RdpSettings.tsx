import { useEffect, useRef, useState } from "react";
import { Field, Input, Label } from "@headlessui/react";
import { useTranslation } from "react-i18next";
import { useAutoRefresh } from "./use-auto-refresh";

type Status = {
  supported: boolean; available: boolean; enabled: boolean; port: number;
  endpoint: string; loopback: boolean; fingerprint: string | null; error: boolean;
  unavailable_reason: "attached" | "private_state" | "stopping" | null;
};

export function RdpSettings({ visible }: { visible: boolean }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [port, setPort] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const request = useRef<AbortController | null>(null);
  const toggle = useRef<HTMLInputElement | null>(null);
  const pointerToggle = useRef(false);
  const portDirty = useRef(false);

  async function api(settings?: { enabled: boolean; port: number }) {
    if (!settings && request.current) return;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    if (settings || !status) setBusy(true);
    if (settings) setError("");
    try {
      const base = new URL(".", `${window.location.origin}${window.location.pathname}`);
      const response = await fetch(new URL("api/rdp", base), {
        method: settings ? "PUT" : "GET", credentials: "same-origin", cache: "no-store", redirect: "error",
        signal: settings ? controller.signal : AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
        headers: { "X-LightOS-RDP": "1", ...(settings ? { "Content-Type": "application/json" } : {}) },
        ...(settings ? { body: JSON.stringify(settings) } : {}),
      });
      if (!response.ok) throw new Error(response.status === 501 ? "rdpUnsupported" : response.status === 409 ? "rdpAttached" : "rdpError");
      const next = await response.json() as Status;
      if (!controller.signal.aborted) {
        setStatus(next);
        if (settings || !portDirty.current) { setPort(String(next.port)); portDirty.current = false; }
        setError((current) => current === "rdpInvalidPort" && portDirty.current ? current : "");
      }
    } catch (cause) {
      if (!controller.signal.aborted) setError(cause instanceof Error && ["rdpUnsupported", "rdpAttached"].includes(cause.message) ? cause.message : "rdpError");
    } finally {
      if (request.current === controller) { request.current = null; setBusy(false); }
    }
  }

  useEffect(() => {
    return () => { request.current?.abort(); request.current = null; setBusy(false); };
  }, [visible]);
  useAutoRefresh(visible, () => api());

  function validPort() { return /^\d+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535; }
  function toggleEnabled(enabled: boolean) {
    if (!status) return;
    if (enabled && !validPort()) { setError("rdpInvalidPort"); return; }
    void api({ enabled, port: validPort() ? Number(port) : status.port });
  }

  function savePort() {
    if (!status || busy || !visible) return;
    if (!validPort()) { setError("rdpInvalidPort"); return; }
    if (Number(port) !== status.port) void api({ enabled: status.enabled, port: Number(port) });
    else { portDirty.current = false; setError(""); }
  }
  const disabled = !status?.supported || !status.available || busy;
  const unavailable = !status?.supported ? "rdpUnsupported" : status.unavailable_reason === "attached" ? "rdpAttached" : "rdpUnavailable";

  return <section className="settings-section" aria-label={t("rdpTitle")}>
    <h3>{t("rdpTitle")}</h3>
    <p>{t("rdpHelp")}</p>
    {status && (!status.supported || !status.available) && <p role="status">{t(unavailable)}</p>}
    <label className="fit-control"><input ref={toggle} type="checkbox" checked={status?.enabled ?? false} disabled={disabled}
      onPointerDown={() => { pointerToggle.current = true; }}
      onPointerUp={() => { pointerToggle.current = false; }}
      onPointerCancel={() => { pointerToggle.current = false; }}
      onChange={(event) => { pointerToggle.current = false; toggleEnabled(event.target.checked); }}
      onBlur={savePort} />{t("rdpEnabled")}</label>
    <Field className="preference-field"><Label htmlFor="rdp-port">{t("rdpPort")}</Label>
      <Input id="rdp-port" type="number" min="1" max="65535" step="1" inputMode="numeric" value={port} disabled={disabled}
        onChange={(event) => { portDirty.current = true; setPort(event.target.value); }} onBlur={(event) => {
          if (event.relatedTarget !== toggle.current && !pointerToggle.current) savePort();
        }}
        onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); } }} />
    </Field>
    <p>{t("rdpPortHelp")}</p>
    {status && <>
      <Field className="preference-field"><Label htmlFor="rdp-endpoint">{t("rdpEndpoint")}</Label><Input id="rdp-endpoint" readOnly value={status.endpoint} onFocus={(event) => event.target.select()} /></Field>
      <p>{t(status.loopback ? "rdpLoopback" : "rdpNetwork")}</p>
      {status.fingerprint && <Field className="preference-field"><Label htmlFor="rdp-fingerprint">{t("rdpFingerprint")}</Label><Input id="rdp-fingerprint" readOnly value={status.fingerprint} onFocus={(event) => event.target.select()} /></Field>}
      {!status.enabled && <p>{t("rdpDisabled")}</p>}
    </>}
    {(error || status?.error && status.supported && status.available) && <p role="alert">{t(error || "rdpStartupError")}</p>}
    {busy && <p role="status">{t("rdpSaving")}</p>}
  </section>;
}
