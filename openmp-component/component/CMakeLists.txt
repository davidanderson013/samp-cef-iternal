cmake_minimum_required(VERSION 3.19)
project(cef-openmp-component LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(SAMP_CEF_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." CACHE PATH "Path to the samp-cef repository")
set(OPEN_MP_ROOT "$ENV{OPEN_MP_ROOT}" CACHE PATH "Path to the open.mp repository")

set(OPENMP_EXT_ROOT "$ENV{OPENMP_EXT_ROOT}" CACHE PATH "Path to the openmp-ext gRPC component repository")
set(OPENMP_EXT_BUILD_DIR "$ENV{OPENMP_EXT_BUILD_DIR}" CACHE PATH "Path to the openmp-ext build directory")
if(NOT OPENMP_EXT_BUILD_DIR AND OPENMP_EXT_ROOT)
	set(OPENMP_EXT_BUILD_DIR "${OPENMP_EXT_ROOT}/build")
endif()
set(SAMP_CEF_ENABLE_GRPC_EXT "OFF" CACHE STRING "Enable openmp-ext gRPC integration: AUTO, ON, or OFF")
set_property(CACHE SAMP_CEF_ENABLE_GRPC_EXT PROPERTY STRINGS AUTO ON OFF)
set(OPEN_MP_SERVER_ROOT "$ENV{OPEN_MP_SERVER_ROOT}" CACHE PATH "Optional open.mp server root used by the install target")
set(SAMP_CEF_INSTALL_COMPONENT_DIR "" CACHE PATH "Directory where open.mp component binaries are installed")
set(SAMP_CEF_INSTALL_COMPONENT_NAME "cef-openmp" CACHE STRING "CMake install component used for CEF open.mp artifacts")
set(SAMP_CEF_INSTALL_GRPC_EXT ON CACHE BOOL "Install omp-grpc next to cef when gRPC integration is enabled")

if(NOT EXISTS "${OPEN_MP_ROOT}/SDK/CMakeLists.txt")
	message(FATAL_ERROR "OPEN_MP_ROOT must point to an open.mp checkout")
endif()

find_program(CARGO_EXECUTABLE cargo REQUIRED)

set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
add_subdirectory("${OPEN_MP_ROOT}/SDK" "${CMAKE_CURRENT_BINARY_DIR}/openmp-sdk")

set(SAMP_CEF_RUST_TARGET "" CACHE STRING "Optional Cargo target triple for the Rust static library")
set(SAMP_CEF_CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-target" CACHE PATH "Cargo target directory")
set(RUST_MANIFEST "${SAMP_CEF_ROOT}/openmp-component/Cargo.toml")
set(GENERATED_INCLUDE_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated/include")
set(GENERATED_CXXBRIDGE_HEADER "${GENERATED_INCLUDE_DIR}/cef-openmp/src/lib.rs.h")

set(CARGO_BUILD_ARGS build --release --manifest-path "${RUST_MANIFEST}")
if(SAMP_CEF_RUST_TARGET)
	list(APPEND CARGO_BUILD_ARGS --target "${SAMP_CEF_RUST_TARGET}")
	set(RUST_LIB_DIR "${SAMP_CEF_CARGO_TARGET_DIR}/${SAMP_CEF_RUST_TARGET}/release")
else()
	set(RUST_LIB_DIR "${SAMP_CEF_CARGO_TARGET_DIR}/release")
endif()

if(WIN32)
	set(RUST_STATIC_LIB "${RUST_LIB_DIR}/cef_openmp.lib")
else()
	set(RUST_STATIC_LIB "${RUST_LIB_DIR}/libcef_openmp.a")
endif()

add_custom_command(
	OUTPUT
		"${RUST_STATIC_LIB}"
		"${GENERATED_CXXBRIDGE_HEADER}"
	COMMAND
		"${CMAKE_COMMAND}" -E env
			"CARGO_TARGET_DIR=${SAMP_CEF_CARGO_TARGET_DIR}"
			"${CARGO_EXECUTABLE}" ${CARGO_BUILD_ARGS}
	COMMAND
		"${CMAKE_COMMAND}"
			"-DRUST_LIB_DIR=${RUST_LIB_DIR}"
			"-DGENERATED_INCLUDE_DIR=${GENERATED_INCLUDE_DIR}"
			-P "${CMAKE_CURRENT_LIST_DIR}/copy_cxxbridge_header.cmake"
	WORKING_DIRECTORY "${SAMP_CEF_ROOT}"
	DEPENDS
		"${SAMP_CEF_ROOT}/Cargo.toml"
		"${SAMP_CEF_ROOT}/Cargo.lock"
		"${RUST_MANIFEST}"
		"${SAMP_CEF_ROOT}/openmp-component/build.rs"
		"${SAMP_CEF_ROOT}/openmp-component/src/lib.rs"
		"${SAMP_CEF_ROOT}/server-core/src/lib.rs"
		"${SAMP_CEF_ROOT}/server-core/src/server.rs"
		"${SAMP_CEF_ROOT}/server-core/src/client.rs"
	VERBATIM
)

add_custom_target(cef_openmp_rust DEPENDS "${RUST_STATIC_LIB}" "${GENERATED_CXXBRIDGE_HEADER}")

add_library(cef SHARED
	src/component.cpp
)

set(SAMP_CEF_GRPC_EXT_ENABLED FALSE)

add_dependencies(cef cef_openmp_rust)

target_include_directories(cef PRIVATE
	"${GENERATED_INCLUDE_DIR}"
	"${OPEN_MP_ROOT}/lib/pawn/source"
	"${OPEN_MP_ROOT}/lib/pawn/source/linux"
)

target_link_libraries(cef PRIVATE
	OMP-SDK
	"${RUST_STATIC_LIB}"
)

target_compile_definitions(cef PRIVATE
	OMP_VERSION_MAJOR=1
	OMP_VERSION_MINOR=5
	OMP_VERSION_PATCH=8
	FLOATPOINT
	PAWN_CELL_SIZE=32
)

string(TOUPPER "${SAMP_CEF_ENABLE_GRPC_EXT}" SAMP_CEF_ENABLE_GRPC_EXT_UPPER)
set(OPENMP_EXT_PROTO_DIR "${OPENMP_EXT_BUILD_DIR}/generated/proto")
if(APPLE)
	set(OPENMP_EXT_GRPC_DEFAULT_LIBRARY "${OPENMP_EXT_BUILD_DIR}/omp-grpc.dylib")
elseif(WIN32)
	set(OPENMP_EXT_GRPC_DEFAULT_LIBRARY "${OPENMP_EXT_BUILD_DIR}/omp-grpc.dll")
else()
	set(OPENMP_EXT_GRPC_DEFAULT_LIBRARY "${OPENMP_EXT_BUILD_DIR}/omp-grpc.so")
endif()
set(OPENMP_EXT_GRPC_LIBRARY "${OPENMP_EXT_GRPC_DEFAULT_LIBRARY}" CACHE FILEPATH "Path to the built omp-grpc shared library")

set(OPENMP_EXT_AVAILABLE FALSE)
if(OPENMP_EXT_ROOT
	AND OPENMP_EXT_BUILD_DIR
	AND EXISTS "${OPENMP_EXT_ROOT}/include/omp_ext/component.hpp"
	AND EXISTS "${OPENMP_EXT_PROTO_DIR}/omp_ext.pb.h"
	AND EXISTS "${OPENMP_EXT_PROTO_DIR}/omp_ext.pb.cc"
	AND EXISTS "${OPENMP_EXT_PROTO_DIR}/omp_ext.grpc.pb.h"
	AND EXISTS "${OPENMP_EXT_GRPC_LIBRARY}")
	set(OPENMP_EXT_AVAILABLE TRUE)
endif()

if(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "ON" OR (SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "AUTO" AND OPENMP_EXT_AVAILABLE))
	if(NOT OPENMP_EXT_AVAILABLE)
		message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT is ON, but openmp-ext headers/proto/library were not found. Set OPENMP_EXT_ROOT/OPENMP_EXT_BUILD_DIR or use -DSAMP_CEF_ENABLE_GRPC_EXT=OFF.")
	endif()

	find_package(Protobuf CONFIG QUIET)
	find_package(gRPC CONFIG QUIET)

	if(NOT TARGET protobuf::libprotobuf OR NOT TARGET gRPC::grpc++)
		if(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "ON")
			message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT is ON, but Protobuf/gRPC CMake packages were not found. Set CMAKE_PREFIX_PATH or use -DSAMP_CEF_ENABLE_GRPC_EXT=OFF.")
		endif()
		message(STATUS "openmp-ext gRPC integration disabled: Protobuf/gRPC CMake packages were not found")
	else()
		add_library(omp_grpc_ext SHARED IMPORTED)
		set_target_properties(omp_grpc_ext PROPERTIES IMPORTED_LOCATION "${OPENMP_EXT_GRPC_LIBRARY}")

		set(SAMP_CEF_GRPC_EXT_ENABLED TRUE)
		message(STATUS "openmp-ext gRPC integration enabled: ${OPENMP_EXT_GRPC_LIBRARY}")
		target_sources(cef PRIVATE "${OPENMP_EXT_PROTO_DIR}/omp_ext.pb.cc")
		target_compile_definitions(cef PRIVATE SAMP_CEF_HAS_GRPC_EXT=1)
		target_include_directories(cef PRIVATE
			"${OPENMP_EXT_ROOT}/include"
			"${OPENMP_EXT_PROTO_DIR}"
			"${OPEN_MP_ROOT}/CAPI/include"
		)
		target_link_libraries(cef PRIVATE
			omp_grpc_ext
			protobuf::libprotobuf
			gRPC::grpc++
		)

		if(APPLE)
			set_property(TARGET cef APPEND PROPERTY BUILD_RPATH "@loader_path")
		elseif(UNIX)
			set_property(TARGET cef APPEND PROPERTY BUILD_RPATH "$ORIGIN")
		endif()
	endif()
elseif(SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "AUTO")
	message(STATUS "openmp-ext gRPC integration disabled: headers/proto/library were not found")
elseif(NOT SAMP_CEF_ENABLE_GRPC_EXT_UPPER STREQUAL "OFF")
	message(FATAL_ERROR "SAMP_CEF_ENABLE_GRPC_EXT must be AUTO, ON, or OFF")
endif()

if(UNIX)
	target_compile_definitions(cef PRIVATE LINUX)
	if(NOT APPLE)
		target_compile_options(cef PRIVATE -fno-eliminate-unused-debug-types)
	endif()
endif()

if(APPLE)
	find_library(SECURITY_FRAMEWORK Security REQUIRED)
	find_library(COREFOUNDATION_FRAMEWORK CoreFoundation REQUIRED)
	target_link_libraries(cef PRIVATE
		"${SECURITY_FRAMEWORK}"
		"${COREFOUNDATION_FRAMEWORK}"
	)
endif()

set_property(TARGET cef PROPERTY OUTPUT_NAME cef)
set_property(TARGET cef PROPERTY PREFIX "")

if(NOT SAMP_CEF_INSTALL_COMPONENT_DIR)
	if(OPEN_MP_SERVER_ROOT)
		set(SAMP_CEF_INSTALL_COMPONENT_DIR "${OPEN_MP_SERVER_ROOT}/components")
	else()
		set(SAMP_CEF_INSTALL_COMPONENT_DIR "components")
	endif()
endif()

install(TARGETS cef
	RUNTIME DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
	LIBRARY DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
	ARCHIVE DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}"
)

if(SAMP_CEF_GRPC_EXT_ENABLED AND SAMP_CEF_INSTALL_GRPC_EXT)
	install(FILES "${OPENMP_EXT_GRPC_LIBRARY}" DESTINATION "${SAMP_CEF_INSTALL_COMPONENT_DIR}" COMPONENT "${SAMP_CEF_INSTALL_COMPONENT_NAME}")
endif()
