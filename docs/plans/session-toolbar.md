# Session toolbar redesign

## Scope

Replace the combined settings panel with a compact action toolbar. Follow RustDesk's separation of display, keyboard and control actions; use the existing Headless UI primitives and Lucide icon library. Keep the remote client, native input safety and saved preferences intact.

## Implementation

1. Put display modes/source selection, keyboard shortcuts, clipboard and connection information in contextual popovers. Keep sound and connection actions directly accessible.
2. Move resolution, encoding/streaming preferences, appearance/language and MCP management into a separate dialog with tabs. Preserve DOM nodes and native client listeners while surfaces are hidden. Clear one-time MCP tokens when their settings surface closes.
3. Retain four-edge drag docking and Alt+Arrow alternatives. Open popovers inward from each edge, constrain their bounds, allow scrolling and keep portals in the fullscreen document tree.
4. Use shared surface, contrast and motion tokens. Pointer-opened popovers use 160 ms opacity/transform transitions; settings use 200 ms. Keyboard actions are immediate; reduced motion removes translation/scaling.
5. Test shell lifecycle, preferences, MCP token clearing, fullscreen and docking. Exercise desktop/mobile/landscape, four edges and corners, focus return/trapping, light/dark contrast and reduced motion in a real browser.
6. Review, bump to 0.1.11, run release gates, merge/push main, safely update only the local transport, and publish verified tag assets. Preserve the persistent desktop owner and GUI processes.

## Reference

Studied RustDesk's `flutter/lib/desktop/widgets/remote_toolbar.dart`, including `_ControlMenu`, display and keyboard submenus and edge docking. This implementation uses the application's existing React components and adds no copied RustDesk code or UI dependencies.


## AUR distribution task

Publish `lightos-remoted-bin` using the organization's existing AUR secrets, following `ca-x/snout`. Keep the executable named `lightos-remote-desktop`. Ship service definitions and automatically initialize/enable/start the service without interactive questions; preserve existing settings and generate a private random initial secret for a fresh install. Updates must not restart the persistent desktop owner or GUI applications. Verify fresh-install dependencies and compositor compatibility instead of assuming a binary/library is usable. AUR deployment follows a published, verified GitHub tag and must ship checksummed sources, .SRCINFO and an exact asset allowlist. Do not put .SRCINFO in the GitHub source tree or expose credentials.


## Final naming and migration decision

The user subsequently approved a new layout instead of preserving the old current path: public command `lightos-remoted`, packaged binary `/usr/lib/lightos-remoted/lightos-remoted`, core release archive `lightos-remoted-linux-x86_64.tar.gz`, new transport/owner units `lightos-remoted.service` and `lightos-remoted-session.service`, configuration `/etc/lightos-remoted/config.env`, state `/var/lib/lightos-remoted`, runtime `/run/lightos-remoted` and `/run/lightos-remoted-session`. Existing legacy installations are preserved and get explicit README cleanup/migration instructions; package hooks must not stop old desktop applications or start a conflicting server. The old `install-service`/rollback implementation remains only for legacy compatibility. Main executable build/CLI/CI/release/AUR references use the new name. MIT is the final license decision and supersedes the briefly proposed AGPL option.


## LightOS service compatibility (final user decision)

Inspection of the original Python distribution's readable wrapper scripts found the service name and authentication environment path hardcoded by LightOS. User explicitly decided to keep both `lightos-remote-desktop.service` and `lightos-remote-desktop-session.service` unchanged. No new service-name aliases are required. Keep `/etc/lightos/remote-desktop.env` as the regular canonical authentication config and preserve its existing nonroot runtime user. The new binary/command/archive names and `/usr/lib/lightos-remoted` payload remain as agreed. Do not remove `/opt/lightos/remote-desktop` or LightOS platform setup scripts: its original setup still checks the Selkies runtime payload. This supersedes earlier proposed new unit/config canonical names.
