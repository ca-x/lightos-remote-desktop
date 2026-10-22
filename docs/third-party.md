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
