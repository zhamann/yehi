// Lays out the npm packages for a release from built binaries:
//   node npm/stage.mjs --artifacts <dir> --out <dir> [--require-all]
// <artifacts>/<platform>/yehi (yehi.exe on Windows) for each platform built. Writes <out>/yehi-<platform>/ for each,
// and <out>/yehi/, the launcher package that depends on all of them. The version comes from Cargo.toml.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");

export const PLATFORMS = {
  "darwin-arm64": { os: "darwin", cpu: "arm64", target: "aarch64-apple-darwin", label: "macOS on Apple silicon" },
  "darwin-x64": { os: "darwin", cpu: "x64", target: "x86_64-apple-darwin", label: "macOS on Intel" },
  "linux-x64": { os: "linux", cpu: "x64", libc: "glibc", target: "x86_64-unknown-linux-gnu", label: "Linux x64" },
  "linux-arm64": { os: "linux", cpu: "arm64", libc: "glibc", target: "aarch64-unknown-linux-gnu", label: "Linux arm64" },
  "win32-x64": { os: "win32", cpu: "x64", target: "x86_64-pc-windows-msvc", label: "Windows x64" },
};

function args() {
  const a = process.argv.slice(2);
  const get = (k) => {
    const i = a.indexOf(k);
    return i >= 0 ? a[i + 1] : undefined;
  };
  const artifacts = get("--artifacts");
  const out = get("--out");
  if (!artifacts || !out) {
    console.error("usage: node npm/stage.mjs --artifacts <dir> --out <dir> [--require-all]");
    process.exit(2);
  }
  return { artifacts, out, requireAll: a.includes("--require-all") };
}

function version() {
  const toml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  const m = toml.match(/^\[package\][^[]*?^version\s*=\s*"([^"]+)"/ms);
  if (!m) throw new Error("no package version in Cargo.toml");
  return m[1];
}

function write(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, typeof data === "string" ? data : JSON.stringify(data, null, 2) + "\n");
}

const { artifacts, out, requireAll } = args();
const v = version();
const main = JSON.parse(fs.readFileSync(path.join(here, "yehi", "package.json"), "utf8"));
fs.rmSync(out, { recursive: true, force: true });

const built = [];
for (const [platform, p] of Object.entries(PLATFORMS)) {
  const exe = p.os === "win32" ? "yehi.exe" : "yehi";
  const src = path.join(artifacts, platform, exe);
  if (!fs.existsSync(src)) {
    if (requireAll) throw new Error(`missing ${src}`);
    continue;
  }
  const dir = path.join(out, `yehi-${platform}`);
  const pkg = {
    name: `yehi-${platform}`,
    version: v,
    description: `The yehi binary for ${p.label}. Install yehi, not this.`,
    homepage: main.homepage,
    repository: main.repository,
    os: [p.os],
    cpu: [p.cpu],
    ...(p.libc ? { libc: [p.libc] } : {}),
    files: ["bin", "LICENSE"],
    preferUnplugged: true,
  };
  if (main.license) pkg.license = main.license;
  write(path.join(dir, "package.json"), pkg);
  write(path.join(dir, "README.md"), `# yehi-${platform}\n\nThe [yehi](https://www.npmjs.com/package/yehi) binary for ${p.label}. Install \`yehi\` instead; npm picks this package for you.\n`);
  fs.copyFileSync(path.join(root, "LICENSE"), path.join(dir, "LICENSE"));
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
  fs.copyFileSync(src, path.join(dir, "bin", exe));
  fs.chmodSync(path.join(dir, "bin", exe), 0o755);
  built.push(platform);
}

const dir = path.join(out, "yehi");
fs.cpSync(path.join(here, "yehi"), dir, { recursive: true });
fs.chmodSync(path.join(dir, "bin", "yehi.js"), 0o755);
fs.copyFileSync(path.join(root, "README.md"), path.join(dir, "README.md"));
fs.copyFileSync(path.join(root, "LICENSE"), path.join(dir, "LICENSE"));
main.version = v;
main.optionalDependencies = Object.fromEntries(
  (requireAll ? Object.keys(PLATFORMS) : built).map((p) => [`yehi-${p}`, v]),
);
write(path.join(dir, "package.json"), main);
console.log(`staged yehi ${v} with ${built.join(", ")} in ${out}`);
