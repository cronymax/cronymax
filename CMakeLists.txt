cmake_minimum_required(VERSION 3.21)

project(cronymax
  VERSION 0.1.0
  LANGUAGES C CXX
)

# Allow CI to inject the release version from the git tag without modifying
# this file. Pass -DCRONYMAX_VERSION=0.2.0 to cmake to override.
if(DEFINED CRONYMAX_VERSION)
  set(PROJECT_VERSION "${CRONYMAX_VERSION}")
endif()

if(APPLE)
  enable_language(OBJCXX)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(CRONYMAX_BUILD_TOOLS "Build local native runtime tools" ON)
option(CRONYMAX_BUILD_APP   "Build the cronymax CEF desktop shell" OFF)
option(CRONYMAX_BUILD_WEB   "Run bun build in web/ before bundling" ON)

find_package(Threads REQUIRED)
find_package(SQLite3 REQUIRED)

# SQLite3::SQLite3 IMPORTED target may not exist in older CMake + Xcode SDK
# combinations. Fall back to variables when the target is not available.
if(NOT TARGET SQLite3::SQLite3)
  add_library(SQLite3::SQLite3 UNKNOWN IMPORTED)
  set_target_properties(SQLite3::SQLite3 PROPERTIES
    IMPORTED_LOCATION "${SQLite3_LIBRARIES}"
    INTERFACE_INCLUDE_DIRECTORIES "${SQLite3_INCLUDE_DIRS}"
  )
endif()

# ---------------------------------------------------------------------------
# yaml-cpp — git submodule at third_party/yaml-cpp.
# ---------------------------------------------------------------------------
set(YAML_CPP_BUILD_TESTS   OFF CACHE BOOL "" FORCE)
set(YAML_CPP_BUILD_TOOLS   OFF CACHE BOOL "" FORCE)
set(YAML_CPP_BUILD_CONTRIB OFF CACHE BOOL "" FORCE)
set(YAML_CPP_INSTALL       OFF CACHE BOOL "" FORCE)
set(YAML_BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
add_subdirectory(third_party/yaml-cpp)

# ---------------------------------------------------------------------------
# nlohmann/json — git submodule at third_party/nlohmann/json.
# ---------------------------------------------------------------------------
set(JSON_BuildTests OFF CACHE BOOL "" FORCE)
set(JSON_Install    OFF CACHE BOOL "" FORCE)
add_subdirectory(third_party/nlohmann/json)

# ---------------------------------------------------------------------------
# Rust runtime workspace (crony / cronymax / cronygraph).
# Defines the IMPORTED target `Cronymax::Crony` for downstream linking.
# Skip with -DCRONYMAX_BUILD_RUST=OFF.
# ---------------------------------------------------------------------------
# Set the macOS deployment target before include(RustRuntime) so the value
# is available when ExternalProject_Add captures the cargo BUILD_COMMAND.
# 12.0 matches CEF_TARGET_SDK in cef/cmake/cef_variables.cmake.in.
if(APPLE AND NOT CMAKE_OSX_DEPLOYMENT_TARGET)
  set(CMAKE_OSX_DEPLOYMENT_TARGET "12.0" CACHE STRING
    "Minimum macOS version to target. Must match CEF_TARGET_SDK." FORCE)
endif()
list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")
include(RustRuntime)

# ---------------------------------------------------------------------------
# Module libraries (flat layout — one static lib per app/<module>).
# ---------------------------------------------------------------------------

# common: shared types and path utilities used by every other module.
add_library(cronymax_common STATIC
  app/common/path_utils.cc
  app/common/uuid_v7.cc
)
target_link_libraries(cronymax_common PUBLIC nlohmann_json)
target_include_directories(cronymax_common PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# macOS notification + dock-badge shim. On Apple, build the Objective-C++
# implementation and link UserNotifications + AppKit. On other platforms,
# build the no-op stub so cronymax_common always exposes the same symbols.
if(APPLE)
  target_sources(cronymax_common PRIVATE app/platform/macos/notifications.mm)
  set_source_files_properties(
    app/platform/macos/notifications.mm
    PROPERTIES
      COMPILE_FLAGS "-fobjc-arc"
      LANGUAGE OBJCXX
  )
  find_library(APPKIT_FRAMEWORK AppKit REQUIRED)
  find_library(USERNOTIFICATIONS_FRAMEWORK UserNotifications REQUIRED)
  find_library(FOUNDATION_FRAMEWORK Foundation REQUIRED)
  target_link_libraries(cronymax_common PUBLIC
    ${APPKIT_FRAMEWORK}
    ${USERNOTIFICATIONS_FRAMEWORK}
    ${FOUNDATION_FRAMEWORK}
  )
else()
  target_sources(cronymax_common PRIVATE app/platform/macos/notifications_stub.cc)
endif()

# event_bus: typed event store (SQLite + JSONL) + inbox triage. 
add_library(cronymax_event_bus STATIC
  app/event_bus/app_event.cc
  app/event_bus/event_bus.cc
)
target_link_libraries(cronymax_event_bus
  PUBLIC cronymax_common SQLite3::SQLite3 nlohmann_json
)
target_include_directories(cronymax_event_bus PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# terminal: PTY session implementation moved to Rust runtime (Phase 4).
# Kept as an INTERFACE library so existing link targets don't need to change.
add_library(cronymax_terminal INTERFACE)
target_link_libraries(cronymax_terminal INTERFACE cronymax_common Threads::Threads)

# runtime_bridge: C++ side of the Rust runtime GIPS client.
# Depends on Cronymax::Crony (static lib + crony.h header) for the C ABI.
# Only available when CRONYMAX_BUILD_RUST is ON (the default).
if(CRONYMAX_BUILD_RUST)
  add_library(cronymax_runtime STATIC
    app/runtime/crony_bridge.cc
    app/runtime/crony_proxy.cc
    app/runtime/legacy_importer.cc
    app/runtime/layout_migrator.cc
    app/runtime/profile_store.cc
    app/runtime/space_store.cc
    app/runtime/space_manager.cc
    app/runtime/workspace_id.cc
  )
  target_link_libraries(cronymax_runtime
    PUBLIC cronymax_common Cronymax::Crony nlohmann_json z iconv yaml-cpp
  )
  target_include_directories(cronymax_runtime
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app
  )
else()
  # Stub so the build doesn't fail when Rust is disabled.
  add_library(cronymax_runtime INTERFACE)
endif()

# Convenience aggregate library that pulls in every native module.
add_library(cronymax_native INTERFACE)
target_link_libraries(cronymax_native INTERFACE
  cronymax_common
  cronymax_terminal
  cronymax_event_bus
  cronymax_runtime
)

# ---------------------------------------------------------------------------
# Tools and CEF desktop shell.
# ---------------------------------------------------------------------------

if(CRONYMAX_BUILD_TOOLS)
  add_executable(native_probe tools/native_probe.cc)
  target_link_libraries(native_probe PRIVATE cronymax_native)

  # rebuild_trace --- admin CLI to regenerate trace.jsonl from SQLite.
  add_executable(rebuild_trace tools/rebuild_trace.cc)
  target_link_libraries(rebuild_trace PRIVATE cronymax_event_bus)

  # assign_block_ids --- one-shot migration CLI for the WYSIWYG workbench.
  # Walks every <workspace>/.cronymax/flows/*/docs/*.md and prepends a
  # `<!-- block: <uuid> -->` marker to any top-level block lacking one.
  add_executable(assign_block_ids tools/assign_block_ids.cc)
  target_link_libraries(assign_block_ids PRIVATE cronymax_common)

  # migrate_review_anchors --- one-shot migration CLI that rewrites the
  # legacy line-range comment anchors into the new block-id form by
  # invoking ReviewStore::MigrateAnchors for every reviews.json file.
  add_executable(migrate_review_anchors tools/migrate_review_anchors.cc)
  target_link_libraries(migrate_review_anchors PRIVATE cronymax_common)
endif()

if(CRONYMAX_BUILD_APP)
  include(cmake/CronymaxApp.cmake)
endif()
