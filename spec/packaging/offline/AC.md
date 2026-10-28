# Offline staging acceptance

- The real Linux CLI accepts `--package-managed --stage-only` against an isolated
  package tree, with an unusable PATH, and changes no files.
- Both package units must exactly equal the executable's exported units. Missing,
  modified or symlinked units and a missing/non-executable/symlinked payload fail.
- Existing fixture configuration remains unchanged and no account, credential,
  recovery, service enablement or runtime path is created.
- AUR install/upgrade hooks forward the explicit staging flag; snapshot/removal
  hooks make no call. Failure of the staging CLI propagates out of the hook.
- The codec install/upgrade/removal hooks make zero systemctl calls offline.
- Invalid staging-environment values are rejected before any executable call.
- Normal package lifecycle, legacy detection, recovery and owner-preservation
  tests continue to pass in GitHub Actions before a new SemVer tag is published.
