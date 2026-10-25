#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { cp, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1];
}

const cefRoot = path.resolve(argument("--cef", process.env.CEF_PATH ?? path.join(rootDir, "third_party", "cef")));
const targetDir = path.resolve(argument("--target", path.join(rootDir, "target", "i686-pc-windows-msvc", "release")));
const outputDir = path.resolve(argument("--output", path.join(rootDir, "redist")));
const cefOutputDir = path.join(outputDir, "cef");

const releaseFiles = [
  "libcef.dll",
  "chrome_elf.dll",
  "d3dcompiler_47.dll",
  "libEGL.dll",
  "libGLESv2.dll",
  "v8_context_snapshot.bin",
  "vk_swiftshader.dll",
  "vk_swiftshader_icd.json",
  "vulkan-1.dll",
];

const resourceFiles = [
  "chrome_100_percent.pak",
  "chrome_200_percent.pak",
  "icudtl.dat",
  "resources.pak",
];

async function assertFile(file) {
  if (!(await stat(file).catch(() => undefined))?.isFile()) {
    throw new Error(`Required file is missing: ${file}`);
  }
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

async function walk(directory, prefix = "") {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) {
      files.push(...await walk(path.join(directory, entry.name), relative));
    } else if (entry.isFile()) {
      files.push(relative);
    }
  }
  return files.sort((a, b) => a.localeCompare(b, "en"));
}

const manifest = JSON.parse(await readFile(path.join(rootDir, "cef-distribution.json"), "utf8"));
const installedManifestPath = path.join(cefRoot, ".cef-distribution.json");
await assertFile(installedManifestPath);
const installedManifest = JSON.parse(await readFile(installedManifestPath, "utf8"));
if (installedManifest.cefVersion !== manifest.cefVersion || installedManifest.sha256 !== manifest.sha256) {
  throw new Error(`CEF distribution does not match cef-distribution.json: ${cefRoot}`);
}

const inputs = [
  [path.join(targetDir, "cef_loader.dll"), path.join(outputDir, "cef.asi")],
  [path.join(targetDir, "cef_client.dll"), path.join(cefOutputDir, "cef-client.dll")],
  [path.join(targetDir, "cef-renderer.exe"), path.join(cefOutputDir, "cef-renderer.exe")],
  [path.join(rootDir, "client", "config.json"), path.join(cefOutputDir, "config.json")],
  ...releaseFiles.map((name) => [path.join(cefRoot, "Release", name), path.join(cefOutputDir, name)]),
  ...resourceFiles.map((name) => [path.join(cefRoot, "Resources", name), path.join(cefOutputDir, name)]),
  [path.join(cefRoot, "LICENSE.txt"), path.join(cefOutputDir, "LICENSE.txt")],
  [path.join(cefRoot, "CREDITS.html"), path.join(cefOutputDir, "CREDITS.html")],
  [path.join(rootDir, "cef-distribution.json"), path.join(cefOutputDir, "cef-distribution.json")],
];

for (const [source] of inputs) {
  await assertFile(source);
}

const existingOutput = await stat(outputDir).catch(() => undefined);
if (existingOutput && !existingOutput.isDirectory()) {
  throw new Error(`Package output exists and is not a directory: ${outputDir}`);
}
if (existingOutput && (await readdir(outputDir)).length > 0) {
  const previousManifest = await readFile(path.join(outputDir, "package-manifest.json"), "utf8")
    .then(JSON.parse)
    .catch(() => undefined);
  if (
    previousManifest?.target !== "i686-pc-windows-msvc"
    || !previousManifest?.files?.["cef/cef-distribution.json"]
  ) {
    throw new Error(`Refusing to replace a directory that is not a client package: ${outputDir}`);
  }
}

await rm(outputDir, { recursive: true, force: true });
await mkdir(cefOutputDir, { recursive: true });
for (const [source, destination] of inputs) {
  await cp(source, destination);
}
await cp(path.join(cefRoot, "Resources", "locales"), path.join(cefOutputDir, "locales"), { recursive: true });

const packagedFiles = await walk(outputDir);
const packageManifest = {
  cefVersion: manifest.cefVersion,
  cefApiVersion: manifest.apiVersion,
  target: "i686-pc-windows-msvc",
  files: Object.fromEntries(await Promise.all(packagedFiles.map(async (relative) => [
    relative.replaceAll(path.sep, "/"),
    await sha256(path.join(outputDir, relative)),
  ]))),
};
await writeFile(path.join(outputDir, "package-manifest.json"), `${JSON.stringify(packageManifest, null, 2)}\n`);

console.log(`Packaged ${packagedFiles.length} files for CEF ${manifest.cefVersion} in ${outputDir}`);
