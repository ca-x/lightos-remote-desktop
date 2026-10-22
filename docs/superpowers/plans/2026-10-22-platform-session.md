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
- [x] 3. Separate desktop owner from transport. Add desktop-session CLI, atomic metadata, validated attachment, owned cleanup regression and reattachment tests.
- [x] 4. Integrate session systemd template and transactional installer snapshots/rollback. Validate no session restart during upgrade/recovery, and preserve fixture-only staging.
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

- Tasks 1–4: workspace tests and Clippy -D warnings passed; 40 installer and 13 compatibility fixtures passed. Native config verification passed; Ghostty survived two complete HTTP transport restarts with unchanged PID/signature, 225 native keybindings, no configuration errors.
- First implementation committed/pushed as 47bcaa5 to feat/native-desktop-handoff. User authorized merge to main after local verification, local service update, and timely commits/pushes.
- Independent read-only reviewer dispatched before merge/deployment; documentation and final deployment verification remain.
- Final review: fixed installed physical-attachment regression and missing D-Bus state propagation. Both regression tests observed RED on the reviewed implementation, then GREEN after fixes; whole workspace and Clippy passed. Also fixed effective User/drop-in/numeric-UID staging compatibility with a dedicated fixture.
- Native Lua reload now exercised in the isolated live test; Ghostty remained mapped with the same PID after both transport generations and config reload. No config errors; 225 native bindings.
- Deployment authentication: noninteractive sudo requires a password; pkexec has no registered authentication agent. Prepare a desktop-terminal sudo/systemd-run update so the installer runs outside the service it replaces.

- Ruling: the user explicitly authorized merging based on local tests. GitHub runners remain queued; merge uses passing local workspace/bundled tests, frontend tests, Clippy, installer fixtures and live Ghostty/config-reload verification, while reporting remote CI as pending.
