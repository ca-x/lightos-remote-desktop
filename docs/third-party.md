# Third-party components

Cargo.lock pins Rust dependency resolution. Crate licenses remain their own;
inventory the exact resolved graph before distributing a release.

The optional private Aquamarine build uses official source from
`https://github.com/hyprwm/aquamarine` at commit
`f31c47a1b9d300847d8fc3108ad959448103dfc1`, version 0.15.1. Source LICENSE declares
BSD-3-Clause. Preserve its license and copyright notice with any distributed
binary or source derivative. The build source retains the original LICENSE;
`install-compat.sh` carries the required files into the private deployment.

Cage, wlroots, Hyprland, FFmpeg, PulseAudio/PipeWire and graphics drivers are
external programs/libraries, not part of the application source. Package choices
and FFmpeg build flags affect the resulting distribution's dependency and license
inventory. This file is a provenance note, not a compliance certification.

The `bundled-jpeg` build includes libjpeg-turbo via turbojpeg-sys 1.2.0 and the
turbojpeg 1.5.1 wrapper. CI archives include their LICENSE notices. Icons are
selected paths from lucide-static 1.61.0, including Feather-derived symbols;
their ISC/MIT notices are in `licenses/lucide.txt` and included with release docs.
