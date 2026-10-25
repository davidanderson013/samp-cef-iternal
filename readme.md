# samp-cef

`samp-cef` embeds Chromium Embedded Framework into GTA San Andreas Multiplayer and lets a gamemode or a client-side plugin render HTML, CSS, and JavaScript interfaces in-game.

This repository is a framework/SDK rather than a standalone mod. Using it requires a server plugin or open.mp component, the client package, and a web interface built for the browser API.

## Current status

| Area | Supported configuration |
| --- | --- |
| Client | Windows x86, Windows 10 or newer |
| SA:MP client | 0.3.DL R1, 0.3.7 R1, and 0.3.7 R3; DL R1 and R3-2 are covered by smoke tests |
| CEF | Official stable `windows32` standard distribution, CEF 150 |
| Rendering | Software BGRA off-screen rendering; GPU and GPU compositing are currently disabled |
| Media | WebM VP9 + Opus; proprietary H.264 + AAC codecs are not included |
| Server | SA:MP plugin and native open.mp component |

SA:MP 0.3.7 R2 is not currently supported by the client API. Do not mix client binaries from different SA:MP revisions.

## Features

- Create browser views from a gamemode or a client-side plugin through the C ABI.
- Render a browser on an SA:MP object texture, including spatial audio settings.
- Exchange typed events between Pawn, the browser process, and client plugins.
- Load HTTP/HTTPS pages or sandboxed local assets from `<GTA>/cef/assets`.
- Forward mouse and keyboard input, resize events, DevTools requests, and browser audio.
- Run the server side as either a legacy SA:MP plugin or an open.mp component.

## Installing the client package

Download the `cef.zip` artifact from [GitHub Actions](https://github.com/zottce/samp-cef/actions), or build and package it using the instructions below. Extract it into the GTA San Andreas directory:

```text
gta_sa.exe
cef.asi
cef/
  cef-client.dll
  cef-renderer.exe
  libcef.dll
  chrome_elf.dll
  locales/
  resources.pak
  ...
```

Keep the complete `cef/` directory from the package. CEF 150 requires more runtime files than the old CEF 89 client, so copying only `libcef.dll`, `cef-client.dll`, and `cef-renderer.exe` is not sufficient.

Client logging is configured in `<GTA>/cef/config.json`. The `log_level` field accepts `off`, `error`, `warn`, `info`, `debug`, or `trace` and defaults to `info`. Client events are written to `<GTA>/cef_client.log`.

Server logging also defaults to `info`. Set the `SAMP_CEF_LOG_LEVEL` environment variable to `off`, `error`, `warn`, `info`, `debug`, or `trace` before starting a legacy SA:MP or open.mp server to override it.

Local pages belong in `<GTA>/cef/assets/`. Browser cache, cookies, logs, and per-instance data are stored under `<GTA>/cef/user_data/`.

Launch multiplayer through `samp.exe`; starting `gta_sa.exe` directly launches single-player GTA.

## Building the Windows x86 client

Requirements:

- stable Rust with the `i686-pc-windows-msvc` target;
- Visual Studio 2022 Build Tools with the C++ toolchain;
- Node.js 20 or newer;
- `tar` with bzip2 support (included with current Windows versions).

From PowerShell:

```powershell
rustup target add i686-pc-windows-msvc

node scripts/download-cef.mjs --output third_party/cef
$env:CEF_PATH = (Resolve-Path third_party/cef).Path

cargo build --release --target i686-pc-windows-msvc `
  -p cef-client -p cef-renderer -p cef-loader

node scripts/package-client.mjs --cef $env:CEF_PATH --output redist
```

The package is written to `redist/`:

- `redist/cef.asi` is the loader;
- `redist/cef/` is the complete client runtime;
- `redist/package-manifest.json` contains SHA-256 hashes for every runtime payload file.

`CEF_PATH` must point to the root of the extracted official distribution. Its import library must be at `Release/libcef.lib`.

See [docs/build.md](docs/build.md) for tests, cross-compilation, bindings generation, and server builds.

## Pinned CEF distribution

The exact distribution is defined in [cef-distribution.json](cef-distribution.json). Build and CI scripts never resolve a floating `latest` archive.

- CEF: `150.0.11+gb887805+chromium-150.0.7871.115`
- Chromium: `150.0.7871.115`
- Release branch: `7871`
- Stable CEF API version: `15000`
- Platform: `windows32`
- Distribution: official `standard` archive
- SHA-256: `03b6b34328ef943d04bbd06479b097de4046e89439ae2d6122d0e95e517a7909`

This archive intentionally uses the official codec set. H.264 and AAC are expected to be unavailable; proprietary codecs are outside the current migration scope.

## Smoke testing

The fixture in [examples/cef150-smoke](examples/cef150-smoke/README.md) verifies the behavior expected from the previous client plugin:

- local HTML with the correct MIME type;
- HTTP fetch and browser/renderer JS IPC;
- keyboard, mouse, resize, and software OSR updates;
- object texture replacement on SA:MP R3;
- WebM VP9 + Opus playback;
- expected rejection of H.264 + AAC.

## Workspace crates

- `cef-sys` — generated Windows x86 CEF C API bindings.
- `cef` — safe and ref-counted Rust wrappers around the CEF C API.
- `cef-client` — the injected SA:MP client plugin.
- `cef-renderer` — the CEF renderer subprocess.
- `cef-loader` — the ASI loader, packaged as `cef.asi`.
- `cef-api` — API used by third-party client plugins.
- `cef-interface` — example client-side interface plugin.
- `cef-messages`, `cef-network` — protocol messages and transport.
- `cef-server`, `cef-server-core` — legacy SA:MP server plugin implementation.
- `cef-openmp` — native open.mp component bridge.
- `openmp-component` — native open.mp component.

## Server builds

Build only the server target appropriate for the host. Building the entire workspace on Linux also selects Windows-only crates.

```sh
cargo build --release --package cef-server --target i686-unknown-linux-gnu
```

For open.mp, use:

```sh
scripts/build-openmp-component.sh --openmp-root /path/to/open.mp
scripts/build-openmp-component.sh --openmp-root /path/to/open.mp --server-root /path/to/omp-server
```

More details are in [openmp-component/README.md](openmp-component/README.md).

## API documentation

- [English API notes](docs/main_en.md)
- [Russian API notes](docs/main_ru.md)
- [Build notes](docs/build.md)

## Video examples

- [Full-house browser surface](https://www.youtube.com/watch?v=Jh9IBlOKoVM)
- [Basic interfaces](https://www.youtube.com/watch?v=jU-O8_t1AfI)
- [Custom GTA interface](https://www.youtube.com/watch?v=qs7n8LoVYs4)
- [Voice chat](https://www.youtube.com/watch?v=vcyTjn3RJhs)
- [In-game TV example](https://www.youtube.com/watch?v=6OnCSHKcOGU)
