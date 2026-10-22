# Installing the native candidate

The executable embeds this transactional installer. On a provisioned LightOS
machine, `sudo ./lightos-remote-desktop install-service` replaces the existing
Python service command and activates Rust after dependency and health checks.
Use `install-service --dry-run` for read-only checks, `--stage-only` to avoid
restarting, and `sudo ./lightos-remote-desktop rollback-service` for the previous
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
installed Omarchy Shell also requires `quickshell`, `gtk-launch` and `foot`.
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
The unreleased native-media binary links `libpulse.so.0` even with audio disabled;
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
bash scripts/install.sh --dry-run --binary /absolute/path/to/lightos-remote-desktop
bash -n scripts/install.sh
bash -n scripts/activation.sh
bash -n tests/installer.sh
bash tests/installer.sh
```

Default input is `target/release/lightos-remote-desktop`; `--binary` overrides it.
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
sudo bash scripts/install.sh --binary /absolute/path/to/lightos-remote-desktop
# After separate-port real desktop acceptance and review:
sudo bash scripts/install.sh --binary /absolute/path/to/lightos-remote-desktop --activate
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
