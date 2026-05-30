#!/usr/bin/env node
"use strict";
/*
 * Assemble the publishable npm packages into npm/dist/.
 *
 *   node npm/build-packages.cjs platform <key> <path-to-binary>   # one platform sub-package
 *   node npm/build-packages.cjs main                              # the main "doriscli" package
 *
 * The version is single-sourced from Cargo.toml so npm always matches the crate.
 * npm/doriscli/ holds the authored sources; ALWAYS publish from npm/dist/*.
 */
const fs = require("fs");
const path = require("path");
const { PLATFORMS } = require("./doriscli/platforms.js");

const NPM_DIR = __dirname; // npm/
const ROOT = path.join(NPM_DIR, ".."); // repo root
const SRC_MAIN = path.join(NPM_DIR, "doriscli"); // authored main package
const DIST = path.join(NPM_DIR, "dist");

function crateVersion() {
  const toml = fs.readFileSync(path.join(ROOT, "Cargo.toml"), "utf8");
  // First line that starts with `version = "..."` is the [package] version
  // (dependency lines start with the dep name, never with `version`).
  const m = toml.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error("could not find a [package] version in Cargo.toml");
  return m[1];
}

function freshDir(p) {
  fs.rmSync(p, { recursive: true, force: true });
  fs.mkdirSync(p, { recursive: true });
}

function buildPlatform(key, binPath) {
  const entry = PLATFORMS[key];
  if (!entry) {
    throw new Error(`unknown platform "${key}". Known: ${Object.keys(PLATFORMS).join(", ")}`);
  }
  if (!binPath || !fs.existsSync(binPath)) {
    throw new Error(`binary not found: ${binPath}`);
  }
  const version = crateVersion();
  const [os, cpu] = key.split("-");
  const outDir = path.join(DIST, `doriscli-${key}`);
  freshDir(path.join(outDir, "bin"));

  const destBin = path.join(outDir, "bin", entry.binName);
  fs.copyFileSync(binPath, destBin);
  fs.chmodSync(destBin, 0o755);

  const pkg = {
    name: `doriscli-${key}`,
    version,
    description: `Prebuilt doriscli binary for ${key}.`,
    license: "Apache-2.0",
    repository: { type: "git", url: "git+https://github.com/morningman/doris-cli.git" },
    os: [os],
    cpu: [cpu],
    files: ["bin/"],
    // Tell Yarn PnP to keep this on disk — it's a native executable, not JS.
    preferUnplugged: true,
  };
  fs.writeFileSync(path.join(outDir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  fs.copyFileSync(path.join(ROOT, "LICENSE.txt"), path.join(outDir, "LICENSE.txt"));
  console.log(`built ${path.relative(ROOT, outDir)}  (${version}, ${os}/${cpu})`);
}

function buildMain() {
  const version = crateVersion();
  const outDir = path.join(DIST, "doriscli");
  freshDir(path.join(outDir, "bin"));

  fs.copyFileSync(path.join(SRC_MAIN, "bin", "doriscli"), path.join(outDir, "bin", "doriscli"));
  fs.chmodSync(path.join(outDir, "bin", "doriscli"), 0o755);
  fs.copyFileSync(path.join(SRC_MAIN, "platforms.js"), path.join(outDir, "platforms.js"));
  fs.copyFileSync(path.join(SRC_MAIN, "README.md"), path.join(outDir, "README.md"));
  fs.copyFileSync(path.join(ROOT, "LICENSE.txt"), path.join(outDir, "LICENSE.txt"));

  const pkg = JSON.parse(fs.readFileSync(path.join(SRC_MAIN, "package.json"), "utf8"));
  delete pkg["//"];
  pkg.version = version;
  pkg.optionalDependencies = {};
  for (const key of Object.keys(PLATFORMS)) {
    pkg.optionalDependencies[`doriscli-${key}`] = version;
  }
  fs.writeFileSync(path.join(outDir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  console.log(`built ${path.relative(ROOT, outDir)}  (${version}, ${Object.keys(PLATFORMS).length} optional deps)`);
}

const [cmd, ...rest] = process.argv.slice(2);
if (cmd === "platform") {
  buildPlatform(rest[0], rest[1]);
} else if (cmd === "main") {
  buildMain();
} else {
  process.stderr.write("usage: build-packages.cjs platform <key> <binary> | main\n");
  process.exit(2);
}
