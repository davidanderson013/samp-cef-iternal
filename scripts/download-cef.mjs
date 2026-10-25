#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { access, copyFile, mkdir, readFile, rename, rm, stat } from "node:fs/promises";
import { pipeline } from "node:stream/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(scriptDir, "..");
const manifestPath = path.join(rootDir, "cef-distribution.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));

let outputDir = path.join(rootDir, "third_party", "cef");
let archivePath = path.join(rootDir, "third_party", "downloads", manifest.archive);
let downloadOnly = false;

for (let i = 2; i < process.argv.length; i += 1) {
  const argument = process.argv[i];
  if (argument === "--output" && process.argv[i + 1]) {
    outputDir = path.resolve(process.argv[++i]);
  } else if (argument === "--archive" && process.argv[i + 1]) {
    archivePath = path.resolve(process.argv[++i]);
  } else if (argument === "--download-only") {
    downloadOnly = true;
  } else {
    throw new Error(`Unknown or incomplete argument: ${argument}`);
  }
}

async function exists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function hashFile(filePath, algorithm) {
  const hash = createHash(algorithm);
  await pipeline(createReadStream(filePath), hash);
  return hash.digest("hex");
}

async function verifyArchive(filePath) {
  const fileStat = await stat(filePath);
  if (fileStat.size !== manifest.size) {
    throw new Error(`CEF archive size mismatch: expected ${manifest.size}, got ${fileStat.size}`);
  }

  const digest = await hashFile(filePath, "sha256");
  if (digest !== manifest.sha256) {
    throw new Error(`CEF archive SHA-256 mismatch: expected ${manifest.sha256}, got ${digest}`);
  }
}

async function downloadArchive() {
  if (await exists(archivePath)) {
    try {
      await verifyArchive(archivePath);
      console.log(`Using verified CEF archive: ${archivePath}`);
      return;
    } catch (error) {
      throw new Error(`Existing archive is invalid; remove it before retrying. ${error.message}`);
    }
  }

  await mkdir(path.dirname(archivePath), { recursive: true });
  const temporaryPath = `${archivePath}.partial-${process.pid}`;
  console.log(`Downloading ${manifest.url}`);

  try {
    const response = await fetch(manifest.url, { redirect: "follow" });
    if (!response.ok || !response.body) {
      throw new Error(`Download failed with HTTP ${response.status}`);
    }
    await pipeline(response.body, createWriteStream(temporaryPath, { flags: "wx" }));
    await verifyArchive(temporaryPath);
    await rename(temporaryPath, archivePath);
  } catch (error) {
    await rm(temporaryPath, { force: true });
    throw error;
  }

  console.log(`Verified CEF archive: ${archivePath}`);
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: "inherit" });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

function tarExecutable() {
  if (process.platform !== "win32") return "tar";

  const windowsRoot = process.env.SystemRoot ?? process.env.WINDIR;
  if (!windowsRoot) {
    throw new Error("SystemRoot is not set; cannot locate the Windows tar executable");
  }

  // Git Bash prepends its GNU tar to PATH. GNU tar treats a native path such
  // as D:\\a\\... as host:path remote archive syntax and fails with
  // "Cannot connect to D". Windows bsdtar accepts the native paths passed by
  // Node, so resolve it explicitly instead of depending on the caller's PATH.
  return path.join(windowsRoot, "System32", "tar.exe");
}

async function isInstalledDistribution() {
  const installedManifestPath = path.join(outputDir, ".cef-distribution.json");
  if (!(await exists(installedManifestPath))) return false;

  const installed = JSON.parse(await readFile(installedManifestPath, "utf8"));
  return (
    installed.sha256 === manifest.sha256 &&
    (await exists(path.join(outputDir, "include", "cef_version.h"))) &&
    (await exists(path.join(outputDir, "Release", "libcef.lib")))
  );
}

async function extractDistribution() {
  if (await isInstalledDistribution()) {
    console.log(`CEF distribution is already installed: ${outputDir}`);
    return;
  }
  if (await exists(outputDir)) {
    throw new Error(`Refusing to replace existing directory: ${outputDir}. Remove or move it first.`);
  }

  const temporaryDir = `${outputDir}.extract-${process.pid}`;
  await mkdir(temporaryDir, { recursive: true });
  try {
    await run(tarExecutable(), ["-xjf", archivePath, "-C", temporaryDir]);
    const extractedRoot = path.join(temporaryDir, manifest.archiveRoot);
    if (!(await exists(path.join(extractedRoot, "Release", "libcef.lib")))) {
      throw new Error("Extracted CEF distribution does not contain Release/libcef.lib");
    }
    await copyFile(manifestPath, path.join(extractedRoot, ".cef-distribution.json"));
    await mkdir(path.dirname(outputDir), { recursive: true });
    await rename(extractedRoot, outputDir);
  } finally {
    await rm(temporaryDir, { recursive: true, force: true });
  }

  console.log(`CEF_PATH=${outputDir}`);
}

if (!downloadOnly && (await isInstalledDistribution())) {
  console.log(`CEF distribution is already installed: ${outputDir}`);
} else {
  await downloadArchive();
  if (!downloadOnly) await extractDistribution();
}
