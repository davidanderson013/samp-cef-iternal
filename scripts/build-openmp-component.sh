#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

openmp_root="${OPEN_MP_ROOT:-}"
openmp_ext_root="${OPENMP_EXT_ROOT:-}"
openmp_ext_build_dir="${OPENMP_EXT_BUILD_DIR:-}"
build_dir="${SAMP_CEF_OPENMP_BUILD_DIR:-$repo_root/target/openmp-component-build}"
grpc="${SAMP_CEF_ENABLE_GRPC_EXT:-OFF}"
build_grpc="${SAMP_CEF_BUILD_GRPC_EXT:-OFF}"
server_root="${OPEN_MP_SERVER_ROOT:-}"
install=0
clean=0
jobs=""
cmake_prefix_path="${CMAKE_PREFIX_PATH:-}"
install_prefix=""
install_component="cef-openmp"
declare -a cef_cmake_args=()
declare -a grpc_cmake_args=()
declare -a build_parallel_args=()

usage() {
	cat <<'EOF'
Usage: scripts/build-openmp-component.sh [options]

Builds the samp-cef open.mp component.

Options:
  --openmp-root PATH          Path to the open.mp checkout.
  --openmp-ext-root PATH      Path to the openmp-ext checkout.
  --openmp-ext-build-dir PATH Path to the openmp-ext build directory.
  --build-dir PATH            CEF component build directory.
  --grpc auto|on|off          Enable optional gRPC registration. Default: off.
  --build-grpc auto|on|off    Build omp-grpc before CEF. Default: off.
  --server-root PATH          Install into PATH/components after building.
  --install                   Run cmake --install after building.
  --prefix PATH               CMAKE_INSTALL_PREFIX used with --install.
  --install-component NAME    CMake install component. Default: cef-openmp.
  --cmake-prefix-path PATH    CMAKE_PREFIX_PATH for grpc/protobuf packages.
  --jobs N                    Parallel build level passed to CMake.
  --clean                     Remove the CEF component build directory first.
  --cef-cmake-arg ARG         Extra CMake configure argument for CEF.
  --grpc-cmake-arg ARG        Extra CMake configure argument for openmp-ext.
  -h, --help                  Show this help.

Examples:
  OPEN_MP_ROOT=/path/to/open.mp scripts/build-openmp-component.sh
  scripts/build-openmp-component.sh --openmp-root /path/to/open.mp --server-root /path/to/omp-server
  scripts/build-openmp-component.sh --openmp-root /path/to/open.mp --grpc on --openmp-ext-root /path/to/openmp-ext
EOF
}

upper() {
	printf '%s' "$1" | tr '[:lower:]' '[:upper:]'
}

component_ext() {
	case "$(uname -s)" in
		Darwin) printf 'dylib' ;;
		MINGW*|MSYS*|CYGWIN*) printf 'dll' ;;
		*) printf 'so' ;;
	esac
}

default_cmake_prefix_path() {
	if [[ -n "$cmake_prefix_path" || "$(uname -s)" != "Darwin" ]]; then
		return
	fi
	if ! command -v brew >/dev/null 2>&1; then
		return
	fi

	local prefixes=()
	local prefix
	for package in grpc protobuf; do
		prefix="$(brew --prefix "$package" 2>/dev/null || true)"
		if [[ -n "$prefix" && -d "$prefix" ]]; then
			prefixes+=("$prefix")
		fi
	done

	if (( ${#prefixes[@]} > 0 )); then
		local joined
		joined="$(IFS=';'; printf '%s' "${prefixes[*]}")"
		cmake_prefix_path="$joined"
	fi
}

cmake_build_target() {
	local dir="$1"
	local target="$2"

	if (( ${#build_parallel_args[@]} > 0 )); then
		cmake --build "$dir" --target "$target" "${build_parallel_args[@]}"
	else
		cmake --build "$dir" --target "$target"
	fi
}

while (( $# > 0 )); do
	case "$1" in
		--openmp-root)
			openmp_root="$2"
			shift 2
			;;
		--openmp-ext-root)
			openmp_ext_root="$2"
			shift 2
			;;
		--openmp-ext-build-dir)
			openmp_ext_build_dir="$2"
			shift 2
			;;
		--build-dir)
			build_dir="$2"
			shift 2
			;;
		--grpc)
			grpc="$2"
			shift 2
			;;
		--build-grpc)
			build_grpc="$2"
			shift 2
			;;
		--server-root)
			server_root="$2"
			install=1
			shift 2
			;;
		--install)
			install=1
			shift
			;;
		--prefix)
			install_prefix="$2"
			shift 2
			;;
		--install-component)
			install_component="$2"
			shift 2
			;;
		--cmake-prefix-path)
			cmake_prefix_path="$2"
			shift 2
			;;
		--jobs|-j)
			jobs="$2"
			shift 2
			;;
		--clean)
			clean=1
			shift
			;;
		--cef-cmake-arg)
			cef_cmake_args+=("$2")
			shift 2
			;;
		--grpc-cmake-arg)
			grpc_cmake_args+=("$2")
			shift 2
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			printf 'Unknown option: %s\n\n' "$1" >&2
			usage >&2
			exit 2
			;;
	esac
done

grpc="$(upper "$grpc")"
build_grpc="$(upper "$build_grpc")"
case "$grpc" in AUTO|ON|OFF) ;; *) printf '%s\n' '--grpc must be auto, on, or off' >&2; exit 2 ;; esac
case "$build_grpc" in AUTO|ON|OFF) ;; *) printf '%s\n' '--build-grpc must be auto, on, or off' >&2; exit 2 ;; esac

if [[ -z "$openmp_ext_build_dir" && -n "$openmp_ext_root" ]]; then
	openmp_ext_build_dir="$openmp_ext_root/build"
fi

if [[ -z "$openmp_root" ]]; then
	printf '%s\n' 'OPEN_MP_ROOT is required. Set OPEN_MP_ROOT or pass --openmp-root PATH.' >&2
	exit 2
fi

if [[ ! -f "$openmp_root/SDK/CMakeLists.txt" ]]; then
	printf 'OPEN_MP_ROOT does not look like an open.mp checkout: %s\n' "$openmp_root" >&2
	exit 1
fi

if [[ "$install" == 1 && -z "$server_root" && -z "$install_prefix" ]]; then
	printf '%s\n' '--install needs --server-root PATH or --prefix PATH' >&2
	exit 2
fi

if [[ "$grpc" != "OFF" ]]; then
	default_cmake_prefix_path
	if [[ -n "$cmake_prefix_path" ]]; then
		export CMAKE_PREFIX_PATH="$cmake_prefix_path"
	fi
fi

if (( clean )); then
	rm -rf "$build_dir"
fi

if [[ -n "$jobs" ]]; then
	build_parallel_args+=(--parallel "$jobs")
fi

ext="$(component_ext)"
if [[ "$grpc" != "OFF" ]]; then
	if [[ -n "$openmp_ext_root" && -f "$openmp_ext_root/CMakeLists.txt" ]]; then
		grpc_lib="$openmp_ext_build_dir/omp-grpc.$ext"
		proto_cc="$openmp_ext_build_dir/generated/proto/omp_ext.pb.cc"

		if [[ "$build_grpc" == "ON" || ( "$build_grpc" == "AUTO" && ( ! -f "$grpc_lib" || ! -f "$proto_cc" ) ) ]]; then
			printf '[samp-cef] Building omp-grpc in %s\n' "$openmp_ext_build_dir"
			grpc_configure_args=(
				-S "$openmp_ext_root"
				-B "$openmp_ext_build_dir"
				-DOPENMP_ROOT="$openmp_root"
			)
			if (( ${#grpc_cmake_args[@]} > 0 )); then
				grpc_configure_args+=("${grpc_cmake_args[@]}")
			fi
			cmake "${grpc_configure_args[@]}"
			cmake_build_target "$openmp_ext_build_dir" omp-grpc
		fi
	elif [[ "$grpc" == "ON" ]]; then
		printf 'openmp-ext was required but not found. Set OPENMP_EXT_ROOT or pass --openmp-ext-root PATH.\n' >&2
		exit 1
	fi
fi

printf '[samp-cef] Building CEF open.mp component in %s\n' "$build_dir"
cef_configure_args=(
	-S "$repo_root/openmp-component/component"
	-B "$build_dir"
	-DOPEN_MP_ROOT="$openmp_root"
	-DOPENMP_EXT_ROOT="$openmp_ext_root"
	-DOPENMP_EXT_BUILD_DIR="$openmp_ext_build_dir"
	-DSAMP_CEF_ENABLE_GRPC_EXT="$grpc"
	-DSAMP_CEF_INSTALL_COMPONENT_NAME="$install_component"
	-DOPEN_MP_SERVER_ROOT="$server_root"
)

if [[ -n "$install_prefix" ]]; then
	cef_configure_args+=(-DCMAKE_INSTALL_PREFIX="$install_prefix")
fi
if (( ${#cef_cmake_args[@]} > 0 )); then
	cef_configure_args+=("${cef_cmake_args[@]}")
fi

cmake "${cef_configure_args[@]}"
cmake_build_target "$build_dir" cef

if (( install )); then
	printf '[samp-cef] Installing CEF open.mp component\n'
	cmake --install "$build_dir" --component "$install_component"
fi

printf '[samp-cef] Built: %s/cef.%s\n' "$build_dir" "$ext"
installed_component_dir=""
if [[ -n "$server_root" ]]; then
	installed_component_dir="$server_root/components"
elif [[ -n "$install_prefix" ]]; then
	installed_component_dir="$install_prefix/components"
fi

if (( install )) && [[ -n "$installed_component_dir" ]]; then
	printf '[samp-cef] Installed components into: %s\n' "$installed_component_dir"
	if [[ "$grpc" == "OFF" ]]; then
		printf '[samp-cef] Load order: CEF\n'
	else
		printf '[samp-cef] Load order: $CAPI, omp-grpc, CEF\n'
	fi
fi
