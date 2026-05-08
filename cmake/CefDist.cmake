# cmake/CefDist.cmake
#
# Downloads and extracts the CEF binary distribution into
# ${CMAKE_BINARY_DIR}/cef-staging/ and sets CEF_ROOT to that path.
#
# CEF dist info is loaded in priority order:
#   1. Environment variables: CEF_ARM64_URL / CEF_ARM64_SHA256
#      (or CEF_X86_64_URL / CEF_X86_64_SHA256 on x86_64)
#   2. cmake/cef-version.env (fallback)
#
# The archive is cached in ${CMAKE_BINARY_DIR}/.cef-cache/.

# ---- Select arch and read CEF dist info (ENV vars take priority, fallback to cef-version.env)
if(CMAKE_SYSTEM_PROCESSOR MATCHES "arm64|aarch64")
  set(_arch "ARM64")
else()
  set(_arch "X86_64")
endif()

if(DEFINED ENV{CEF_${_arch}_URL})
  set(_cef_url "$ENV{CEF_${_arch}_URL}")
  set(_cef_sha "$ENV{CEF_${_arch}_SHA256}")
  message(STATUS "CEF dist info loaded from environment variables.")
else()
  set(_env_file "${CMAKE_SOURCE_DIR}/cmake/cef-version.env")
  if(NOT EXISTS "${_env_file}")
    message(FATAL_ERROR "cmake/cef-version.env not found and CEF_${_arch}_URL env var is not set.")
  endif()
  file(STRINGS "${_env_file}" _env_lines REGEX "^CEF_${_arch}_(URL|SHA256)=")
  foreach(_line IN LISTS _env_lines)
    if(_line MATCHES "^CEF_${_arch}_URL=(.+)$")
      set(_cef_url "${CMAKE_MATCH_1}")
    elseif(_line MATCHES "^CEF_${_arch}_SHA256=(.+)$")
      set(_cef_sha "${CMAKE_MATCH_1}")
    endif()
  endforeach()
  message(STATUS "CEF dist info loaded from cmake/cef-version.env.")
endif()

if(NOT _cef_url)
  message(FATAL_ERROR "No CEF URL found for arch ${_arch}.")
endif()
message(STATUS "CEF URL: ${_cef_url}")

# ---- Download (cached in build/.cef-cache/) --------------------------------
set(_cache_dir "${CMAKE_BINARY_DIR}/.cef-cache")
file(MAKE_DIRECTORY "${_cache_dir}")

string(SHA1 _url_hash "${_cef_url}")
set(_archive "${_cache_dir}/cef-${_url_hash}.tar.bz2")

if(NOT EXISTS "${_archive}")
  message(STATUS "Downloading CEF binary distribution...")
  set(_dl_extra)
  if(_cef_sha)
    list(APPEND _dl_extra EXPECTED_HASH SHA256=${_cef_sha})
  endif()
  file(DOWNLOAD "${_cef_url}" "${_archive}" SHOW_PROGRESS STATUS _status ${_dl_extra})
  list(GET _status 0 _code)
  if(NOT _code EQUAL 0)
    list(GET _status 1 _msg)
    file(REMOVE "${_archive}")
    message(FATAL_ERROR "CEF download failed: ${_msg}")
  endif()
else()
  message(STATUS "CEF archive cached: ${_archive}")
endif()

# ---- Extract (skipped if stamp matches) ------------------------------------
set(_stage "${CMAKE_BINARY_DIR}/cef-staging")
set(_stamp "${_stage}/.cef-stamp")

set(_need_extract TRUE)
if(EXISTS "${_stamp}")
  file(READ "${_stamp}" _stamped_url)
  if(_stamped_url STREQUAL _cef_url)
    set(_need_extract FALSE)
  endif()
endif()

if(_need_extract)
  set(_tmp "${CMAKE_BINARY_DIR}/.cef-extract-tmp")
  file(REMOVE_RECURSE "${_tmp}" "${_stage}")
  file(MAKE_DIRECTORY "${_tmp}")
  message(STATUS "Extracting CEF binary distribution...")
  file(ARCHIVE_EXTRACT INPUT "${_archive}" DESTINATION "${_tmp}")
  file(GLOB _top "${_tmp}/cef_binary_*")
  list(GET _top 0 _top_dir)
  file(RENAME "${_top_dir}" "${_stage}")
  file(REMOVE_RECURSE "${_tmp}")
  file(WRITE "${_stamp}" "${_cef_url}")
else()
  message(STATUS "CEF already staged at ${_stage}")
endif()

set(CEF_ROOT "${_stage}" CACHE PATH "Staged CEF binary distribution" FORCE)
message(STATUS "CEF staged at ${CEF_ROOT}")
