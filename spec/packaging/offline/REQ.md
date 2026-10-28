---
id: REQ-PACKAGING-OFFLINE-001
status: accepted
---

# Offline package staging

Pond/LightOS image builds must be able to install the AUR payload without
creating a shared account, authentication secret, desktop owner, runtime state,
or an enabled/running service. All downloads, builds and executable checks run
in GitHub Actions; this development host is not an installation target.

The executable owns the contract: `install-service --package-managed --stage-only`
is a read-only check of the installed executable and both exact embedded units.
It does not run the embedded installer, systemctl, NSS helpers, or the doctor.
This flag combination is Linux-specific and leaves standalone staging unchanged.

The AUR hooks select that mode only with the explicit environment variable
`LIGHTOS_REMOTED_PACKAGE_STAGE_ONLY=1`. Offline pre-upgrade/pre-remove hooks do
not snapshot, recover, disable or restart a live deployment. The optional codec
hook also performs no systemctl operation in this mode. Unset/0 retains normal
installation behavior; other values fail closed.

Activation on a booted instance remains an explicit later call to
`lightos-remoted install-service --package-managed`, after LightOS has configured
the instance and its intended desktop attachment. It creates or preserves private
per-instance authentication through the existing installer. Offline validation
does not prove Niri capture/input, physical VT handoff or platform connectivity.

No automatic chroot/container detection, dependency bypass, shared default
password, new service alias, or changes to host services are in scope.
