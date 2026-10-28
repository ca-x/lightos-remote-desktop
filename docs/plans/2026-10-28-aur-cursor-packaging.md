# AUR cursor companion repair

## Cause

- The installed `0.1.22-1` transport requires the independent-cursor capability
  before capture readiness. Its fallback loads a sibling `lightos-cursor.so`.
- Neither the generic release archive nor the AUR recipe included that file.
  The bare missing-file error was reported as `Wayland capture: No such file
  or directory`; executable-dependency-only `doctor` could still report success.
- The actual recovered older transport used `j/lightoscursor`, whereas the new
  binary uses `j/lightosindependentcursor`. An existing owner did not supply the
  new capability merely because it remained alive across the upgrade.

## Repair and evidence

- Require the companion in the real `package()` regression: observed missing-file
  RED, then 49 checks GREEN. Related bootstrap/codec/release guards pass 107/47/54
  checks. Historical release fixtures pin their own pkgrel instead of inheriting it.
- Bump only the main AUR package to `0.1.22-2`; compile the companion from its
  matching release source using installed Hyprland headers, and package it as
  a regular `0644` file. Add an Arch-native compilation check to GitHub CI.
- The user explicitly permitted local compilation and installation for this
  incident. Verify the cached release checksum, retain the existing Aquamarine
  payload, and use the real package function to include the ABI-matched companion.
- On 2026-10-28, installation passed both native readiness probes. The transport
  is active with zero restarts, and a subsequent client received `desktop ready`.
  The desktop-owner PID stayed 12817 with the same start timestamp and zero
  restarts. Pacman owns the new companion; authentication was not replaced.
- Independent read-only review found no blocking P1/P2. Do not restart the owner
  as a workaround, move the existing v0.1.22 tag, or continue Niri-owner research
  instead of the oci-kit/Pond task. New GitHub checks remain separately observable.
