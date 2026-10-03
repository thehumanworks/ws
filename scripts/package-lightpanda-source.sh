#!/bin/sh
# Package the exact, hash-pinned upstream source for a ws release directory.
set -eu
destination=$1
source_file="$destination/lightpanda-source-1.0.0.tar.gz"
curl -qfsSL --proto '=https' --proto-redir '=https' --max-time 120 \
  https://codeload.github.com/lightpanda-io/browser/tar.gz/588f6223b9cae8a2406aeef035ed9363a3e404fd \
  -o "$source_file"
python3 - "$source_file" <<'PY'
import hashlib, pathlib, sys
expected = "48a9781aca4ec577b3795944f84eccb363746feeceda7d214615332e4167917b"
if hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest() != expected:
    raise SystemExit("Lightpanda source archive checksum mismatch")
PY
cp THIRD_PARTY.md assets/LICENSE.lightpanda assets/lightpanda-build.zig.zon assets/lightpanda-pins.txt "$destination/"
