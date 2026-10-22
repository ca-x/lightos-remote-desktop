# Platform configuration and persistent desktop implementation plan

Goal: implement the approved issue #2 split and preserve UI applications across transport service updates.

Architecture: platform-detect reads OS identity and installed desktop capabilities. desktop-config writes only a private runtime config, loading the actual native Omarchy entry and adding only remote-session overrides. A separately supervised desktop-session owns Hyprland, Shell, D-Bus and audio; serve attaches using a versioned descriptor and never tears down that owner.

## Requirements and boundaries
- Preserve minimal/custom/attached desktop modes; never overwrite host Hyprland or Omarchy defaults.
- Identify distribution separately from desktop support. Arch-family does not imply Omarchy.
- No runtime GitHub downloads; explicit diagnostics for incompatible installed Lua modules.
- Transport restart and rollback must not restart an existing session service. First migration from the old combined process necessarily rebuilds its session once.
- Keep systemd control-group cleanup. Do not leave orphaned desktop processes as a handoff mechanism.
- Descriptor must be private, owned, versioned, bounded, atomic, and validate sockets and owner liveness. A dead owner must never be adopted.
- Session restarts/reboots/explicit desktop reset can still terminate GUI apps; only transport updates preserve them.
- User authorized local deployment and timely commit/push. No tag or GitHub Release is requested.

## Tasks
- [x] 1. Add platform-detect crate; fixture coverage for Arch, EndeavourOS, Manjaro, Omarchy, missing installs and actual Lua verification. Integrate doctor diagnostics.
- [x] 2. Add desktop-config crate; keep legacy fallback, add selective Lua entry with session state/PATH isolation; verify generated config against installed Hyprland without changing host files.
- [ ] 3. Separate desktop owner from transport. Add desktop-session CLI, atomic metadata, validated attachment, owned cleanup regression and reattachment tests.
- [ ] 4. Integrate session systemd template and transactional installer snapshots/rollback. Validate no session restart during upgrade/recovery, and preserve fixture-only staging.
- [ ] 5. Document migration and operational boundary; run workspace tests, fmt/clippy, installer fixtures and isolated real desktop reconnect test; obtain fresh review.

## Review focus
- Stale descriptors/PID reuse and socket ownership; malformed descriptors and symlinks.
- Partial startup/termination must clean owned processes; attached transport must not remove session state.
- Root and transport environment overrides must not leak into session owner or physical attachment.
- Rollback must restore all new managed files; unchanged upgrades retain rollback snapshot.
- Newer upstream module dependencies must fail visibly and never fall through to a different profile silently.

## Execution ledger
- Baseline: 56 library tests passed, 2 explicit environment-dependent tests ignored.
- Worktree: managed /home/czyt/.codex/worktrees/9305/lightos-remote-desktop, based on HEAD; source checkout clean.
- Ruling: execute the previously approved design directly and cover handoff as a lifecycle bug fix; no additional spec approval loop.

- User refinement: Omarchy 4.x loads the actual installed/user Lua entry first, conf only when no Lua entry exists; no copied Omarchy bindings fallback. A DesktopInitializer trait selects GenericInitializer or OmarchyInitializer.
- Native Lua validation passed against this machine. Real startup exposed a legacy IPC dispatcher incompatibility: Lua requires hl.dsp.exec_cmd; corrected and rerun.
- User refinement: probes must check executable existence and distinguish missing prerequisites from unsupported capabilities; no silent fallback or skipped-success live tests.
