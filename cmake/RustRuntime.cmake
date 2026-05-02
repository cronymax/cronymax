# cmake/RustRuntime.cmake
#
# Drives `cargo build` for the Rust workspace at the repo root and
# exposes the resulting `crony` static library (plus its bundled C
# header) as a CMake IMPORTED target named `Cronymax::Crony`.
#
# Authored as part of the `rust-runtime-migration` change (task 1.1 /
# 1.3). The integration is intentionally narrow:
#
#   * We only build `-p crony` so the workspace can grow other crates
#     without dragging extra link-time dependencies into the CEF host.
#   * Profile mirrors CMake's `CMAKE_BUILD_TYPE`. Defaults to release.
#   * No automatic source globbing — Cargo handles incremental builds
#     itself, and we let CMake re-invoke `cargo build` every configure
#     step which is cheap when nothing has changed.

include_guard(GLOBAL)

option(CRONYMAX_BUILD_RUST "Build the Rust runtime workspace via cargo" ON)

if(NOT CRONYMAX_BUILD_RUST)
  message(STATUS "Cronymax: Rust runtime build disabled (CRONYMAX_BUILD_RUST=OFF)")
  return()
endif()

find_program(CARGO_EXECUTABLE cargo)
if(NOT CARGO_EXECUTABLE)
  message(FATAL_ERROR
    "cargo not found on PATH. Install rustup (https://rustup.rs) or set "
    "-DCRONYMAX_BUILD_RUST=OFF to skip the Rust runtime build."
  )
endif()

# Map CMAKE_BUILD_TYPE -> cargo profile.
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
  set(_cronymax_cargo_profile "dev")
  set(_cronymax_cargo_target_dir "debug")
else()
  set(_cronymax_cargo_profile "release")
  set(_cronymax_cargo_target_dir "release")
endif()

set(_cronymax_rust_root      "${CMAKE_SOURCE_DIR}")
set(_cronymax_cargo_target   "${CMAKE_BINARY_DIR}/rust-target")
set(_cronymax_crony_lib_name "${CMAKE_STATIC_LIBRARY_PREFIX}crony${CMAKE_STATIC_LIBRARY_SUFFIX}")
set(_cronymax_crony_lib_path "${_cronymax_cargo_target}/${_cronymax_cargo_target_dir}/${_cronymax_crony_lib_name}")
set(_cronymax_runtime_bin    "${_cronymax_cargo_target}/${_cronymax_cargo_target_dir}/crony${CMAKE_EXECUTABLE_SUFFIX}")

include(ExternalProject)

set(_cronymax_cargo_args
  build
  --manifest-path "${_cronymax_rust_root}/Cargo.toml"
  --target-dir    "${_cronymax_cargo_target}"
  -p crony
  --lib
  --bin crony
)
if(_cronymax_cargo_profile STREQUAL "release")
  list(APPEND _cronymax_cargo_args --release)
endif()

ExternalProject_Add(cronymax_rust
  PREFIX            "${CMAKE_BINARY_DIR}/rust-build"
  SOURCE_DIR        "${_cronymax_rust_root}"
  CONFIGURE_COMMAND ""
  BUILD_COMMAND     "${CARGO_EXECUTABLE}" ${_cronymax_cargo_args}
  INSTALL_COMMAND   ""
  BUILD_ALWAYS      TRUE
  USES_TERMINAL_BUILD TRUE
  BUILD_BYPRODUCTS
    "${_cronymax_crony_lib_path}"
    "${_cronymax_runtime_bin}"
)

# Imported library target consumers link against.
add_library(Cronymax::Crony STATIC IMPORTED GLOBAL)
set_target_properties(Cronymax::Crony PROPERTIES
  IMPORTED_LOCATION "${_cronymax_crony_lib_path}"
)
add_dependencies(Cronymax::Crony cronymax_rust)

# Public C header consumers `#include "crony.h"`. Lives next to the
# Rust source so the header and ffi.rs stay in lock step. Surfaced
# via an INTERFACE include directory so any target linking
# Cronymax::Crony picks it up automatically.
set(_cronymax_crony_include "${_cronymax_rust_root}/crony/include")
file(MAKE_DIRECTORY "${_cronymax_crony_include}")
set_property(TARGET Cronymax::Crony APPEND PROPERTY
  INTERFACE_INCLUDE_DIRECTORIES "${_cronymax_crony_include}"
)

# System libraries the Rust standard library / tokio / tracing pull in.
if(APPLE)
  set_property(TARGET Cronymax::Crony APPEND PROPERTY
    INTERFACE_LINK_LIBRARIES
      "-framework CoreFoundation"
      "-framework Security"
      "-framework SystemConfiguration"
  )
elseif(UNIX)
  set_property(TARGET Cronymax::Crony APPEND PROPERTY
    INTERFACE_LINK_LIBRARIES dl pthread m
  )
endif()

# Surface paths for downstream packaging steps (e.g. copying the
# standalone runtime binary into the .app bundle).
set(CRONYMAX_RUNTIME_BINARY "${_cronymax_runtime_bin}" CACHE INTERNAL "")
set(CRONYMAX_CRONY_LIBRARY  "${_cronymax_crony_lib_path}" CACHE INTERNAL "")

message(STATUS "Cronymax: Rust runtime build wired (profile=${_cronymax_cargo_profile})")
