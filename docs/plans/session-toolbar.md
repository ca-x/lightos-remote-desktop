# Session toolbar and AUR delivery plan

## Requirements

- Replace the combined settings panel with a compact toolbar inspired by RustDesk's display, keyboard and control action groups. Use existing Headless UI primitives and Lucide icons; add no parallel UI library.
- Put display modes/source selection, keyboard shortcuts, clipboard and connection information in contextual popovers. Keep sound and connection actions directly accessible.
- Put resolution, encoding/streaming preferences, appearance/language and MCP management in a separate tabbed dialog. Preserve DOM nodes and client listeners while hidden. Clear one-time MCP tokens and abort pending requests when leaving their settings surface.
- Retain four-edge dragging and Alt+Arrow alternatives. Menus open inward, avoid viewport edges, remain scrollable and close with their parent toolbar. Fullscreen includes portalled UI. Measure layout without animation transforms.
- Use coherent dark/light surface tokens, 44px toolbar targets and visible focus. Pointer-opened popovers use 160ms opacity/transform transitions and settings use 200ms; keyboard actions are immediate and reduced motion removes scaling.

## Distribution contract

- Version: 0.1.11. License: MIT; third-party licenses remain separate.
- Public binary/command: `lightos-remoted`; core archive: `lightos-remoted-linux-x86_64.tar.gz`.
- AUR package: `lightos-remoted-bin`, using the organization's existing AUR secrets and the snout release workflow as reference.
- Package payload: `/usr/lib/lightos-remoted/lightos-remoted`, command entry `/usr/bin/lightos-remoted`; the optional `lightos-codec-ffmpeg` retains its name and requires a matching host ABI.
- Preserve LightOS service names `lightos-remote-desktop.service` and `lightos-remote-desktop-session.service`, plus canonical authentication configuration `/etc/lightos/remote-desktop.env`. Respect an existing valid nonroot runtime user and secret.
- AUR fresh installs initialize and start services automatically. Upgrades restart only the transport and preserve the persistent desktop owner. Legacy hand-installed instances stay running until the user explicitly follows README migration instructions.
- README includes backup, explicit application shutdown, removal of old local unit overrides, activation and conditional cleanup. Preserve LightOS-owned configuration, platform scripts and `/opt/lightos/remote-desktop`: its original setup still checks the Selkies payload.
- Publish AUR only after a real stable GitHub release and verified assets. Generate real checksums and .SRCINFO in the AUR deployment checkout, not the GitHub source tree. Do not expose credentials.

## Validation and delivery

1. Check shell lifecycle, preferences, MCP token clearing, fullscreen, docking and untransformed layout.
2. Exercise all four edges and corners, desktop/mobile/landscape, focus return/trapping, dark/light contrast and reduced motion in a real browser.
3. Verify native binary assets/capture and installer behavior; test AUR bootstrap and release guards without touching host services, users or configuration.
4. Independently review UI and packaging, then the integrated branch. Run frontend/Rust/format/Clippy and packaging/workflow checks.
5. Commit/push, merge main, create a tag, download and verify final tag archives, publish stable and verify AUR Git/RPC. The user will install the AUR package themselves; do not update or migrate this host’s production service.

## Reference

Studied RustDesk's `flutter/lib/desktop/widgets/remote_toolbar.dart`, including `_ControlMenu`, display and keyboard submenus and edge docking. No RustDesk source code was copied.
