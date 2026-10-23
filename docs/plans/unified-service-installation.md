# Unified service installation implementation plan

> **For agentic workers:** Use subagent-driven-development or executing-plans to implement these tasks with independent review.

**Goal:** Make the binary authoritative for both systemd services, standalone installation and AUR lifecycle.

**Architecture:** Embed one pair of complete unit definitions plus shared bootstrap/activation resources in `lightos-remoted`. AUR generates its pacman-owned units from the release executable and calls its lifecycle commands; it does not maintain independent unit/bootstrap copies. Retain legacy transactional migration explicitly where needed rather than silently stopping a running old desktop.

**Tech Stack:** Existing Rust/clap, Bash, systemd, makepkg and GitHub Actions. No new dependency is needed.

**Spec:** Latest user requirements in this session and `docs/plans/session-toolbar.md`; this plan supersedes its old installer/AUR implementation split.

## Global constraints

- Both service names and `/etc/lightos` authentication remain unchanged; executable path `/usr/lib/lightos-remoted/lightos-remoted`, CLI `/usr/bin/lightos-remoted`.
- `install-service` supports a fresh system and creates both services automatically; normal upgrades preserve owner/applications and restart only transport.
- Binary owns definitions and lifecycle. AUR owns installed files through pacman and invokes binary lifecycle; no root build.
- Preserve nonroot UID validation, known-good recovery after consecutive failures, credentials, native settings and legacy desktop safety.
- Dry-run and unit export do not write state or require root. Test roots never execute real systemd/account commands. Do not modify this host's production installation.

## Review focus

Package-owned files must never be overwritten by a standalone installer; old custom units/drop-ins must not be silently deleted; doctor failure must retain known-good rollback; UID aliases must not allow root; attached physical desktops must not accidentally start a virtual owner.

## Task 1: Binary-owned service definitions and standalone installer

Files: `src/install.rs`, `scripts/service-*.sh`, `systemd/lightos-remoted-*.service`, `tests/cli_install.rs`, bootstrap fixtures; keep legacy script/resources for explicit legacy compatibility where necessary.

- [ ] Add regressions for fresh install with both units/new paths, mutation-free export/dry-run, same-owner transport upgrades, rollback, package ownership refusal and legacy preservation.
- [ ] Implement `install-service --print-unit transport|session` (stdout only, no root), `--package-managed` (activate package-owned payload), `--snapshot`, `--remove`; package lifecycle flags mutually exclusive. Default installs the binary and both complete units then invokes the same bootstrap. Preserve optional sibling helper and prior payload; define rollback for the new layout. Retain explicit legacy compatibility for existing CLI integration tests as needed.
- [ ] Move shared resources out of AUR-only directory and embed them. Keep stage/test-root operation isolated and update existing tests without dropping their guarantees.
- [ ] Verify Rust installer tests, bootstrap fixtures, legacy installer fixtures, shellcheck and formatting; commit a self-contained implementation.

## Task 2: AUR consumes release executable

Files: `lightos-remoted-bin/PKGBUILD`, install hook, `.github/workflows/aur.yml`, prune/release fixtures and `docs/aur.md`.

- [ ] Generate both unit files from the release binary's `--print-unit` interface during packaging and install them as pacman-owned files.
- [ ] Replace bootstrap/script/unit AUR assets with lifecycle calls into installed binary; preserve snapshot, removal and activation semantics.
- [ ] Verify source metadata, lifecycle fixtures, allowlist and eventual actual released binary package. Commit after Task 1 interfaces are available.

## Task 3: Bilingual documentation and release

Files: `README.md`, `README.en.md`, release notes, CI archive inputs and plan contract.

- [ ] Keep complete Chinese/English README content, commands and migration conditions aligned with mutual links. Include both README files in release archives.
- [ ] Independently review installer/AUR integration, run covering tests and exact-SHA CI, then merge main and publish v0.1.11 plus AUR. User installs production themselves.

## Task 4: RDP settings entry

User requirements: RDP belongs in the independent Settings dialog, with an enable switch and port field only. RDP picture quality remains client/protocol negotiated; browser streaming preferences must not affect RDP.

Files: `src/rdp*`, RDP state wiring in `src/main.rs` / `src/server.rs`, `web/src/RdpSettings.tsx`, settings tab and translations, focused tests, `docs/rdp.md`.

- [ ] Add authenticated same-origin GET/PUT `/api/rdp`, private atomic persistence, strict port validation and unsupported/attached-desktop responses.
- [ ] Manage listener changes without restarting HTTP or the desktop. Prepare TLS/bind/save before replacing the old service; failures retain the previous configuration/listener. Shutdown releases held input. Preserve existing explicit TLS configuration; otherwise generate and persist a private self-signed identity with an established Rust crate. Retain the current bind-address policy and show the real endpoint/fingerprint.
- [ ] Add Settings tab with two editable controls, visible status/errors and canceled pending UI requests when hidden. No toolbar action or RDP quality control.
- [ ] Run focused backend/component tests and static checks, then independent review. User requested unit tests for this increment rather than repeated full builds; the final release workflow builds tag artifacts.
