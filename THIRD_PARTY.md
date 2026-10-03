# Bundled Lightpanda

macOS and GNU Linux builds embed a gzip-compressed, unmodified upstream
Lightpanda 1.0.0 executable. It is extracted and run as a separate process.
Copyright (C) 2023-2026 Lightpanda (Selecy SAS), Francis Bouvier and Pierre Tachoire.
Upstream identifies the default project license as AGPL-3.0-only; source headers
also mention AGPL version 3 or later. The upstream license is reproduced in
`assets/LICENSE.lightpanda`. No inference about the licensing of this combination
is made from its process boundary.

Immutable source commit: `588f6223b9cae8a2406aeef035ed9363a3e404fd`.
Repository: https://github.com/lightpanda-io/browser
Release: https://github.com/lightpanda-io/browser/releases/tag/1.0.0
Executable size/SHA256 pins: `assets/lightpanda-pins.txt`.
Exact upstream dependency source URLs and hashes: `assets/lightpanda-build.zig.zon`.

Release packages include `lightpanda-source-1.0.0.tar.gz`, the exact pinned
upstream source snapshot, this notice, the license and dependency source
references. `scripts/package-lightpanda-source.sh` obtains and verifies that
snapshot. It contains the upstream build instructions and dependency manifest;
dependencies have their own licenses, notices and corresponding source.

Distribution requires reviewing applicable AGPL corresponding-source and notice
obligations, including build dependencies and the licensing of the combined
distribution. Bundling the tag snapshot and links alone is not a claim that all
such obligations have been fulfilled. Release maintainers must retain the
matching source and dependency materials for binaries they distribute.

Lightpanda has no graphical renderer: its PNG output renders page text, rather
than a Chromium-style screenshot. Linux browser executables depend on glibc;
the GNU target names reflect this distribution requirement.
