# Third-party components

Cargo.lock pins Rust dependency resolution. Crate licenses remain their own;
inventory the exact resolved graph before distributing a release.

The optional private Aquamarine build uses official source from
`https://github.com/hyprwm/aquamarine` at commit
`f31c47a1b9d300847d8fc3108ad959448103dfc1`, version 0.15.1. Source LICENSE declares
BSD-3-Clause. Preserve its license and copyright notice with any distributed
binary or source derivative. The build source retains the original LICENSE;
`install-compat.sh` carries the required files into the private deployment.

Cage, wlroots, Hyprland, PulseAudio/PipeWire and graphics drivers are
external programs/libraries, not part of the application source. Package choices
affect the resulting distribution's dependency and license
inventory. This file is a provenance note, not a compliance certification.

The `bundled-jpeg` build includes libjpeg-turbo via turbojpeg-sys 1.2.0 and the
turbojpeg 1.5.1 wrapper. CI archives include their LICENSE notices. Icons are
selected paths from lucide-static 1.61.0, including Feather-derived symbols;
their ISC/MIT notices are in `licenses/lucide.txt` and included with release docs.

Native H.264 uses openh264 0.9.9 (BSD-2-Clause) and source-built Cisco OpenH264
through openh264-sys2 0.9.9 (BSD-2-Clause). Release packaging must include both
wrapper and upstream notices (the wrapper crate supplies its attribution and
license declaration in README.md, not a standalone LICENSE file; standard terms
and attribution are also supplied in `licenses/openh264-rust.txt`). Source-built OpenH264 is not Cisco's distributed
binary and must not be represented as covered by that binary's patent licensing
arrangement. Releases distribute this source-built implementation with upstream
and wrapper notices, without claiming Cisco binary patent coverage or providing
a patent/legal clearance guarantee. Distributors must assess their own obligations.
Native audio uses libpulse-binding (MIT/Apache-2.0) and dynamically linked
libpulse; system libpulse and its transitive dependencies retain their licenses.
FFmpeg is only an optional development/test decoder, not a runtime dependency.
