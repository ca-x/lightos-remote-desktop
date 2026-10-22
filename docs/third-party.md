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

Native Opus uses the `opus` 0.4.0 wrapper (MIT/Apache-2.0) and `opusic-sys` 0.7.5
(BSD-3-Clause), with its bundled libopus source statically built. CI includes the
wrapper's MIT notice, binding LICENSE and upstream Opus COPYING in each archive.
The application codec does not add a dynamic libopus dependency; existing system
Pulse/libsndfile libraries may themselves depend on a system libopus.

The embedded frontend uses React/React DOM, Headless UI, Lucide React, i18next
and react-i18next, with exact resolutions in package-lock.json. Vite/TypeScript
are build-time tools, not target-machine services. The frontend build collects
license notices from the installed production dependency graph into
web/dist/NOTICE.frontend.txt; CI includes this as licenses/frontend.txt. Lucide's
ISC/MIT notices must be retained, including inherited Feather attribution.

MCP uses the official `modelcontextprotocol/rust-sdk` crates `rmcp` and
`rmcp-macros` 3.1.2 (Apache-2.0), with schemars (MIT) for tool schemas. Credential
creation uses rand's OS randomness and RustCrypto sha2 (MIT/Apache-2.0); no custom
cryptographic primitive or MCP framing is implemented. Cargo.lock records exact
versions. CI collects their upstream notices using `scripts/mcp-notices.py` into
`licenses/mcp.txt`. The rmcp 3.1.2 crate archives omit a license file; the upstream
root LICENSE from VCS commit `02c62aef2e331e5cf79c06c744eb1eb052cc8ebd` (including its
licensing transition notice and both license texts) is retained verbatim in
`docs/licenses/rmcp.txt` and used as the packaging fallback for both SDK crates.
The Cua repository was consulted as interface documentation only; no Cua runtime,
images or source are embedded.
