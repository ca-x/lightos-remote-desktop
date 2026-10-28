# Niri native-capture feasibility gate

## Contract

Before selecting a persistent Niri owner, exercise the existing native capture
backend against Niri 26.04 in GitHub. This is a feasibility test of an existing
interface, not an implementation of Niri session ownership or a Pond image.
No production code or the immutable v0.1.22 release source changes in this phase.

- Use a private non-root fixture user, bus, runtime and software-rendered nested
  Cage/Niri pair. Never attach a running host's desktop, GPU, VT or credentials.
- Select the actual Niri-owned Wayland socket by its peer process and UID;
  capturing Cage's parent surface is not sufficient.
- Probe the required protocols, capture a nonuniform client frame on output
  `winit`, and retain a screenshot plus a bounded-scope machine-readable report.
- After capture feasibility, inject actual pointer motion/button and key
  press/release commands. Require the independent wev client to observe every
  transition after injection, not only protocol globals or successful sends.
- Clipboard, Pond/Quickshell, persistent-owner recovery and platform
  proxy/physical VT acceptance remain explicitly untested.

## Execution

1. Commit the ignored Rust integration probe and explicit workflow. Ordinary
   CI compiles it but never attaches a desktop. All execution stays in Actions.
2. Independent read-only review of isolation/cleanup and scope before dispatch.
3. Run `niri-probe.yml`, observe the actual result, and address root causes with
   fresh remote evidence. A successful feasibility probe need not manufacture
   a failing test; a subsequent behavior fix does require a meaningful RED.
4. Only after capture feasibility, exercise input and specify shell/owner acceptance. Keep
   the overall desktop CLI gate and image packaging closed meanwhile.

## Ledger

- 2026-10-28: Offline-staging candidate `b244af2` passed full CI `40682739378`
  and independent review. Published SSH tag `v0.1.22`; its separate exact-tag
  CI -> Release -> Arch codec -> AUR pipeline is running, not yet claimed done.
- Payload audit `40683134808` confirms Pond shell starts `qs -c pond-shell`.
  Upstream Niri v26.04 source commit is
  `8ed0da44d974c32c6877d2f4630c314da0717ecb`; nested startup must omit `--session`.
- Main run `40686718003` stopped at three rustfmt differences in the new probe.
  Apply only the runner's diff and rebuild remotely. No Niri runtime result is
  inferred from this format failure; tag v0.1.22 uses separate immutable source.
- Run `40687440648` compiled the probe and passed the initial workspace tests,
  then feature clippy reported `chunks_exact_to_as_chunks` on Rust 1.99. Replace
  the fixed-size pixel iterator with typed `as_chunks::<4>()`, not a lint allow.
  Independent review found no execution/isolation/cleanup P1/P2; dispatch the
  real probe after this correction. No runtime result is claimed beforehand.
- Probe `40689971420` at `72f5239` succeeded: actual Niri-owned socket,
  1272x688 nonuniform fresh frame, capture/keyboard/pointer globals present.
  This proves software-rendered capture without a physical VT, not input
  delivery or the Pond shell. Extend the same fixture with real wev-observed
  motion/button/key transitions next; production code remains unchanged.
- Input probe `40691441191` at `d07e36b` failed with an empty observed log
  suffix. Independent review found that Niri's spawn path discards client
  stdout/stderr, so `stdbuf` alone cannot connect wev to the observer. This is
  an observation-chain failure, not evidence of undelivered input. Review also
  found numeric-prefix false positives in the receipt predicate. Add regression
  cases before changing it; upstream wev prints XKB `evdev + 8`, hence key 38
  is the receipt for injected evdev 30. All execution remains in GitHub.
- Regression commit `67bb65b` run `40693788740` stopped at one rustfmt
  expression layout before the new tests ran. Apply only the emitted layout;
  this format failure is not the required behavioral RED.
