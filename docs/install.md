# Service installation

## Current standalone and AUR installation

The executable is the source of truth for both service definitions and their
lifecycle. `sudo ./lightos-remoted install-service` supports a fresh system: it
installs `/usr/lib/lightos-remoted/lightos-remoted`, the `/usr/bin/lightos-remoted`
entry and both complete vendor units, initializes a nonroot account and private
authentication if absent, then starts/enables the services. Existing credentials
and native settings are preserved. Desktop dependencies and any required private
Aquamarine library must already be installed; this command does not fetch them.

```sh
./lightos-remoted install-service --dry-run
sudo ./lightos-remoted install-service
sudo ./lightos-remoted rollback-service
```

`--dry-run` is a rootless, read-only plan; `--stage-only` deploys files without
initializing accounts/credentials or starting services. Standalone transactions
are retained under `/var/lib/lightos-remoted-install`; activation/recovery state
uses `/var/lib/lightos-remoted-package`. Normal upgrades retain the desktop
owner and restart only transport. A first-install rollback requires explicitly
saving work, stopping both services and using `rollback-service --stage-only`,
because it removes the newly installed service files. Rollback retains credentials.

AUR uses `install-service --print-unit transport` and `--print-unit session` to
obtain the same embedded units, which pacman owns. Its lifecycle hook calls the
installed executable with `--package-managed`, `--snapshot` or `--remove`.
Standalone installation refuses package-owned files. Removal disables future
startup of both services without stopping their currently running processes.
See [AUR details](aur.md) for installation and legacy migration.

Virtual mode starts/enables the owner before activating transport; an already
running owner is retained. Attached physical mode does not start a virtual
owner, and disables its future startup without stopping a previously running
owner. Save work before deliberately switching or ending that existing desktop.
The transport unit orders after the owner but does not unconditionally require it.
Its packaged session runtime is a fallback default; an explicit Wayland or X11
attachment takes precedence without a conflicting persistent-session argument.
Both attachment types disable future virtual-owner startup while preserving an
already-running owner. Recovery of an older owned transport retains the legacy
session-runtime environment so it reconnects to the existing owner.

Custom or legacy `/etc/systemd/system` units/drop-ins are not silently replaced.
Back them up and follow the README migration steps. The old migration path
remains available explicitly through `install-service --legacy` and
`rollback-service --legacy`. The rest of this document describes that legacy
transactional layout and compatibility-library provisioning.

## Capture backends and runtime libraries

Official binaries enable `bundled-jpeg,rdp,portal`. Portal support links
`libpipewire-0.3.so.0` even when private audio is disabled. Building that feature
requires the PipeWire/SPA development headers, pkg-config and libclang in
addition to the existing Wayland, XKB, Pulse and codec build tools. CI installs
these dependencies before compiling; AUR already includes PipeWire. A custom
build without `portal` still supports native Wayland and X11 and reports Portal
as unavailable instead of advertising a nonfunctional backend.

Owned virtual sessions retain their Cage/Hyprland/D-Bus dependencies. Attaching
to an existing session does not start those programs. Native Wayland clipboard
uses optional `wl-copy`/`wl-paste`; X11 authentication requires `xauth`, with
capture and XTest input checked on the exact display. `doctor` separates
required startup dependencies from optional clipboard and shell helpers. A
successful probe still requires a real first frame before the backend is ready.
See [runtime platforms](runtime-platforms.md) and [desktop switching](desktop-switching.md).

## Legacy migration reference

The executable embeds this transactional installer. On a provisioned LightOS
machine, `sudo ./lightos-remoted install-service --legacy` replaces the existing
Python service command and activates Rust after dependency and health checks.
Use `install-service --legacy --dry-run` for read-only checks, `--stage-only` to avoid
restarting, and `sudo ./lightos-remoted rollback-service --legacy` for the previous
transaction. Root is needed only for installation, never for desktop runtime.
This does not bootstrap Cage/Hyprland or the version-specific Aquamarine patch.

The installer does not build, download, install packages, edit the original unit,
or remove the Python launcher or `/opt/lightos/remote-desktop` payload. Use a
reviewed local release executable. Native video negotiates H.264/WebCodecs with
SIMD JPEG fallback over WebSocket, **not hardware/zero-copy performance parity**.
H.264 uses source-built OpenH264 and needs browser HTTPS/localhost support. Audio, clipboard, Unicode,
resize and disconnect cleanup require separate real desktop verification.

Downloaded executables embed the compiled React frontend; do not install Node.js
or run a web development server on the target. When compiling from source, run
`npm ci && npm run build` before Cargo. CI follows this order and includes
frontend license notices in release archives. The appearance section remembers
language/theme locally and never changes the remote desktop's own theme.

Native capture uses standard Wayland protocols/libwayshot, with **Cage/wlroots**
as the parent compositor for nested Hyprland. Install the system `cage` and
Hyprland packages; Weston is not the selected parent due to protocol-version
incompatibility. The pinned private Aquamarine 0.15.1/SONAME 14 compatibility
build has produced a real native desktop frame and HTTP health 200 on separate
port 18766. That candidate also passed static-resource auth 401/200 and
foreign-origin rejection 403 checks. These separate-port results do not imply
host service cutover or complete feature/performance parity. libxkbcommon
is a build/link dependency; media/clipboard support also depends on external
tools. Use `doctor` for configuration/dependency checks, then test an actual
session separately. The installer copies only the selected application
executable, never system dependencies or extracted smoke-test trees.

With the default `LIGHTOS_REMOTE_DESKTOP_SHELL=auto`, an
installed Omarchy Shell also requires `quickshell` and `gtk-launch`; terminal
selection comes from the native configuration.
It restores the user's wallpaper, bar and app menu without running the host
Omarchy provisioning or importing remote variables into the host user manager.
Use `LIGHTOS_REMOTE_DESKTOP_SHELL=minimal` for the terminal-only fallback.

Audio requires the system `pipewire`, `pipewire-pulse`, `wireplumber`, `libpulse`
and `pactl` tools. The selected design uses an owned per-desktop PipeWire graph,
private Pulse-compatible server and WirePlumber `lightos` profile with hardware
monitors disabled, started after the private session D-Bus. It must not depend
on shared host audio, host Pulse authentication or copied host cookies. No manual
user-group changes are required. The private-audio prototype captured more than
76 KB of PCM; the integrated runtime still requires installed WebSocket audio
validation. Doctor/HTTP health alone does not prove working audio.
The native-media binary links `libpulse.so.0` even with audio disabled;
install the runtime library before replacing an older executable.

## Provision the private compatibility library first

Build as the desktop user using `scripts/build-aquamarine.sh`, then inspect and
provision its output before installing/activating the service:

```sh
bash scripts/install-compat.sh --check
bash scripts/test-install-compat.sh
# Operator action only, after review:
sudo bash scripts/install-compat.sh
```

The default build directory is `.local/aquamarine/build`; override it with
`--build-dir DIR`. This separate installer requires its `aquamarine.pc`,
`compatibility.txt` build receipt, `libaquamarine.so.0.15.1`, the repository's
recorded patch, and a nonempty upstream LICENSE. LICENSE is inferred from
`build/../LICENSE` or supplied explicitly with `--license FILE` (use the notice
from the pinned source, not a substitute). It verifies pkg-config version
0.15.1, ELF SONAME `libaquamarine.so.14`, receipt version/upstream/pinned commit
`f31c47a1b9d300847d8fc3108ad959448103dfc1`, and recorded library/patch SHA-256
checksums without evaluating receipt contents or executing the library. Receipts
prove local consistency, not signed upstream authenticity; review the pinned
source/patch and trust the build input before running the installer as root.

It atomically publishes a root-owned, content-addressed directory:

```text
/usr/local/lib/lightos-remote-desktop/compat/<library-sha256>/
  libaquamarine.so.14
  LICENSE
  receipt.txt
```

The receipt records version, upstream, commit, SONAME, library/patch checksums
and the copied LICENSE checksum. Library/directory modes are 0755; notice and
receipt modes are 0644. Missing, altered or symlinked notice/receipt files in an
existing release fail checks. Keep the BSD-3-Clause LICENSE/copyright notice
with any redistributed library; this script does not replace wider dependency
license review. It never writes `/usr/lib`, changes the system linker cache,
modifies the original secret env, executes a library/candidate or calls systemd.

The compat installer sets exactly one
`LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR` assignment to the stable directory in
`/etc/lightos/remote-desktop-native.env`, replacing duplicate assignments for
that key while preserving unrelated single-line assignments and comments.
It never sources/evaluates env text. Multiline values, line continuations and
unsupported syntax are refused without rewriting the file; simplify them
manually first. The resulting file is root-owned mode 0600. Existing native env
is backed up under a private `compat-transaction.*` directory in
`/var/lib/lightos-remote-desktop`; the shared installer lock prevents concurrent
installer writes. Idempotent provisioning preserves the prior rollback backup.

`--check`/`--dry-run` are read-only and validate inputs, existing releases and env
syntax. `--test-root EXISTING_TEMP_DIRECTORY` prefixes all deployment paths and
allows non-root tests without host changes. The test suite builds a tiny ELF
fixture with `cc` and never loads it. To restore the previous native env:

```sh
sudo bash scripts/install-compat.sh --rollback --check
sudo bash scripts/install-compat.sh --rollback
```

Rollback is consumed once, restores the exact backup (or removes a newly created
native env), retains immutable compat releases and never restarts the service.
Use it only when the running candidate no longer needs the new path, or coordinate
an explicit service rollback separately. Prepared backup metadata is published
before the native env rename, allowing explicit rollback after interruption;
ordinary errors/signals restore the old env and transaction pointer automatically.
Power loss/storage failures still require operator inspection.

## Plan and validate first

```sh
bash scripts/install.sh --check
bash scripts/install.sh --dry-run --binary /absolute/path/to/lightos-remoted
bash -n scripts/install.sh
bash -n scripts/activation.sh
bash -n tests/installer.sh
bash tests/installer.sh
```

Default input is `target/release/lightos-remoted`; `--binary` overrides it.
`--check` and `--dry-run` are equivalent read-only filesystem/prebuilt checks,
not runtime validation: they do not run doctor or contact systemd. A missing
existing system unit is an error. This is a migration installer, not a fresh
system provisioning tool.

Before cutover, run the candidate as the desktop user on a separate port and
private runtime directory, using the runtime's documented environment keys.
Verify real capture/input, authenticated HTTP/WebSocket, foreign-origin
rejection and `/healthz`. A successful doctor or systemd active state alone is
not proof of a working desktop. See `verification.md` for the recorded host
cutover and real desktop acceptance results.

## Existing configuration

Keep `/etc/lightos/remote-desktop.env`. The installer never creates, sources,
evaluates, prints, or rewrites it. `config/example.env` is a reference, not an
automatically installed configuration; its empty password intentionally cannot
activate. Supply a non-default secret privately, never in command arguments or
version control. Retain the original `LIGHTOS_REMOTE_DESKTOP_*` keys, including
`ENABLE`, `USER`, `PORT`, `AUTH_USER`, `AUTH_PASSWORD`. Default service port: 8766.
Use systemd EnvironmentFile syntax, not shell programs, substitutions or
`export` statements. The Rust runtime consumes systemd-exported values.

An optional `/etc/lightos/remote-desktop-native.env` is loaded **after** the
original env file by the native service and transient doctor/probe units. It is
never created, sourced, printed or overwritten by `install.sh`. The separate
`install-compat.sh` provisions its Aquamarine setting as described above. Existing
contents are included in transaction/first-install backups, but rollback leaves
both env files and subsequent operator edits untouched. Missing native config
is allowed. Keep credentials in the original file; this native-only file is for
settings such as the separately provisioned private Aquamarine directory:

```ini
LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/absolute/path/to/validated/private/lib
```

Use an operator-owned, stable deployment path, not an ignored smoke-test
extraction. Provision root-owned configuration privately (mode 0600); systemd
loads it before changing User. Later env-file assignments override earlier
ones, so avoid duplicating authentication/port keys unintentionally. Restoring
the original Python drop-in removes this additional env-file inclusion.

The existing unit retains `User=czyt`, `RuntimeDirectory=lightos-remote-desktop`,
`RestartSec=3`, and other original settings except the native-specific overrides
described here. Its `[Unit]` description is `LightOS Native Rust Remote Desktop`
so status no longer identifies the native process as Selkies. The managed drop-in resets
`ExecStart` to `current/lightos-remote-desktop serve`, requires the existing
environment file, loads the optional native env file and supplies the runtime
directory environment. Other local
drop-ins remain untouched; review `systemctl cat lightos-remote-desktop.service`
for conflicting overrides before activation. Do not change the service to root.

The native drop-in does not set or reset `SupplementaryGroups`; inherited GPU
group settings in `10-lightos-render-groups.conf` remain effective and untouched,
alongside normal NSS account memberships. Audio uses the private session stack
described above, not a supplementary-group workaround for host Pulse access.
No account memberships, host audio authentication data or permissions are changed.

## Stage and explicitly activate (root required)

```sh
sudo bash scripts/install.sh --binary /absolute/path/to/lightos-remoted
# After separate-port real desktop acceptance and review:
sudo bash scripts/install.sh --binary /absolute/path/to/lightos-remoted --activate
```

Staging does not call systemctl, restart or enable the service. It DOES change
the on-disk drop-in/current symlink; an unrelated daemon-reload/restart can pick
it up. Do not stage on the live host before acceptance tests. These are operator
instructions only; this installer work does not perform host cutover.

Activation refuses a missing/empty authentication secret and runs the exact
candidate's `doctor` through a transient systemd unit with the existing service
User, both environment files and a separate private runtime directory. The
runtime doctor must reject default/invalid credentials and configuration; the
installer never interprets the env file as shell. Only after doctor succeeds
does it reload/restart and run the live health/PID/restart gate described below.
No `enable` is performed.

### Live probe interface (required from the runtime)

The installer requires this additional binary subcommand; it does not implement
the HTTP client itself or silently fall back to `is-active`:

```text
lightos-remote-desktop probe --expect-pid <positive PID>
```

`probe` must read the same systemd-exported `LIGHTOS_REMOTE_DESKTOP_*` port/auth
keys as `serve`, never source an env file, and connect only to literal IPv4
`127.0.0.1:<configured port>` (default 8766), `GET /healthz`. No proxy, redirects,
DNS target, URL credentials, or supplied remote URL. Use Basic Auth in a header
if required, never log credentials. Its monotonic deadline must cover connect,
reads and retries, with bounded response size. Success requires HTTP 200 plus a
JSON object with boolean `native: true`, boolean `ready: true` and numeric `pid`
equal to `--expect-pid`; malformed/non-native/wrong-process responses cannot succeed.
The runtime must return 200 only for live native capture, not just a listener or
a historical frame. Exit 0 for readiness and nonzero for timeout, invalid data,
unsupported flags or any other failure; stdout is not parsed by the installer.
The runtime implements a fixed 35-second readiness deadline; the installer
always supplies `--expect-pid` even though the runtime accepts it optionally.
No unsupported `--expected-pid` or `--timeout-secs` flags are passed. This
installer suite does not claim real-session validation of the runtime probe.

After restart the installer obtains `MainPID`, `ExecMainPID`, `NRestarts`,
`ActiveState` and `SubState` in a single systemd property read. It requires a
positive, matching main/exec PID different from the pre-restart PID, and active/
running state. The exact release runs `probe` under the same non-root User and
environment files (original required, native optional), in a transient unit
with a 40-second runtime limit and 45-second client timeout. Initial readiness
uses the runtime's 35-second probe deadline, followed by five one-second samples
with unchanged PID/restart count and a final live probe with the same bounded
deadline. Every sample
must remain active/running; a restart, PID change or stale health response fails
the gate. Systemd client operations are timeout-bounded too. Missing `probe`
support fails closed and triggers rollback. This is a bounded cutover gate,
not long-term monitoring or a substitute for real desktop acceptance tests.

Releases are immutable SHA-256 directories:
`/usr/local/lib/lightos-remote-desktop/releases/<sha>/lightos-remote-desktop`.
The relative `current` symlink and drop-in are replaced using same-filesystem
renames. A private flock serializes installers. Each transaction backs up the
prior symlink, managed override, original unit and both existing env files beneath
`/var/lib/lightos-remote-desktop/transaction.*`; `original/` retains the first
snapshot. Backups contain secrets and must remain root-only. Identical staging
is a no-op and keeps the preceding rollback snapshot.

On installation/restart/health failure, the EXIT/signal handler stops the failed
candidate before restoring the preceding symlink and managed override. If
activation was attempted it reloads systemd and restarts the prior service when
previously active, checking five seconds of stable PID/restart state; otherwise
it leaves it stopped. A recovery failure is reported explicitly and retains
the transaction for retry rather than marking rollback consumed. Recovery of
the Python payload does not run the native probe or claim native readiness.
No script can recover automatically from SIGKILL, power loss or storage failure;
the two renames are not a cross-file ACID transaction. A prepared backup and
`latest` pointer are published before either live path changes, so use the
explicit rollback command after interruption (even if no `complete` marker was
written). Inspect journals and
health before declaring success; automatic recovery can itself fail.

## Rollback

```sh
sudo bash scripts/install.sh --rollback --check
sudo bash scripts/install.sh --rollback
# Or restore and restart the previous service immediately:
sudo bash scripts/install.sh --rollback --activate
```

Rollback restores the last prepared installation's previous symlink and
managed override (or removes them when originally absent). It leaves env/unit
edits and original Python payload untouched. Rollback is consumed once; choose
the activating form initially if desired. If already rolled back without
activation, explicitly reload/restart the service as operator. Reverting to the
original Python launcher does not fix its known UWSM compatibility failure.
Rollback activation stops the candidate, restores disk state, restarts the
previous service and checks five seconds of stable PID/restart state. It does
not run `doctor` or native `probe`: the preserved Python launcher has no such
CLI. Retained releases/backups are not garbage-collected.

## Isolated installer tests

`--test-root /absolute/existing/temp-directory` prefixes all deployment paths,
permits non-root filesystem testing, never executes a binary, and forbids
`--activate`. It never invokes systemctl/systemd-run, even if present in PATH.
Use only disposable directories with a fixture system unit. The behavioral
suite checks preservation, idempotence, upgrade/rollback, injected write
failure, read-only checks and unsafe path refusal. It separately sources the
activation helpers with mock systemd/probe/sleep boundaries to exercise live
gate failures and recovery without executing a candidate or contacting host
systemd. It does not claim to test the runtime HTTP implementation, real
systemd activation or native desktop behavior.

## Persistent desktop session

For virtual-desktop deployments, version 0.1.6 installs an independent `lightos-remote-desktop-session.service`
and a `95-session.conf` transport drop-in. The session unit uses the same
non-root user (named or numeric UID) as the existing service and retains normal `KillMode=control-group`
cleanup. The transport requires the session service but the session has no
`PartOf`/`BindsTo` relationship back to the transport. Updating or restarting the
transport therefore leaves the desktop's cgroup alive. Both managed files are
included in installer transaction snapshots and rollback. Existing attached
physical-desktop deployments omit the owner unit and persistent-runtime override.

The owner uses `/run/lrd-session` (0700) and atomically publishes `session.json`
(0600) after desktop startup. Attachment checks descriptor protocol, process
identity/start time, file/socket ownership and the owner's exclusive lock.
Missing descriptors wait up to 30 seconds for first startup. Invalid or
incompatible descriptors fail visibly and never replace a live desktop.
A transport update does not hot-reload the desktop owner's executable or config;
those changes take effect on an explicit session restart.

The first migration from the combined service terminates that old desktop once.
Applications launched afterward survive transport updates. Rollback to a version
before 0.1.6 cannot reconnect to the new descriptor; it creates its legacy desktop
and does not expose windows retained by the independent session. Do not stop the
session owner merely to roll back a transport: that closes its applications.

```sh
systemctl status lightos-remote-desktop.service lightos-remote-desktop-session.service
# Reconnect transport while retaining GUI apps:
sudo systemctl restart lightos-remote-desktop.service
# Explicitly reset the desktop; this closes its applications:
sudo systemctl restart lightos-remote-desktop-session.service
```

Standalone `serve` continues to own and clean up its desktop unless
`--session-runtime DIR` is supplied. For manual lifecycle testing, start
`desktop-session --runtime-dir DIR` separately and connect `serve --session-runtime DIR`.
The owner needs a short absolute runtime path (maximum 28 bytes for Hyprland IPC).
Attached physical desktop mode remains non-owning and does not resize the host.

## Architecture and local verification

`crates/platform-detect` reads distribution identity, resolves regular executable
files and locates installed Omarchy configuration. `crates/desktop-config`
renders the remote Lua entry or reads native conf includes, excluding host
monitor/autostart directives. `src/desktop` exposes `DesktopInitializer`, with
`GenericInitializer` and `OmarchyInitializer` producing a shared startup plan.
Session ownership and descriptor handoff are separate from these strategies.
No startup fetches GitHub configuration or maintains copied Omarchy keybindings.

Native Lua source/require files stay referenced directly. Legacy conf includes
are read into a private generated file at owner startup; subsequent changes
require a session restart. Unknown include variables, include cycles and invalid
configurations produce diagnostics rather than a silently different desktop.
Standard Omarchy physical-monitor/autostart modules are omitted remotely; custom
Lua outside those modules executes as configured by the user.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
bash tests/installer.sh
bash scripts/test-install-compat.sh
# Explicit host-dependent checks (never counted as passing when skipped):
cargo test --test native_configuration -- --ignored --nocapture
cargo test --test persistent_session -- --ignored --nocapture
```

The latter test discovers installed prerequisites and a supported GUI terminal
before starting. Missing tools fail with the missing prerequisite. It creates its
own desktop, maps a GUI terminal, restarts the HTTP process twice, and checks the
same desktop signature/window PID and native bindings afterward. On the
2026-10-22 Omarchy host it selected Ghostty, retained its PID through both
reconnections, loaded 225 native bindings and reported no config errors. These
checks do not prove all GUI applications, networking paths or media latency.
