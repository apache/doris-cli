"use strict";
// Canonical table of supported platforms — the single source of truth shared by
// the bin launcher (bin/doriscli) and the build script (npm/build-packages.cjs).
//
// The key is `${process.platform}-${process.arch}`, which also doubles as:
//   - the npm "<os>-<cpu>" pair (e.g. darwin-arm64), and
//   - the platform sub-package name suffix: `doriscli-<key>`.
const PLATFORMS = {
  "darwin-arm64": { rustTarget: "aarch64-apple-darwin", binName: "doriscli" },
  "darwin-x64": { rustTarget: "x86_64-apple-darwin", binName: "doriscli" },
  "linux-x64": { rustTarget: "x86_64-unknown-linux-gnu", binName: "doriscli" },
  "linux-arm64": { rustTarget: "aarch64-unknown-linux-gnu", binName: "doriscli" },
  "win32-x64": { rustTarget: "x86_64-pc-windows-msvc", binName: "doriscli.exe" },
};

module.exports = { PLATFORMS };
