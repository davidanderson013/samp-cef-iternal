# CEF binding generation

`wrapper.h` is the bindgen entry point for the CEF C API. Generate the Windows x86 bindings from the exact distribution pinned in `cef-distribution.json`:

```powershell
$env:CEF_PATH = (Resolve-Path third_party/cef).Path
$env:LIBCLANG_PATH = "C:/path/to/llvm/bin"

cargo install bindgen-cli --version 0.72.1 --locked
node scripts/generate-cef-bindings.mjs
```

Run the command from the repository root. `CEF_PATH` must contain `include/cef_version.h` and the headers used by `utils/wrapper.h`.

The generator selects stable `CEF_API_VERSION` 15000 from `cef-distribution.json`, targets 32-bit Windows, and normalizes CEF callbacks to Rust's Windows `system` ABI. Review and compile the generated `cef-sys/src/bindings.rs` before committing it.
