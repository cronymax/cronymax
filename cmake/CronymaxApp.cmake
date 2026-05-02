# cmake/CronymaxApp.cmake
#
# Builds the cronymax CEF desktop shell (cronymax.app + helper bundles on
# macOS). Included from the top-level CMakeLists.txt when CRONYMAX_BUILD_APP
# is ON.

# CEF distribution layout. Two modes:
#
#   1. (default) Build CEF_ROOT from the in-tree cef/ submodule plus a
#      downloaded prebuilt framework. Set CRONYMAX_CEF_DIST_URL to a CEF
#      binary archive matching the submodule commit; the framework is staged
#      under ${CMAKE_BINARY_DIR}/cef-staging/.
#
#   2. (override) Pass -DCEF_ROOT=/path/to/cef_binary_* to point at an
#      already-extracted upstream binary distribution. The submodule is
#      ignored in that case.
if(NOT CEF_ROOT OR NOT EXISTS "${CEF_ROOT}/cmake/FindCEF.cmake")
  include(${CMAKE_SOURCE_DIR}/cmake/CefSubmodule.cmake)
endif()

if(NOT EXISTS "${CEF_ROOT}/cmake/FindCEF.cmake")
  message(FATAL_ERROR
    "CEF_ROOT does not look like a CEF binary distribution: ${CEF_ROOT}\n"
    "Either set CRONYMAX_CEF_DIST_URL (uses the cef/ submodule) or pass "
    "-DCEF_ROOT=/path/to/cef_binary_*.")
endif()

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
  app/browser/space_manager.cc
  app/browser/space_manager.h
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
  )
else()
  list(APPEND CRONYMAX_APP_SRCS app/browser/main.cc)
endif()

add_executable(cronymax_app MACOSX_BUNDLE ${CRONYMAX_APP_SRCS})

target_include_directories(cronymax_app PRIVATE
  ${CEF_ROOT}
  ${CMAKE_CURRENT_SOURCE_DIR}/app
)

SET_EXECUTABLE_TARGET_PROPERTIES(cronymax_app)

target_link_libraries(cronymax_app PRIVATE
  cronymax_native
  libcef_dll_wrapper
  ${CEF_STANDARD_LIBS}
)

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
    app/browser/process_helper_mac.cc
    app/browser/render_app.cc
    app/browser/render_app.h
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
    )
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
# Web resources: build with Vite (gated by CRONYMAX_BUILD_WEB) then copy
# web/dist/ into the bundle so the CEF shell can load it from file://.
# ---------------------------------------------------------------------------

if(CRONYMAX_BUILD_WEB)
  find_program(PNPM_EXECUTABLE pnpm)
  if(NOT PNPM_EXECUTABLE)
    message(FATAL_ERROR
      "pnpm not found on PATH. Install pnpm (https://pnpm.io/installation) or "
      "configure with -DCRONYMAX_BUILD_WEB=OFF to skip the frontend build.")
  endif()

  # Always-run target: pnpm install + pnpm build in web/. The output is the
  # entire web/dist/ tree, which is non-trivial to express as BYPRODUCTS, so
  # we use a phony stamp file and force re-run on every cronymax_app build.
  add_custom_target(cronymax_web ALL
    COMMAND ${PNPM_EXECUTABLE} install --frozen-lockfile
    COMMAND ${PNPM_EXECUTABLE} build
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/web"
    COMMENT "Building cronymax web frontend (pnpm + Vite)"
    VERBATIM
  )
  add_dependencies(cronymax_app cronymax_web)

  # Optional CI gate: typecheck + lint. Not in ALL — opt in via
  # `cmake --build build --target cronymax_web_check`.
  add_custom_target(cronymax_web_check
    COMMAND ${PNPM_EXECUTABLE} typecheck
    COMMAND ${PNPM_EXECUTABLE} lint
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/web"
    COMMENT "Running cronymax web typecheck + lint"
    VERBATIM
  )
endif()

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
    "${CMAKE_CURRENT_SOURCE_DIR}/assets/builtin-doc-types"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-doc-types"
  COMMENT "Bundling built-in doc-type schemas into cronymax.app"
  VERBATIM
)

# Built-in agent definitions (e.g. the `critic` reviewer). Copied alongside
# the doc-type schemas so the AgentRegistry can merge them with workspace
# overrides under <workspace>/.cronymax/agents/.
add_custom_command(TARGET cronymax_app POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-agents"
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CMAKE_CURRENT_SOURCE_DIR}/assets/builtin-agents"
    "$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/builtin-agents"
  COMMENT "Bundling built-in agent definitions into cronymax.app"
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
