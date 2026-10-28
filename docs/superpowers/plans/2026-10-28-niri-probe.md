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
  proxy/physical VT acceptance remain explicitly untested until each receives
  its own execution evidence. The next increment checks production clipboard
  write with independent wl-paste, and independent foreground wl-copy with
  production clipboard read. Use distinct Unicode/multiline selections.

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
- Actual behavioral RED: `0114c31` run `40694410495`, test job `122837240964`
  passed formatting, then failed the realistic XKB-key receipt and rejected-key
  prefix regression (the old predicate falsely accepted key 300). Replace
  substring matching with exact event/interface and integer-field parsing.
  Explicitly redirect wev to its own private log, wait for keyboard focus, then
  take an offset in that same log before injecting. The workflow also executes
  every receipt regression before its ignored real-desktop probe. GREEN and
  independent re-review are pending; no input-delivery success is claimed.
- Retain the observed old key-300 failure and independently mutate the valid
  key-38 fixture to key 380 or button 2720, so a wrong keyboard field cannot mask
  a pointer-prefix regression in future implementations.
- Re-review of `0114c31..9d1b57a` closed both old P2 findings with no new
  blocking P1/P2. Probe `40695810183` at `9d1b57a` passed all four receipt
  regressions, then failed the real gate: its actual log contains XKB-38
  press/release and left-button press/release, but the first absolute pointer
  movement produced `enter`, not `motion`. Preserve the strict motion check:
  establish pointer plus keyboard focus first, take the offset, then inject a
  distinct second movement inside the client. This is another fixture fix,
  not a production backend failure or an input-delivery pass.
- GREEN: `ab28c28` probe `40697001655`/job `122846361301` succeeded, including
  all four predicate regressions and actual motion, left press/release and
  XKB-38 press/release after the focus boundary. Report remains 1272x688 and
  explicitly keeps Pond shell, platform proxy and physical VT false.
- Extend the same isolated socket with two-way text clipboard. Independent
  helpers clear inherited environment, use foreground ownership and kill on
  drop; reads and final owner shutdown are bounded. Image/notification
  clipboard acceptance stays false. This new feasibility check changes no
  production behavior and does not manufacture a failing test.
- Clipboard review found one P2: the test directly awaited its initial
  production write and could hang until job timeout. Extract that wait without
  changing its behavior, then pass an honestly stalled dependency to the same
  test policy boundary. An outer five-second test supervisor must observe this
  regression fail before the probe gains its own three-second write deadline.
  No local executable validation or fake production failure is introduced.
- Text clipboard GREEN: `878a791` probe `40699256007`/job `122854289937`
  verified independent client-to-backend and backend-to-client Unicode text.
  Full CI `40699006757` also succeeded. This is not full Pond acceptance.
- The stalled-write regression at `e8a4a5d`, CI `40700198019`/test job
  `122857611085`, failed after its five-second supervisor expired. Add the
  missing three-second deadline to the shared probe wait; remote GREEN is
  pending. Defer native-owner implementation at the user's request and return
  to oci-kit/Pond after addressing CI and the reported package activation issue.
