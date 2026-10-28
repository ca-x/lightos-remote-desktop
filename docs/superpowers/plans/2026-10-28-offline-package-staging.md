# Offline package staging plan and ledger

## Contract

Implement `spec/packaging/offline/REQ.md` and its acceptance criteria before
building Pond's desktop OCI image. Keep external LightOS service names, native
authentication ownership and ordinary AUR activation behavior unchanged.

## Steps

1. Add isolated CLI and real-hook regression tests. Publish source via SSH and
   observe the intended missing-feature failure in GitHub Actions.
2. Implement read-only package validation directly in Rust, and explicit AUR
   hook selection. Re-run all checks remotely; no host installation or tests.
3. Independent read-only review, fix important findings with remote regression
   evidence, then bump the next unoccupied SemVer and publish through the
   existing tag -> Release -> Arch codec -> AUR pipeline.
4. Record publication/ABI evidence and resume the Rust `lighos-init` desktop gate
   and Pond image work. A staged package is not a working Niri remote desktop.

## Ledger

- 2026-10-28: User authorizes upstream program/AUR changes, SSH commits/pushes
  and a new release if needed for OCI staging. Queried remote before editing:
  main `b086a3fd2c02c92a7de20bd38e2002aef16d80e9`, latest release `v0.1.21`.
  Fast-forwarded the clean local source checkout through SSH; no runtime assets
  or rootfs were downloaded to this machine.
- New test-first phase intentionally exposes the existing mutually exclusive
  flags and live-only package hooks. Only Actions may execute the tests.
- Run `40672275877`, package-contracts job `122761789913`, observed the intended
  hook failure: the recorded live lifecycle differed from the required two
  staging-only calls. The main test job stopped first at two rustfmt changes in
  the new test file; apply that runner-produced formatting without claiming a
  Rust behavioral RED that did not run.
- Implemented package validation directly in Rust before temporary-script
  extraction, avoiding any installer/systemctl/NSS invocation. AUR hooks select
  it only with the explicit staging environment. Full remote GREEN and review
  are still required before bumping or tagging a release.
