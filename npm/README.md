# Publish Apache Doris CLI to npm

Run the commands below from the repository root to build the packages and
publish them from your terminal. [Release script](release.cjs)

## Prerequisites

- Apple Silicon Mac with Rust, Xcode Command Line Tools, and Docker that can
  run `linux/arm64` and `linux/amd64` containers.
- Node.js 20 or later and npm.
- A granular npm access token with write access to all four Doris CLI packages
  and **Bypass 2FA** enabled. [Create an access token](https://docs.npmjs.com/creating-and-viewing-access-tokens/)
- Permission to push release tags to `apache/doris-cli`.

## 1. Prepare the release commit

Set `[package].version` in [`Cargo.toml`](../Cargo.toml) to the new version.
Run `cargo check` to update `Cargo.lock`, then commit and merge both files.
Check out the commit you want to release before building.

## 2. Build the npm archives

```bash
node npm/release.cjs build
```

The script builds macOS ARM64 on the host and Linux x64 and ARM64 in Docker.
It prints the directory under `npm/dist/` containing the four `.tgz` files.
This step needs no npm token.

If Docker Hub cannot pull `rust:1-bookworm`, use the mirror for the build.

```bash
DORISCLI_RUST_IMAGE=mirror.gcr.io/library/rust:1-bookworm node npm/release.cjs build
```

## 3. Tag and publish

Replace `X.Y.Z` with the version from `Cargo.toml`. Use the exact archive
directory printed by the build.

```bash
git tag -a vX.Y.Z -m "doriscli X.Y.Z"
git push https://github.com/apache/doris-cli.git vX.Y.Z
node npm/release.cjs publish npm/dist/release-X.Y.Z-XXXXXX
```

Enter the token at the `npm publish token` prompt. The terminal does not echo
your input. The script publishes the three platform packages first and the
`@apache-doris/doriscli` launcher last.

If publishing stops partway through, rerun `publish` with the same archive
directory. The script skips archives already published with identical
contents and stops if npm has different contents for that version.

## 4. Verify the release

```bash
npm view @apache-doris/doriscli@X.Y.Z version
npm view @apache-doris/doriscli@X.Y.Z optionalDependencies
npm exec --yes --package=@apache-doris/doriscli@X.Y.Z -- doriscli --version
```

Confirm that the launcher and all three platform dependencies show `X.Y.Z`.
See the [release runbook](../RELEASING.md#troubleshooting) for troubleshooting.
