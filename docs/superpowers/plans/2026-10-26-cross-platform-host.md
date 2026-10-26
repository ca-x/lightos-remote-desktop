# Cross-platform Host Implementation Plan

> Execute the approved scope continuously using subagent-driven-development for
> independent native backends and inline integration for shared contracts.

**Goal:** Add foreground and service execution on Windows/macOS for logged-in
physical desktops while retaining Linux behavior.

**Architecture:** Keep existing media/display/backend contracts. Introduce
platform-neutral host lifecycle, audio, permission, installer and private-file
contracts; choose native implementations automatically.

**Spec:** ../specs/2026-10-26-cross-platform-host.md

## Global constraints

- Preserve MIT, Linux service names, wire formats and explicit user preferences.
- Use maintained Rust packages; do not copy GPL/AGPL host code.
- No production service or desktop mutations during development.
- Process and service modes share the same capture/transport implementation.
- Screen/input/audio probes must exercise actual capability or report a reason.
- Physical resize, virtual display drivers and login/UAC desktop control remain
  outside this implementation's capability claims.

## Review focus

- Windows Session 0 vs interactive user identity and privilege boundaries.
- macOS permission denial/revocation and launchd user session identity.
- Private tokens/settings and Windows ACLs; no weakening Unix checks.
- Slow consumers, cancellation, lock state and held input release.
- Linux dependency gates, service install/export and native build reproducibility.

## Tasks

- [ ] Define shared host/audio/permissions/service/private-file contracts and
  implement Linux adapters with current callers using those contracts.
- [ ] Implement Windows capture, input, clipboard, permission/session checks and
  foreground/SCM service supervision plus install-service integration.
- [ ] Implement macOS capture, input, clipboard, permission/session checks and
  foreground/LaunchAgent supervision plus install-service integration.
- [ ] Implement portable private settings storage and native dependency gates;
  preserve MCP/RDP restrictions and adapt shared server cursor/audio calls.
- [ ] Add native CI and dependency/license documentation, verify Linux regression
  and native builds, then review and commit the completed implementation.
