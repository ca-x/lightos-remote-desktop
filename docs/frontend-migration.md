# React / TypeScript frontend migration

Status: implemented for v0.1.3; locally validated as 0.1.3-dev on 2026-10-22.
Use the lighter Headless UI option. GitHub Actions builds and tests the frontend
before compiling Rust and includes frontend license notices in release archives.
User constraint: use an established UI component library, not a custom UI kit.
Additional explicit constraints: use a mature React UI library and icon package;
use ui-ux-pro-max, emil-design-eng and animate for interaction/motion design;
embed all production frontend resources into the Rust executable.

## Goal

Replace the current DOM-driven UI with React and TypeScript, add English and
Simplified Chinese, and provide light, dark and system-following themes without
regressing remote input, frame presentation or the single-binary deployment.

## Capability boundaries

| Module | Responsibility | Depends on |
| --- | --- | --- |
| remote-client | Typed protocol, Canvas/WebCodecs, audio, input, bounded ACKs and cleanup | Existing Rust protocol |
| react-shell | Floating toolbar, monitor selection, settings and client lifecycle | remote-client |
| locale | Typed Chinese/English resources, browser detection and local preference | react-shell |
| theme | Semantic color tokens, light/dark/system and local preference | react-shell |

Build order: remote-client / react-shell together, then locale and theme.

## Recommended approach

- Use TypeScript, React, Vite and Headless UI, with lucide-react icons.
  The user chose the lighter option over MUI and authorized implementation.
  Use library Button, Field, Label, Popover, Select, Input and Textarea primitives.
  Do not combine UI libraries or add a general-purpose CSS/component framework.
  Keep custom code limited to remote-desktop behavior, toolbar placement and
  application layout. Keep the Rust media backend and wire protocol.
  Do not add a Node production server, external CDN or global state framework.
- Keep frames, pointer movement and audio outside React state. Publish small UI
  snapshots to React, with performance text updated at its existing interval.
  React owns layout, Headless UI interaction state and disabled-state snapshots.
  The controller owns empty status/option leaf contents and uncontrolled input
  values through the typed Elements boundary, preserving the tested transport
  logic. Do not put React children inside controller-owned text/option leaves.
- Retain draggable toolbar placement, tooltips, all three viewport modes,
  browser-following resolution, desktop/output switching, clipboard and generic
  keyboard capture. Theme changes must never recolor the captured desktop.
- Use react-i18next for application strings and labels on headless primitives.
  Use semantic CSS tokens for light/dark, compact sizing and reduced motion.
- Translate static labels, tooltips, accessibility text and client-owned status
  messages through typed resources. Treat server/device descriptions as data,
  rendered as text rather than HTML. Initial languages: zh-CN and en.
- Offer browser-following language plus explicit Chinese/English selection.
  Offer system/light/dark theme. Persist explicit choices locally; inaccessible
  or malformed storage must not prevent connecting. Follow system changes only
  in system mode, with correct initialization before the first painted toolbar.
- Bundle assets into the Rust binary under the configured URL prefix. Relative
  asset paths must work with authenticated subfolders. Node is build-time only;
  CI builds/tests the frontend before compiling Rust and packages frontend notices.
  Include JavaScript, CSS, icon resources and locale dictionaries in that bundle;
  do not load a CDN, remote fonts or an external web server. Embed the generated
  asset directory, including hashed chunks, rather than only the HTML entry.
- Dispose the exact client instance created by the React effect. Mount/unmount
  must close sockets, native browser decoders, audio and event listeners without
  creating duplicate controllers during development effect replay.
- Mount library popovers and notifications inside the fullscreen application
  container, not outside it. Keyboard handling must recognize controls rendered
  through portals: typing in a Select, form or dialog must never be forwarded to
  the remote desktop. Remote capture resumes only with desktop focus.

## Interaction and motion decisions

Use the existing compact floating-toolbar direction, not a dashboard/landing
page. The UI/UX search's landing-page conversion suggestions are not applicable
to an immersive remote desktop and must not be copied into the interface.

- Reuse library focus management, Select and Popover; native title tooltips and
  status regions do not require a second component library.
  Use Lucide React icon components, with localized accessible names on controls.
- Remote input, keyboard shortcuts, frame updates and performance values change
  immediately without motion. A drag follows the pointer directly, without lag
  or decorative bounce. Provide keyboard placement as an alternative to dragging.
- Occasional pointer-opened panels use short, trigger-origin-aware library motion
  for spatial consistency; use 160 ms entry and 125 ms exit with the existing
  strong ease-out curve cubic-bezier(0.23, 1, 0.32, 1). Do not add a motion library
  for a fade. Do not apply a second animation on top of library motion.
- Keyboard-initiated panel changes are immediate. Reduced-motion preference
  removes spatial motion; any useful opacity feedback remains short and gentle.
- Restrict motion to transform and opacity, never blanket transition: all.
  Gate hover-only effects to fine pointers with hover support. Test rapid
  open/close, fullscreen portals, focus restoration and desktop input isolation.

## Acceptance

- Typecheck, production frontend build, component/preference tests and existing
  protocol/lifecycle regression tests pass, followed by Rust and installer checks.
- Browser checks cover Chinese/English, every theme, system theme changes,
  reload persistence, keyboard labels and unavailable storage.
- Real connection checks cover H.264/JPEG, fullscreen/fit/actual-size, resolution,
  dragging, desktop/monitor switching, audio, clipboard and teardown/reconnect.
- No per-frame React render; compare the same local workload before/after.
  Do not claim that React migration itself lowers transport latency.
- Deploy locally and verify the installed version and real page before a separate
  release. Preserve transactional rollback to v0.1.2.

## Non-goals

No codec rewrite, new remote protocol, authentication change, desktop theme
modification, automatic translation of remote application content, or expanded
language catalogue in this iteration. Avoid a full visual redesign: preserve
the compact floating-toolbar interaction already agreed with the user.
