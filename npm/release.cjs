"use strict";

const { execFileSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { buildPlatform, buildMain, crateVersion } = require("./build-packages.cjs");
const { PLATFORMS } = require("./doriscli/platforms.js");

const ROOT = path.dirname(__dirname);
const DIST = path.join(__dirname, "dist");
const BUILD_CACHE = path.join(ROOT, "target", "npm-release");
const REGISTRY = "https://registry.npmjs.org";
const RUST_IMAGE = process.env.DORISCLI_RUST_IMAGE || "rust:1-bookworm";
const LINUX_PLATFORMS = { "linux-x64": "linux/amd64", "linux-arm64": "linux/arm64" };
const PACKAGE_DIRS = [...Object.keys(PLATFORMS).map(key => `doriscli-${key}`), "doriscli"];

function run(command, args, options) {
  execFileSync(command, args, { cwd: ROOT, stdio: "inherit", ...options });
}

function buildBinary(key, { rustTarget, binName }) {
  const targetDir = path.join(BUILD_CACHE, key);
  const args = ["build", "--release", "--locked", "--target", rustTarget];
  fs.mkdirSync(targetDir, { recursive: true });

  if (key === "darwin-arm64") {
    run("cargo", args, { env: { ...process.env, CARGO_TARGET_DIR: targetDir } });
  } else {
    const platform = LINUX_PLATFORMS[key];
    if (!platform) throw new Error(`No build configuration for ${key}.`);
    const cargoCache = path.join(BUILD_CACHE, "cargo");
    fs.mkdirSync(cargoCache, { recursive: true });
    run("docker", [
      "run", "--rm", "--platform", platform,
      "--user", `${process.getuid()}:${process.getgid()}`,
      "--volume", `${ROOT}:/source:ro`,
      "--volume", `${targetDir}:/build`,
      "--volume", `${cargoCache}:/cargo`,
      "--workdir", "/source",
      "--env", "CARGO_HOME=/cargo",
      "--env", "CARGO_TARGET_DIR=/build",
      RUST_IMAGE, "cargo", ...args,
    ]);
  }

  return path.join(targetDir, rustTarget, "release", binName);
}

function buildRelease() {
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    throw new Error("Building all release packages requires Apple Silicon macOS.");
  }
  run("docker", ["info", "--format", "Docker {{.ServerVersion}} ({{.OSType}}/{{.Architecture}})"]);

  fs.mkdirSync(DIST, { recursive: true });
  const archiveDir = fs.mkdtempSync(path.join(DIST, `release-${crateVersion()}-`));
  const staging = fs.mkdtempSync(path.join(DIST, ".packages-"));
  try {
    for (const [key, platform] of Object.entries(PLATFORMS)) {
      buildPlatform(key, buildBinary(key, platform), staging);
    }
    buildMain(staging);
    for (const pkg of PACKAGE_DIRS) {
      run("npm", ["pack", "--ignore-scripts", "--pack-destination", archiveDir, path.join(staging, pkg)]);
    }
  } catch (error) {
    fs.rmSync(archiveDir, { recursive: true, force: true });
    throw error;
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }

  console.log(`Release archives: ${archiveDir}`);
  console.log(`To publish, run: node npm/release.cjs publish ${path.relative(ROOT, archiveDir)}`);
}

function releaseArchives(directory) {
  const version = crateVersion();
  return PACKAGE_DIRS.map(pkg => {
    const name = `@apache-doris/${pkg}`;
    const archive = path.resolve(directory, `apache-doris-${pkg}-${version}.tgz`);
    if (!fs.existsSync(archive)) throw new Error(`Missing release archive: ${archive}`);
    const manifest = JSON.parse(execFileSync("tar", ["-xOf", archive, "package/package.json"], {
      encoding: "utf8",
    }));
    if (manifest.name !== name || manifest.version !== version) {
      throw new Error(`${archive} must contain ${name}@${version}.`);
    }
    return { archive, name, version };
  });
}

async function alreadyPublished({ archive, name, version }) {
  const url = `${REGISTRY}/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
  const response = await fetch(url, { cache: "no-store", signal: AbortSignal.timeout(30_000) });
  if (response.status === 404) return false;
  if (!response.ok) throw new Error(`npm returned HTTP ${response.status} for ${name}@${version}.`);

  const metadata = await response.json();
  const digest = createHash("sha512").update(fs.readFileSync(archive)).digest("base64");
  if (metadata.dist?.integrity !== `sha512-${digest}`) {
    throw new Error(`${name}@${version} is already published with different contents.`);
  }
  console.log(`Already published: ${name}@${version}`);
  return true;
}

function promptToken() {
  const token = execFileSync("bash", ["-c", [
    'IFS= read -rsp "npm publish token: " token </dev/tty || exit 1',
    "printf '\\n' >&2",
    'printf "%s" "$token"',
  ].join("\n")], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  if (!token || /\s/.test(token)) throw new Error("Enter an npm token without whitespace.");
  return token;
}

async function publishRelease(directory) {
  const pending = [];
  for (const pkg of releaseArchives(directory)) {
    if (!(await alreadyPublished(pkg))) pending.push(pkg);
  }
  if (!pending.length) return;

  const token = promptToken();
  const configDir = fs.mkdtempSync(path.join(os.tmpdir(), "doriscli-npm-"));
  const config = path.join(configDir, "npmrc");
  try {
    fs.writeFileSync(config, "//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}\n", { mode: 0o600 });
    for (const { archive } of pending) {
      run("npm", ["publish", archive, "--access", "public", "--ignore-scripts", "--registry", REGISTRY,
        "--userconfig", config], { cwd: configDir, env: { ...process.env, NODE_AUTH_TOKEN: token } });
    }
  } finally {
    fs.rmSync(configDir, { recursive: true, force: true });
  }
}

async function main() {
  const [command = "build", directory, ...extra] = process.argv.slice(2);
  if (command === "build" && !directory) {
    buildRelease();
  } else if (command === "publish" && directory && !extra.length) {
    await publishRelease(directory);
  } else {
    const usage = "Usage: node npm/release.cjs [build | publish <archive-directory>]";
    if (command === "--help" && !directory) console.log(usage);
    else throw new Error(usage);
  }
}

main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
