# Build notes

## Pinned CEF input

Client builds use the exact official CEF archive recorded in `cef-distribution.json`. The manifest contains the URL, archive size, SHA-1, SHA-256, CEF/Chromium versions, release branch, and stable CEF API version.

Download and extract the verified distribution with Node.js:

```powershell
node scripts/download-cef.mjs --output third_party/cef
$env:CEF_PATH = (Resolve-Path third_party/cef).Path
```

The downloader refuses to replace an unrelated output directory or reuse an archive whose size or SHA-256 does not match the manifest. `CEF_PATH` is the distribution root containing `Release/libcef.lib`, `Resources/`, and `include/`. Use `--archive <path>` to select the archive cache or `--download-only` to verify the download without extracting it.

## Native Windows build

Install stable Rust, the 32-bit MSVC target, Visual Studio 2022 Build Tools with C++, and Node.js 20 or newer.

```powershell
rustup target add i686-pc-windows-msvc
$env:CEF_PATH = (Resolve-Path third_party/cef).Path

cargo build --release --target i686-pc-windows-msvc `
  -p cef-client -p cef-renderer -p cef-loader
```

Outputs are written to `target/i686-pc-windows-msvc/release/`:

- `cef_loader.dll` becomes `cef.asi` in the package;
- `cef_client.dll` becomes `cef/cef-client.dll` and is loaded by the ASI loader;
- `cef-renderer.exe` is the CEF renderer subprocess.

## Runtime package

Never assemble a CEF 150 installation by copying only three binaries. Package the compiled client together with the complete required CEF runtime:

```powershell
node scripts/package-client.mjs --cef $env:CEF_PATH --output redist
```

Optional arguments:

- `--target <directory>` selects another directory containing `cef_loader.dll`, `cef_client.dll`, and `cef-renderer.exe`;
- `--output <directory>` changes the package destination;
- `--cef <directory>` overrides `CEF_PATH`.

The packager verifies the installed CEF manifest before copying files. It creates `redist/cef.asi`, `redist/cef/`, and `redist/package-manifest.json`. The package manifest records the CEF API version, target triple, and SHA-256 of every runtime payload file.

## Tests and linting

CEF-linked test executables need `Release/libcef.dll` on `PATH`:

```powershell
$env:CEF_PATH = (Resolve-Path third_party/cef).Path
$env:PATH = "$env:CEF_PATH\Release;$env:PATH"

cargo test --release --target i686-pc-windows-msvc -p cef -p cef-client
cargo clippy --release --target i686-pc-windows-msvc -p cef-client --no-deps
```

## Regenerating CEF bindings

Bindings are generated from the headers in the pinned distribution and the stable API version in `cef-distribution.json`:

```powershell
$env:CEF_PATH = (Resolve-Path third_party/cef).Path
$env:LIBCLANG_PATH = "C:/path/to/llvm/bin"
cargo install bindgen-cli --version 0.72.1 --locked
node scripts/generate-cef-bindings.mjs
```

The generator targets Windows x86 and normalizes CEF callbacks to Rust's Windows `system` ABI.

## Cross-compiling the Windows client from macOS/Linux

Install:

- `cargo-xwin` with `cargo install cargo-xwin --locked`;
- Node.js, `curl`, `7z`, `nasm`, and LLVM with `llvm-lib`;
- Rust's `i686-pc-windows-msvc` target.

Run:

```sh
scripts/build-client-win32.sh
```

The script downloads and verifies CEF when `CEF_PATH` is unset. Set `DX_SDK` to an existing DirectX SDK June 2010 `Lib/x86` directory, or allow the script to download and extract it into `third_party/dxsdk`.

## SA:MP server plugin

Build the legacy 32-bit Linux server plugin separately from the Windows client crates:

```sh
rustup target add i686-unknown-linux-gnu
cargo build --release --package cef-server --target i686-unknown-linux-gnu
```

## open.mp component

Default local build:

```sh
scripts/build-openmp-component.sh --openmp-root /path/to/open.mp
```

Build and install into a server:

```sh
scripts/build-openmp-component.sh \
  --openmp-root /path/to/open.mp \
  --server-root /path/to/omp-server
```

Add `--clean` for a clean rebuild. The open.mp checkout can also be provided through `OPEN_MP_ROOT`. Installed component artifacts go into `<server>/components`; the component load name is `CEF`.

See `openmp-component/README.md` for direct CMake commands and configuration details.
