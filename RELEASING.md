# Releasing Apache Doris CLI

This is the release manager's guide to publishing a new version of the Doris CLI
to npm. The CLI is distributed as **prebuilt npm packages** under the
`@apache-doris` scope; end users install it with
`npm install -g @apache-doris/doriscli`.

The whole build-and-publish pipeline is automated in
[`.github/workflows/release-npm.yml`](.github/workflows/release-npm.yml). A
release manager's job is to bump the version, validate with a dry run, and push
a tag — the workflow does the rest.

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
`npm/build-packages.cjs`, so npm always matches the crate. The git tag does not
set the version — it only has to match it.

npm versions are **immutable**: you cannot re-publish a version. Every release is
a new version number, and a broken release is fixed by shipping the next patch
(see [Fixing a broken release](#fixing-a-broken-release)).

## One-time prerequisites

- An npm account in the **`apache-doris`** npm organization with publish rights to
  the `@apache-doris` scope.
- Repository secret **`NPM_TOKEN`** set to a **Classic _Automation_ token**
  (npmjs.com → Access Tokens → Generate New Token → Classic Token → **Automation**).
  The org enforces two-factor auth for writes, and an Automation token is the type
  that bypasses 2FA in CI. A granular or read-only token will fail — see
  [Troubleshooting](#troubleshooting).
- Permission to push tags to `apache/doris-cli`.

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

### 2. Dry-run the pipeline (recommended)

From the GitHub UI: **Actions → release-npm → Run workflow**, leaving
**`dry_run` ☑ checked** (the default). This builds every platform, assembles all
packages, and runs `npm publish --dry-run` — **no token is used and nothing is
published.** Confirm the run is green.

CLI equivalent:

```bash
gh workflow run release-npm.yml -f dry_run=true
```

### 3. Tag and push (the real publish)

```bash
git tag -a vX.Y.Z -m "doriscli X.Y.Z"
git push <apache-remote> vX.Y.Z      # the remote pointing at apache/doris-cli
```

The `v*.*.*` tag triggers `release-npm`, which builds all three platforms and
publishes. Platform packages publish first; the main package publishes last, so
its `optionalDependencies` already resolve on the registry.

### 4. Verify

After the run is green:

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

## How the pipeline works

`release-npm.yml` has two jobs:

- **build** — a matrix over the three platforms. Each runner does
  `cargo build --release`, then `node npm/build-packages.cjs platform <key> <binary>`
  assembles that platform package and uploads it as an artifact.
- **publish** — downloads the artifacts, **re-applies the executable bit**,
  assembles the main package, and `npm publish`es each one.

> ⚠️ **Do not remove the `chmod 0755` step in the publish job.**
> `actions/upload-artifact` strips the Unix executable bit, so the prebuilt
> binary arrives in the publish job as `0644`. The publish job re-runs
> `chmod 0755` on each binary *before* `npm publish`; without it, npm ships a
> non-executable binary and the CLI fails at runtime with `EACCES`. (This is
> exactly how `0.1.0` shipped broken.)

### Actions allowlist

`apache/doris-cli` is under the Apache org's GitHub Actions allowlist: only
GitHub-owned actions (`actions/*`) and **local** actions (`./.github/actions/...`)
are permitted. That's why the Rust toolchain setup is a local composite action
(`.github/actions/setup-rust-toolchain`) instead of a third-party one. **Do not
add third-party actions** to the workflow — they will be rejected.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `403 Forbidden ... You may not perform that action with these credentials` | The token can't publish that name. Ensure package names are scoped to `@apache-doris` and `NPM_TOKEN` has publish rights to the scope. |
| `403 ... Two-factor authentication ... required` | `NPM_TOKEN` doesn't bypass 2FA. Use a **Classic Automation** token, not a granular or read-only one. |
| CLI fails with `EACCES` after install | The published binary isn't executable — the `chmod 0755` step in the publish job is missing or didn't match the binary path. |
| CI fails with a lockfile / `--locked` error | `Cargo.lock` is out of sync after the version bump. Run `cargo check` (rewrites the lock) and commit `Cargo.lock`. |
| Want to add a platform | Add it to `npm/doriscli/platforms.js` **and** the build matrix in `release-npm.yml`. |

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
