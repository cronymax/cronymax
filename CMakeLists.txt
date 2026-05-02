cmake_minimum_required(VERSION 3.21)

project(cronymax
  VERSION 0.1.0
  LANGUAGES C CXX
)

if(APPLE)
  enable_language(OBJCXX)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(CRONYMAX_BUILD_TOOLS "Build local native runtime tools" ON)
option(CRONYMAX_BUILD_APP   "Build the cronymax CEF desktop shell" OFF)
option(CRONYMAX_BUILD_WEB   "Run pnpm build in web/ before bundling" ON)

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
# yaml-cpp (static) via FetchContent — used by app/flow and app/document for
# parsing flow.yaml, agent.yaml, and doc-type schemas. Round-trip writing is
# not required in this change.
# ---------------------------------------------------------------------------
include(FetchContent)
set(YAML_CPP_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(YAML_CPP_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(YAML_CPP_BUILD_CONTRIB OFF CACHE BOOL "" FORCE)
set(YAML_CPP_INSTALL OFF CACHE BOOL "" FORCE)
set(YAML_BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
# yaml-cpp 0.8.0 declares cmake_minimum_required < 3.5, which CMake 4.x no
# longer supports. Force a compatible policy version when descending into the
# fetched subdirectory.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5 CACHE STRING "" FORCE)
FetchContent_Declare(
  yaml-cpp
  GIT_REPOSITORY https://github.com/jbeder/yaml-cpp.git
  GIT_TAG        0.8.0
  GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(yaml-cpp)

# ---------------------------------------------------------------------------
# Module libraries (flat layout — one static lib per app/<module>).
# ---------------------------------------------------------------------------

# common: shared types and path utilities used by every other module.
add_library(cronymax_common STATIC
  app/common/json_value.cc
  app/common/path_utils.cc
  app/common/uuid_v7.cc
)
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

# sandbox: command classifier + permission/policy + sandbox launcher.
add_library(cronymax_sandbox STATIC
  app/sandbox/command_classifier.cc
  app/sandbox/permission_broker.cc
  app/sandbox/sandbox_launcher.cc
  app/sandbox/sandbox_policy.cc
)
target_link_libraries(cronymax_sandbox PUBLIC cronymax_common)

# workspace: file broker + space store (depends on sandbox).
add_library(cronymax_workspace STATIC
  app/workspace/file_broker.cc
  app/workspace/space_store.cc
)
target_link_libraries(cronymax_workspace
  PUBLIC cronymax_sandbox SQLite3::SQLite3
)

# event_bus: typed event store (SQLite + JSONL) + inbox triage. Sits on top
# of cronymax_workspace because it borrows the SpaceStore's sqlite3 handle.
add_library(cronymax_event_bus STATIC
  app/event_bus/app_event.cc
  app/event_bus/event_bus.cc
)
target_link_libraries(cronymax_event_bus
  PUBLIC cronymax_common cronymax_workspace SQLite3::SQLite3
)
target_include_directories(cronymax_event_bus PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# terminal: PTY session implementation (macOS only for now).
add_library(cronymax_terminal STATIC)
target_link_libraries(cronymax_terminal PUBLIC cronymax_common Threads::Threads)
if(APPLE)
  target_sources(cronymax_terminal PRIVATE app/terminal/pty_session_mac.cc)
else()
  message(WARNING "PTY implementation is currently macOS-only for the prototype.")
endif()

# agent: planner + tool registry + LLM router (depends on sandbox + workspace).
add_library(cronymax_agent STATIC
  app/agent/agent_runtime.cc
  app/agent/graph_engine.cc
  app/agent/model_router.cc
  app/agent/tool_registry.cc
)
target_link_libraries(cronymax_agent
  PUBLIC cronymax_sandbox cronymax_workspace cronymax_common cronymax_document
)

# document: doc-type schemas, agent definitions, document store, reviews,
# reviewer pipeline. Hand-edited YAML in the workspace's .cronymax/ tree is
# the source of truth.
add_library(cronymax_document STATIC
  app/document/agent_definition.cc
  app/document/agent_registry.cc
  app/document/doc_type_registry.cc
  app/document/doc_type_schema.cc
  app/document/document_store.cc
  app/document/review_store.cc
  app/document/reviewer_pipeline.cc
  app/document/reviews_state.cc
  app/document/schema_reviewer.cc
)
target_link_libraries(cronymax_document
  PUBLIC cronymax_common cronymax_workspace yaml-cpp
)
target_include_directories(cronymax_document PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# flow: workspace layout helpers, flow/agent registries, FlowRuntime, router,
# mention parser, trace writer. Layers on top of cronymax_agent and
# cronymax_document.
add_library(cronymax_flow STATIC
  app/flow/flow_definition.cc
  app/flow/flow_registry.cc
  app/flow/flow_runtime.cc
  app/flow/fs_watcher.cc
  app/flow/gitignore_helper.cc
  app/flow/mention_parser.cc
  app/flow/router.cc
  app/flow/trace_event.cc
  app/flow/trace_writer.cc
  app/flow/workspace_layout.cc
)
target_link_libraries(cronymax_flow
  PUBLIC cronymax_common cronymax_workspace cronymax_agent cronymax_document cronymax_event_bus yaml-cpp
  PRIVATE "-framework CoreServices"
)
target_include_directories(cronymax_flow PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# Convenience aggregate library that pulls in every native module.
add_library(cronymax_native INTERFACE)
target_link_libraries(cronymax_native INTERFACE
  cronymax_common
  cronymax_sandbox
  cronymax_workspace
  cronymax_terminal
  cronymax_agent
  cronymax_document
  cronymax_flow
  cronymax_event_bus
)

# space_manager: CEF-free Space lifecycle library so native_probe can
# exercise the same SpaceManager/SpaceStore code paths the CEF app uses,
# without dragging in CEF or the rest of app/browser/.
add_library(cronymax_space STATIC
  app/browser/space_manager.cc
)
target_link_libraries(cronymax_space
  PUBLIC cronymax_workspace cronymax_agent cronymax_terminal cronymax_document cronymax_flow cronymax_event_bus
)
target_include_directories(cronymax_space PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/app)

# ---------------------------------------------------------------------------
# Tools and CEF desktop shell.
# ---------------------------------------------------------------------------

if(CRONYMAX_BUILD_TOOLS)
  add_executable(native_probe tools/native_probe.cc)
  target_link_libraries(native_probe PRIVATE cronymax_native cronymax_space)

  # Loader tests for cronymax_document + cronymax_flow YAML parsers.
  # Run with: ./build/loader_test
  add_executable(loader_test tools/loader_test.cc)
  target_link_libraries(loader_test PRIVATE cronymax_agent cronymax_document cronymax_flow)

  # rebuild_trace --- admin CLI to regenerate trace.jsonl from SQLite.
  add_executable(rebuild_trace tools/rebuild_trace.cc)
  target_link_libraries(rebuild_trace PRIVATE cronymax_event_bus cronymax_workspace)

  # assign_block_ids --- one-shot migration CLI for the WYSIWYG workbench.
  # Walks every <workspace>/.cronymax/flows/*/docs/*.md and prepends a
  # `<!-- block: <uuid> -->` marker to any top-level block lacking one.
  add_executable(assign_block_ids tools/assign_block_ids.cc)
  target_link_libraries(assign_block_ids PRIVATE cronymax_common)

  # migrate_review_anchors --- one-shot migration CLI that rewrites the
  # legacy line-range comment anchors into the new block-id form by
  # invoking ReviewStore::MigrateAnchors for every reviews.json file.
  add_executable(migrate_review_anchors tools/migrate_review_anchors.cc)
  target_link_libraries(migrate_review_anchors PRIVATE cronymax_document)
endif()

if(CRONYMAX_BUILD_APP)
  include(cmake/CronymaxApp.cmake)
endif()
