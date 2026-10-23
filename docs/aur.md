# Arch Linux / AUR

Install and update the x86_64 binary package:

```sh
yay -S lightos-remoted-bin
yay -Syu
```

For optional hardware H.264 encoding, install the separate binary package:

```sh
yay -S lightos-codec-ffmpeg-bin
```

`lightos-remoted-bin` lists this as an optional dependency and works without it.
The helper supplies FFmpeg-backed GPU encoders; it does not replace the service
or desktop owner. It is built from the same upstream release tag against Arch's
FFmpeg headers and libraries in Arch Linux CI. AUR downloads that dedicated
Arch archive; the Ubuntu worker archive is never used on Arch. There is no local
Rust or C++ compilation. Packaging verifies its checksum, source/ABI manifest,
ELF dependencies and notices. Fixed FFmpeg SONAME dependencies prevent installing
a worker against incompatible libraries. A new Arch worker package is required
when those ABIs change.

The service discovers `/usr/lib/lightos-remoted/lightos-codec-ffmpeg` beside its
own executable. Installing, upgrading or removing the helper refreshes only an
already-active transport with the canonical managed executable and unit. The
hook skips stopped services, legacy/custom base units and recovery instances;
it never starts or restarts the desktop owner. Existing GUI applications remain
running, though browser/RDP connections reconnect. This refresh clears cached
encoder probes. Drivers and service-user GPU access still determine whether
hardware encoding works. Missing or unusable hardware falls back to built-in
software H.264, then JPEG as needed.

On a fresh system, installation creates the nonroot `lightos-remoted` account,
generates a private random password, checks the candidate, and enables/starts
`lightos-remote-desktop.service`. No manual service enable step is required. The
installer also enables/starts `lightos-remote-desktop-session.service` for a fresh
virtual desktop. That service owns the desktop and its applications independently.
Ghostty is installed for the generic desktop.
Open `http://HOST:8766`; the initial username is `admin`. Read the password locally
as root:

```sh
sudo cat /etc/lightos/remote-desktop.env
```

Keep this file private. Existing configuration is never replaced or evaluated as
a shell script. Edit it with `sudoedit /etc/lightos/remote-desktop.env`; an optional
`/etc/lightos/remote-desktop-native.env` is loaded afterwards. The existing
`LIGHTOS_REMOTE_DESKTOP_*` configuration keys remain supported.

| Purpose | Path |
| --- | --- |
| Command | `/usr/bin/lightos-remoted` |
| Packaged executable | `/usr/lib/lightos-remoted/lightos-remoted` |
| Automatic setup/retry | `lightos-remoted install-service --package-managed` |
| Private Aquamarine library | `/usr/lib/lightos-remoted/aquamarine/libaquamarine.so.14` |
| Configuration | `/etc/lightos/remote-desktop.env` |
| Desktop account home/data | `/var/lib/lightos-remoted` |
| Root-owned package recovery data | `/var/lib/lightos-remoted-package` |
| Transport / desktop runtime | `/run/lightos-remoted` / `/run/lightos-remoted-session` |

The release executable owns both service definitions and all installation
behavior. During packaging, `makepkg` exports the two units with
`lightos-remoted install-service --print-unit transport` and `--print-unit session`
as the build user, then installs those files under pacman ownership. The package
contains no separate bootstrap or activation script. Its install/upgrade/removal
hooks call the installed executable with `install-service --package-managed`
after installation or upgrade, `install-service --snapshot` before an upgrade,
and `install-service --remove` before removal. Standalone installation uses the same installer without `--package-managed`; it creates
both services and refuses to overwrite package-owned files.

Normal upgrades check the candidate and restart only the transport. The desktop
owner has no `PartOf`/`BindsTo` relationship to the transport, so existing desktop
applications keep running. Package hooks select `--package-managed` so the
installer activates the pacman-owned payload and units. The existing service names
and canonical auth file remain unchanged because LightOS platform setup scripts
depend on them; no service aliases are installed.

Before replacing a previous new-layout package, the upgrade hook snapshots its
binary and any locally installed sibling codec helper. Persistent digest markers
allow only a successfully activated payload to replace this recovery copy;
a candidate rejected by the doctor or later activation gates cannot displace it.
If activation fails, the hook reports failure and attempts to restart the previous
transport through a runtime systemd override. It does not overwrite package-owned files or restart
the desktop owner. This recovery override lasts until reboot; correct the failed
package before rebooting. After correcting dependencies or installing a working
package, retry the same checks and activation with:

```sh
sudo lightos-remoted install-service --package-managed
systemctl status lightos-remote-desktop.service lightos-remote-desktop-session.service
journalctl -u lightos-remote-desktop.service -u lightos-remote-desktop-session.service
```

Removing the package disables future transport and desktop-owner startup while
retaining running services, credentials/account/data, and older installations.
Finish and save remote work before deliberately stopping/removing retained sessions.

## Existing installations and manual migration

The setup hook detects an old transport or owner unit in `/etc/systemd/system`,
or an active transport whose `/proc/PID/exe` is outside the new packaged/recovery
binary paths. A running instance of this package is recognized by its executable
path and remains eligible for normal upgrades. When a legacy instance is
detected, setup leaves it untouched and skips new-service activation, preventing
a second server on port 8766. Installing this package does not migrate or stop its persistent desktop.

Migration requires an intentional session cutover. Back up the old units,
drop-ins, `/etc/lightos/remote-desktop.env`,
`/etc/lightos/remote-desktop-native.env`, and any settings/data you need. Save
work in the old remote desktop. Stop/disable the legacy transport and then stop
its owner `lightos-remote-desktop-session.service`; this ends that desktop's GUI
applications. Remove the legacy units/drop-ins after saving them elsewhere.
In particular, the old installer's `90-native.conf` and `95-session.conf` point at
the old payload/runtime and must not override the complete AUR units. Do not
rewrite these live while the old desktop is running. Continue by
preserving `/etc/lightos` and `/opt/lightos/remote-desktop` (the original Python
payload/platform setup integration). Reload systemd, and run `sudo lightos-remoted install-service --package-managed`. Confirm the new
service and browser login work before deleting any old configuration, state or
binaries. The README has the explicit cleanup commands. Package hooks never run
those cleanup commands.

The old `/usr/local/lib/lightos-remote-desktop/current` deployment, releases,
compatibility library and `/var/lib/lightos-remote-desktop` recovery state are
retained until you remove them yourself. The canonical
`/etc/lightos/remote-desktop.env` and optional native environment are preserved
byte for byte. If `LIGHTOS_REMOTE_DESKTOP_USER` names an existing nonroot account,
the binary installer honors its UID, primary group and home in both service units.
After reloading systemd it also resolves both effective unit users through NSS
and requires the configured nonroot UID, including when later drop-ins override
`User=`. Root aliases and a mismatched desktop-owner UID are rejected before
running the candidate.
Without a configured user, fresh installs use the dedicated account. Existing
shell preferences and private library settings are retained; review old absolute
paths before deliberately deleting their targets. Do not delete the platform's
Python payload or setup scripts as part of removing the old Rust deployment.

## Compatibility and optional encoding

The main application is precompiled; `makepkg` builds a small private Aquamarine
0.15.1 compatibility library as the build user. The source is pinned to commit
`f31c47a1b9d300847d8fc3108ad959448103dfc1` and the reviewed protocol-version patch.
No build happens in the root install hook. The package requires system Aquamarine
0.15.1 and verifies Hyprland's SONAME-14 dependency at build time. Its private
library has generated dependencies on the actual hyprutils/display-info SONAMEs.
If Arch changes these ABIs, update/rebuild and revalidate the package before
upgrading; a different compositor ABI is not silently accepted.

Before deleting an old compatibility-library directory, use `sudoedit
/etc/lightos/remote-desktop-native.env` and set
`LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/usr/lib/lightos-remoted/aquamarine`.
Remove or update any other assignment of this key in that file, then run the
bootstrap and verify the desktop after a planned owner restart. Changing the
owner library requires ending that desktop session, so save application work
first. Keep the old library until that verification succeeds.

Fresh installations include the Cage/Hyprland desktop, Mesa, private D-Bus and
PipeWire/Pulse/WirePlumber audio programs, Ghostty, and a usable font. They use the
generic minimal desktop under the dedicated account. When an existing
configured account is retained, its own home and shell preferences are used;
configuration is not copied between accounts.

The core works without the optional `lightos-codec-ffmpeg-bin` package. Its helper is
owned by that package; the main package owns the service binary and units.
A separately validated Arch-compatible helper may also be installed locally at
`/usr/lib/lightos-remoted/lightos-codec-ffmpeg`, but move that unmanaged file aside
before installing the helper package to avoid a pacman file conflict. Old legacy
helpers stay in their old deployment rather than crossing incompatible layouts.

## Release automation

`.github/workflows/aur.yml` accepts only an explicit workflow dispatch with a
`vX.Y.Z` tag. It rejects draft/prerelease/malformed/downgrade inputs,
requires successful tag CI, checks the exact six release assets and core checksum,
and verifies both ELF payloads, the worker ABI manifest and licenses. Tag CI
explicitly dispatches the Arch worker workflow after publishing release assets,
since events created by GitHub's own token do not trigger another workflow.
The Arch worker job then dispatches AUR once the dedicated worker assets are
available for a stable release. This CI → Arch CI → AUR order ensures that the
Arch assets exist before AUR validation starts.

Draft releases wait until publication. After manually publishing a draft,
dispatch the Arch worker workflow; if all six assets are already present,
dispatch AUR directly. Use the same AUR dispatch to retry publication:

```sh
gh workflow run aur.yml --ref main -f version=v0.1.11
```

One shared guard checks both AUR versions before either deployment. The helper
publishes first; the main package waits for success or an already-current helper.
Retries skip each package independently when its version and pkgrel are current.
Same-version dispatches retain each tracked pkgrel; a newer upstream version
starts at pkgrel 1. `force=true` republishes identical metadata and never permits
a version or pkgrel downgrade.

Both deployments use the organization's existing `AUR_USERNAME`, `AUR_EMAIL` and
`AUR_SSH_PRIVATE_KEY` secrets through the pinned action. GitHub keeps `SKIP`
source checksums and no `.SRCINFO`; deployment generates real checksums and
`.SRCINFO`. The exact package allowlists are:

| Package | Published files |
| --- | --- |
| `lightos-remoted-bin` | `PKGBUILD`, `.SRCINFO`, `lightos-remoted-bin.install`, `aquamarine-wayland.patch` |
| `lightos-codec-ffmpeg-bin` | `PKGBUILD`, `.SRCINFO`, `lightos-codec-ffmpeg-bin.install`, `validate-arch-codec.py` |

Arch CI's `scripts/codec-worker-notices.py` collector retains the locked runtime
dependency notices, including
FFmpeg's Rust bindings, static OpenH264 and TurboJPEG native code and wrappers.
System FFmpeg libraries and GPU drivers remain system dependencies. Unit
definitions and installer scripts come from the release executable rather than
independent AUR assets. The package installs the bundled Chinese and English
READMEs alongside the bundled docs. No generated metadata or downloads are
committed back to GitHub.

Validation must distinguish mocked service activation from a real fresh-machine
installation. The installer fixtures exercise initialization, preservation,
transport-only upgrades, activation recovery, legacy detection and removal
without changing host services. A successful fixture run alone does not establish
that every supported GPU or fresh Arch machine can run the nested desktop.
