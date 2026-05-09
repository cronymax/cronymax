# cmake/CronymaxApp.cmake
#
# Builds the cronymax CEF desktop shell (cronymax.app + helper bundles on
# macOS). Included from the top-level CMakeLists.txt when CRONYMAX_BUILD_APP
# is ON.

include(${CMAKE_SOURCE_DIR}/cmake/CefDist.cmake)

list(APPEND CMAKE_MODULE_PATH "${CEF_ROOT}/cmake")
find_package(CEF REQUIRED)

add_subdirectory("${CEF_LIBCEF_DLL_WRAPPER_PATH}"
                 "${CMAKE_BINARY_DIR}/libcef_dll_wrapper")

# ---------------------------------------------------------------------------
# Main app target: app/browser/* → cronymax.app
# ---------------------------------------------------------------------------

set(CRONYMAX_APP_SRCS
  app/browser/app_delegate.cc
  app/browser/app_delegate.h
  app/browser/bridge_handler.cc
  app/browser/bridge_handler.h
  app/browser/client_handler.cc
  app/browser/client_handler.h
  app/browser/desktop_app.cc
  app/browser/desktop_app.h
  app/browser/main_window.cc
  app/browser/main_window.h
  app/browser/profile_store.cc
  app/browser/profile_store.h
  app/browser/space_manager.cc
  app/browser/space_manager.h
  # unified-icons: semantic icon registry + native button factories.
  app/browser/icon_data.h
  app/browser/icon_registry.h
  app/browser/icon_registry.cc
  # arc-style-tab-cards (Phase 1 skeleton)
  app/browser/tab.cc
  app/browser/tab.h
  app/browser/tab_behavior.h
  app/browser/tab_toolbar.cc
  app/browser/tab_toolbar.h
  app/browser/tab_manager.cc
  app/browser/tab_manager.h
  # arc-style-tab-cards (Phase 3+ behaviors)
  app/browser/tab_behaviors/simple_tab_behavior.cc
  app/browser/tab_behaviors/simple_tab_behavior.h
  app/browser/tab_behaviors/web_tab_behavior.cc
  app/browser/tab_behaviors/web_tab_behavior.h
)

if(APPLE)
  list(APPEND CRONYMAX_APP_SRCS
    app/browser/main_mac.mm
    app/browser/mac_view_style.h
    app/browser/mac_view_style.mm
    # workspace-with-profile: native NSOpenPanel folder picker.
    app/browser/mac_folder_picker.h
    app/browser/mac_folder_picker.mm
    # unified-icons: macOS implementation — NSImage rasterisation of embedded SVGs.
    app/browser/icon_registry_mac.mm
  )
else()
  list(APPEND CRONYMAX_APP_SRCS app/browser/main.cc)
endif()

# ---------------------------------------------------------------------------
# Codicons SVG embedding: generate icon_data.cc at build time from the
# vscode-codicons submodule so the binary carries all SVGs as string literals.
# No runtime file I/O and no bundle Resources/icons/ copy needed.
# ---------------------------------------------------------------------------
set(CODICONS_SRC_DIR
    "${CMAKE_CURRENT_SOURCE_DIR}/third_party/vscode-codicons/src/icons")
set(ICON_DATA_CC "${CMAKE_BINARY_DIR}/generated/browser/icon_data.cc")
file(GLOB CODICON_SVG_FILES "${CODICONS_SRC_DIR}/*.svg")

add_custom_command(
  OUTPUT  "${ICON_DATA_CC}"
  COMMAND ${CMAKE_COMMAND}
    -DICONS_DIR=${CODICONS_SRC_DIR}
    -DOUTPUT_FILE=${ICON_DATA_CC}
    -P "${CMAKE_CURRENT_SOURCE_DIR}/cmake/GenerateIconData.cmake"
  DEPENDS ${CODICON_SVG_FILES}
          "${CMAKE_CURRENT_SOURCE_DIR}/cmake/GenerateIconData.cmake"
  COMMENT "Generating icon_data.cc from Codicons SVG sources"
  VERBATIM
)
list(APPEND CRONYMAX_APP_SRCS "${ICON_DATA_CC}")

add_executable(cronymax_app MACOSX_BUNDLE ${CRONYMAX_APP_SRCS})

# profile_store.cc uses yaml-cpp which requires exceptions. Override the
# target-wide -fno-exceptions flag for this single translation unit.
set_source_files_properties(
  app/browser/profile_store.cc
  PROPERTIES COMPILE_FLAGS "-fexceptions"
)

target_include_directories(cronymax_app PRIVATE
  ${CEF_ROOT}
  ${CMAKE_CURRENT_SOURCE_DIR}/app
  ${CMAKE_BINARY_DIR}/generated
)

SET_EXECUTABLE_TARGET_PROPERTIES(cronymax_app)

target_link_libraries(cronymax_app PRIVATE
  cronymax_native
  libcef_dll_wrapper
  ${CEF_STANDARD_LIBS}
  yaml-cpp
)

if(APPLE)
  target_link_libraries(cronymax_app PRIVATE
    "-framework QuartzCore"
  )
endif()

set_target_properties(cronymax_app PROPERTIES
  OUTPUT_NAME "cronymax"
  MACOSX_BUNDLE_GUI_IDENTIFIER  "dev.prototype.cronymax"
  MACOSX_BUNDLE_BUNDLE_NAME     "cronymax"
  MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
)

# ---------------------------------------------------------------------------
# macOS bundle assembly: Info.plist + CEF framework + helper apps.
# ---------------------------------------------------------------------------

if(APPLE)
  set(EXECUTABLE_NAME "cronymax")
  set(PRODUCT_NAME    "cronymax")
  set(VERSION_SHORT   "${PROJECT_VERSION}")
  set_target_properties(cronymax_app PROPERTIES
    MACOSX_BUNDLE_INFO_PLIST
      "${CMAKE_CURRENT_SOURCE_DIR}/app/browser/mac/Info.plist.in"
  )

  COPY_MAC_FRAMEWORK("cronymax_app" "${CEF_BINARY_DIR}"
                     "$<TARGET_BUNDLE_DIR:cronymax_app>")

  set(CRONYMAX_HELPER_SRCS
    app/renderer/main.cc
    app/renderer/app.cc
    app/renderer/app.h
  )
  set(CRONYMAX_HELPER_TARGET      "cronymax_app_helper")
  set(CRONYMAX_HELPER_OUTPUT_NAME "cronymax Helper")

  foreach(_suffix_list ${CEF_HELPER_APP_SUFFIXES})
    string(REPLACE ":" ";" _suffix_list ${_suffix_list})
    list(GET _suffix_list 0 _name_suffix)
    list(GET _suffix_list 1 _target_suffix)
    list(GET _suffix_list 2 _plist_suffix)

    set(_helper_target      "${CRONYMAX_HELPER_TARGET}${_target_suffix}")
    set(_helper_output_name "${CRONYMAX_HELPER_OUTPUT_NAME}${_name_suffix}")
    set(_helper_info_plist  "${CMAKE_BINARY_DIR}/helper-Info${_target_suffix}.plist")

    file(READ "${CMAKE_CURRENT_SOURCE_DIR}/app/browser/mac/helper-Info.plist.in" _plist_contents)
    string(REPLACE "\${EXECUTABLE_NAME}" "${_helper_output_name}" _plist_contents ${_plist_contents})
    string(REPLACE "\${PRODUCT_NAME}"    "${_helper_output_name}" _plist_contents ${_plist_contents})
    string(REPLACE "\${BUNDLE_ID_SUFFIX}" "${_plist_suffix}"      _plist_contents ${_plist_contents})
    string(REPLACE "\${VERSION_SHORT}"   "${PROJECT_VERSION}"     _plist_contents ${_plist_contents})
    file(WRITE ${_helper_info_plist} ${_plist_contents})

    add_executable(${_helper_target} MACOSX_BUNDLE ${CRONYMAX_HELPER_SRCS})
    SET_EXECUTABLE_TARGET_PROPERTIES(${_helper_target})
    target_include_directories(${_helper_target} PRIVATE
      ${CEF_ROOT}
      ${CMAKE_CURRENT_SOURCE_DIR}/app
    )
    target_link_libraries(${_helper_target} PRIVATE
      libcef_dll_wrapper
      ${CEF_STANDARD_LIBS}
      Cronymax::Crony
      nlohmann_json
    )
    if(APPLE)
      target_link_libraries(${_helper_target} PRIVATE
        "-framework CoreFoundation"
        "-framework Security"
        "-framework SystemConfiguration"
      )
    endif()
    set_target_properties(${_helper_target} PROPERTIES
      MACOSX_BUNDLE_INFO_PLIST ${_helper_info_plist}
      OUTPUT_NAME              ${_helper_output_name}
    )

    add_dependencies(cronymax_app ${_helper_target})
    add_custom_command(
      TARGET cronymax_app
      POST_BUILD
      COMMAND ${CMAKE_COMMAND} -E copy_directory
        "$<TARGET_BUNDLE_DIR:${_helper_target}>"
        "$<TARGET_BUNDLE_DIR:cronymax_app>/Contents/Frameworks/${_helper_output_name}.app"
      VERBATIM
    )
  endforeach()
endif()

# ---------------------------------------------------------------------------
# Standalone runtime binary packaging.
#
# `crony` is a separate Rust binary the CEF host spawns
# during startup (see app/runtime/). On macOS it lives next to
# the helper apps under Contents/Frameworks/; on other platforms it
# lives next to the host executable. The binary path is exposed by
# RustRuntime.cmake as ${CRONYMAX_RUNTIME_BINARY}.
# ---------------------------------------------------------------------------
if(CRONYMAX_RUNTIME_BINARY)
  add_dependencies(cronymax_app cronymax_rust)
  if(APPLE)
    set(_cronymax_runtime_dest
      "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Frameworks/crony")
  else()
    set(_cronymax_runtime_dest
      "$<TARGET_FILE_DIR:cronymax_app>/crony${CMAKE_EXECUTABLE_SUFFIX}")
  endif()
  # Use a dedicated custom target (not POST_BUILD on cronymax_app) so the
  # copy runs whenever cronymax_rust is rebuilt — even if no C++ file changed
  # and cronymax_app itself is not re-linked.
  add_custom_target(cronymax_bundle_crony ALL
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
      "${CRONYMAX_RUNTIME_BINARY}"
      "${_cronymax_runtime_dest}"
    DEPENDS cronymax_rust
    COMMENT "Bundling crony binary"
    VERBATIM
  )
  add_dependencies(cronymax_app cronymax_bundle_crony)
endif()

# ---------------------------------------------------------------------------
# Web resources: build with Vite (gated by CRONYMAX_BUILD_WEB) then copy
# web/dist/ into the bundle so the CEF shell can load it from file://.
# ---------------------------------------------------------------------------

if(CRONYMAX_BUILD_WEB)
  find_program(BUN_EXECUTABLE bun)
  if(NOT BUN_EXECUTABLE)
    message(FATAL_ERROR
      "bun not found on PATH. Install bun (https://bun.sh/docs/installation) or "
      "configure with -DCRONYMAX_BUILD_WEB=OFF to skip the frontend build.")
  endif()

  # Always-run target: bun install + bun run build in web/. The output is the
  # entire web/dist/ tree, which is non-trivial to express as BYPRODUCTS, so
  # we use a phony stamp file and force re-run on every cronymax_app build.
  add_custom_target(cronymax_web ALL
    COMMAND ${BUN_EXECUTABLE} install --frozen-lockfile
    COMMAND ${BUN_EXECUTABLE} run build
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/web"
    COMMENT "Building cronymax web frontend (bun + Vite)"
    VERBATIM
  )
  add_dependencies(cronymax_app cronymax_web)

  # Optional CI gate: typecheck + lint. Not in ALL — opt in via
  # `cmake --build build --target cronymax_web_check`.
  add_custom_target(cronymax_web_check
    COMMAND ${BUN_EXECUTABLE} run typecheck
    COMMAND ${BUN_EXECUTABLE} run lint
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/web"
    COMMENT "Running cronymax web typecheck + lint"
    VERBATIM
  )
endif()

# App icon: copy assets/installer/AppIcon.icns into the bundle Resources/
# so macOS can display it in the Dock, Finder, and window decorations.
add_custom_command(TARGET cronymax_app POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${CMAKE_CURRENT_SOURCE_DIR}/assets/installer/AppIcon.icns"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/AppIcon.icns"
  COMMENT "Bundling AppIcon.icns into cronymax.app"
  VERBATIM
)

add_custom_command(TARGET cronymax_app POST_BUILD
  # All panels are now React; copy only the built dist/. Remove the
  # destination first so stale, content-hashed asset files from prior
  # builds don't accumulate alongside the current ones.
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/web"
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CMAKE_CURRENT_SOURCE_DIR}/web/dist"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/web"
)

# Built-in document type schemas. Copied into the bundle so the document
# subsystem can locate them at Resources/builtin-doc-types/. The
# DocTypeRegistry first loads built-ins from here, then merges per-workspace
# overrides from <workspace>/.cronymax/doc-types/.
add_custom_command(TARGET cronymax_app POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-doc-types"
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CMAKE_CURRENT_SOURCE_DIR}/.cronymax/doc-types"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-doc-types"
  COMMENT "Bundling built-in doc-type schemas into cronymax.app"
  VERBATIM
)

# Built-in agent definitions. Copied alongside the doc-type schemas so the
# AgentRegistry can merge them with workspace overrides under
# <workspace>/.cronymax/agents/.
add_custom_command(TARGET cronymax_app POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-agents"
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CMAKE_CURRENT_SOURCE_DIR}/.cronymax/agents"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-agents"
  COMMENT "Bundling built-in agent definitions into cronymax.app"
  VERBATIM
)

# Built-in (preset) flow definitions. Copied into the bundle so the flow
# registry can surface them in the Settings / Flows dropdown even before the
# user creates a workspace-local copy. Workspace-local flows with the same
# id take priority over these bundled presets.
add_custom_command(TARGET cronymax_app POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-flows"
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CMAKE_CURRENT_SOURCE_DIR}/.cronymax/flows"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-flows"
  COMMENT "Bundling built-in preset flows into cronymax.app"
  VERBATIM
)

# Always-run sync of the freshly built web/dist/ into the bundle. cronymax_app
# may not relink on every build (e.g. when only frontend files change), so its
# POST_BUILD wouldn't fire. This phony target runs unconditionally.
if(CRONYMAX_BUILD_WEB)
  add_custom_target(cronymax_web_sync ALL
    COMMAND ${CMAKE_COMMAND} -E rm -rf
      "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/web"
    COMMAND ${CMAKE_COMMAND} -E copy_directory
      "${CMAKE_CURRENT_SOURCE_DIR}/web/dist"
      "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/web"
    COMMENT "Syncing web/dist/ into cronymax.app bundle"
    VERBATIM
  )
  add_dependencies(cronymax_web_sync cronymax_app cronymax_web)
endif()
