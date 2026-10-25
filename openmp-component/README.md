# open.mp component

This crate builds the Rust side of the native open.mp CEF component. The C++ component wrapper is in `component/` and links the Rust static library through `cxx`.

## Build

Recommended build from the repository root:

```sh
scripts/build-openmp-component.sh --openmp-root /path/to/open.mp
```

The script expects:

- `OPEN_MP_ROOT` or `--openmp-root`
- `target/openmp-component-build` for the CEF component build directory

To build and install into an open.mp server:

```sh
scripts/build-openmp-component.sh --openmp-root /path/to/open.mp --server-root /path/to/omp-server
```

This installs `cef.*` into `/path/to/omp-server/components`.

The output library is `target/openmp-component-build/cef.*`. Put it into the open.mp `components` directory and add `CEF`/`cef` to the server `components` config list.

The component binds its CEF transport to `network.bind:(network.port + cef.port_offset)`. The default `cef.port_offset` is `2`, matching the SA:MP plugin behavior.

## CMake

The script is the easiest path, but the component can still be built directly with CMake:

```sh
cmake -S openmp-component/component -B target/openmp-component-build \
  -DOPEN_MP_ROOT=/path/to/open.mp
cmake --build target/openmp-component-build --target cef
```

CMake presets are available from the component source directory:

```sh
export OPEN_MP_ROOT=/path/to/open.mp
cmake -S openmp-component/component --preset default
(cd openmp-component/component && cmake --build --preset default)
```

The install rules are isolated under the `cef-openmp` CMake install component, so installing CEF does not also install open.mp SDK dependencies:

```sh
cmake --install target/openmp-component-build --component cef-openmp
```
