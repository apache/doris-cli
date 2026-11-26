# Releasing Apache Doris CLI

This is the release manager's guide to publishing a new version of the Doris CLI
to npm. The CLI is distributed as **prebuilt npm packages** under the
`@apache-doris` scope; end users install it with
`npm install -g @apache-doris/doriscli`.

Use [`npm/release.cjs`](npm/release.cjs) to build and publish the packages from
your machine.

## What gets published

A release publishes four packages, all at the same version:

| Package | Contents |
|---|---|
| `@apache-doris/doriscli` | The launcher + `optionalDependencies` on the three platform packages. **This is what users install.** |
| `@apache-doris/doriscli-darwin-arm64` | Prebuilt binary for macOS arm64 |
| `@apache-doris/doriscli-linux-x64` | Prebuilt binary for Linux x64 |
| `@apache-doris/doriscli-linux-arm64` | Prebuilt binary for Linux arm64 |

On install, npm resolves **only** the platform package matching the user's
OS + CPU (via `optionalDependencies` plus each platform package's `os`/`cpu`
fields); the launcher then execs that binary. The set of platforms lives in
`npm/doriscli/platforms.js` — the single source of truth shared by the launcher
and the packaging script.

## Versioning

**The published version is read from `Cargo.toml` (`[package].version`)** by
`npm/build-packages.cjs`. Tag the release commit as `vX.Y.Z`, using the same
version as the crate.

npm versions are **immutable**: you cannot re-publish a version. Every release is
a new version number, and a broken release is fixed by shipping the next patch
(see [Fixing a broken release](#fixing-a-broken-release)).

## Prerequisites

- To build, use an Apple Silicon Mac with Rust, Xcode Command Line Tools, and
  Docker configured to run `linux/arm64` and `linux/amd64` containers.
- Install Node.js 20 or later and npm.
- To publish, use an npm granular access token with write access to the four
  `@apache-doris` packages and **Bypass 2FA** enabled. You also need permission
  to push release tags to `apache/doris-cli`.

## Release steps

### 1. Bump the version

Edit `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

Refresh the lockfile so the CI gate (which runs `--locked`) stays green, then
commit both files and merge to `main`:

```bash
cargo check                   # rewrites Cargo.lock to match the new version
git add Cargo.toml Cargo.lock && git commit -m "release X.Y.Z"
```

### 2. Build and inspect the packages

```bash
node npm/release.cjs build
```

The script builds macOS ARM64 on the host and both Linux architectures in Docker
using `rust:1-bookworm`. All builds use `Cargo.lock`. The script prints the
directory under `npm/dist/` that contains the four ready-to-publish `.tgz`
files. This command only builds and packs the archives.

If your Docker installation pulls Rust from another registry, set
`DORISCLI_RUST_IMAGE` to the equivalent image for the build command.

### 3. Tag and publish from your terminal

```bash
git tag -a vX.Y.Z -m "doriscli X.Y.Z"
git push https://github.com/apache/doris-cli.git vX.Y.Z
node npm/release.cjs publish npm/dist/release-X.Y.Z-XXXXXX
```

Use the archive directory printed by the build. The script validates all four
archives against the current `Cargo.toml` version and prompts for the npm token
without echoing it. It supplies the token to the local `npm publish` processes
and publishes the three platform packages first, followed by the main package.
If a publish fails partway through, rerun the command with the same archive
directory. The script skips any archives already published with identical
contents.

### 4. Verify

After publishing succeeds, check the version on npm and test a fresh install.

```bash
npm view @apache-doris/doriscli dist-tags          # `latest` should be X.Y.Z

# install into a throwaway dir and actually run the binary
tmp=$(mktemp -d) && ( cd "$tmp" \
  && npm install @apache-doris/doriscli \
  && ./node_modules/.bin/doriscli --version )
rm -rf "$tmp"
```

npm moves the `latest` dist-tag to the highest version automatically, so a plain
`npm install -g @apache-doris/doriscli` picks up the new release.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `403 Forbidden ... You may not perform that action with these credentials` | Check that your token grants write access to all four `@apache-doris` packages. |
| `403 ... Two-factor authentication ... required` | Check that **Bypass 2FA** is enabled on the granular access token. |
| CLI fails with `EACCES` after install | Check for mode `0755` on `package/bin/doriscli` inside the relevant platform archive. |
| CI fails with a lockfile / `--locked` error | `Cargo.lock` is out of sync after the version bump. Run `cargo check` (rewrites the lock) and commit `Cargo.lock`. |
| npm already has the version with different contents | Use the original archive directory to resume a partial publish, or bump the version for a new release. |
| Want to add a platform | Add it to `npm/doriscli/platforms.js` and add its build configuration to `npm/release.cjs`. |

## Fixing a broken release

A published version is immutable, so fix it by publishing the next patch version.
Then steer users off the bad one:

```bash
for p in doriscli doriscli-darwin-arm64 doriscli-linux-x64 doriscli-linux-arm64; do
  npm deprecate @apache-doris/$p@<bad-version> "broken; use <good-version>+"
done
```

If the bad version is only minutes old and almost certainly uninstalled, you can
instead `npm unpublish @apache-doris/<pkg>@<bad-version>` for each package
(npm allows single-version unpublish within 72 hours).
