#!/usr/bin/env node

import { readFile, rename, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(scriptDir, "..");
const manifest = JSON.parse(
  await readFile(path.join(rootDir, "cef-distribution.json"), "utf8"),
);
const cefRoot = path.resolve(process.env.CEF_PATH ?? path.join(rootDir, "third_party", "cef"));
const wrapper = path.join(rootDir, "utils", "wrapper.h");
const output = path.join(rootDir, "cef-sys", "src", "bindings.rs");
const temporaryOutput = `${output}.generated-${process.pid}`;

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: rootDir,
      env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? "error" },
      stdio: "inherit",
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

const args = [
  wrapper,
  "--output",
  temporaryOutput,
  "--rust-target",
  "1.93",
  "--default-enum-style",
  "moduleconsts",
  "--no-doc-comments",
  "--allowlist-type",
  "cef_.*",
  "--allowlist-function",
  "cef_.*",
  "--allowlist-var",
  "CEF_.*",
  "--",
  `-I${cefRoot}`,
  `-DCEF_API_VERSION=${manifest.apiVersion}`,
  "--target=i686-pc-windows-msvc",
];

try {
  await run("bindgen", args);
  let bindings = await readFile(temporaryOutput, "utf8");

  // Rust's `system` ABI maps to stdcall on Windows x86 and matches the
  // callback declarations used by the hand-written wrapper implementation.
  bindings = bindings.replaceAll('extern "stdcall"', 'extern "system"');

  const required = [
    `pub const CEF_API_VERSION: u32 = ${manifest.apiVersion};`,
    `b"${manifest.apiHashWindows}\\0"`,
    "pub fn cef_api_hash(",
    "pub struct _cef_settings_t",
    "pub struct _cef_render_handler_t",
    "pub struct _cef_v8_value_t",
  ];
  for (const marker of required) {
    if (!bindings.includes(marker)) {
      throw new Error(`Generated bindings are missing required marker: ${marker}`);
    }
  }

  await writeFile(temporaryOutput, bindings);
  await rename(temporaryOutput, output);
  console.log(`Generated ${output} for CEF API ${manifest.apiVersion}`);
} finally {
  await rm(temporaryOutput, { force: true });
}
