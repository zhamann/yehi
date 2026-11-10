#!/usr/bin/env node
// Runs the yehi binary from the platform package npm installed alongside this one
// (yehi-darwin-arm64, yehi-linux-x64, ...), passing arguments, output and exit status straight through.

"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");

const SUPPORTED = ["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64", "win32-x64"];

function isMusl() {
  if (process.platform !== "linux") return false;
  try {
    return !process.report.getReport().header.glibcVersionRuntime;
  } catch {
    return false;
  }
}

function binary() {
  if (process.env.YEHI_BINARY) return process.env.YEHI_BINARY;
  const platform = `${process.platform}-${process.arch}`;
  const exe = process.platform === "win32" ? "yehi.exe" : "yehi";
  try {
    return require.resolve(`yehi-${platform}/bin/${exe}`);
  } catch {
    let why;
    if (!SUPPORTED.includes(platform)) {
      why = `there's no prebuilt yehi for ${platform} (there is for ${SUPPORTED.join(", ")})`;
    } else if (isMusl()) {
      why = "the prebuilt Linux binaries need glibc, and this system uses musl (Alpine)";
    } else {
      why = `the package yehi-${platform} wasn't installed (was npm run with --omit=optional or --no-optional?)`;
    }
    console.error(`yehi: ${why}. Build it from source instead with Rust: cargo install --git https://github.com/zhamann/yehi`);
    process.exit(1);
  }
}

const bin = binary();
try {
  fs.accessSync(bin, fs.constants.X_OK);
} catch {
  try {
    fs.chmodSync(bin, 0o755);
  } catch {}
}
const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`yehi: couldn't run ${bin}: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
