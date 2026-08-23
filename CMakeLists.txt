cmake_minimum_required(VERSION 3.28...3.30)

include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/common/bootstrap.cmake" NO_POLICY_SCOPE)

project(${_name} VERSION ${_version})

option(ENABLE_FRONTEND_API "Use obs-frontend-api for UI functionality" OFF)
option(ENABLE_QT "Use Qt functionality" OFF)

include(compilerconfig)
include(defaults)
include(helpers)

add_library(${CMAKE_PROJECT_NAME} MODULE)

find_package(libobs REQUIRED)
target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE OBS::libobs)

if(ENABLE_FRONTEND_API)
  find_package(obs-frontend-api REQUIRED)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE OBS::obs-frontend-api)
endif()

if(ENABLE_QT)
  find_package(Qt6 COMPONENTS Widgets Core)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE Qt6::Core Qt6::Widgets)
  target_compile_options(
    ${CMAKE_PROJECT_NAME}
    PRIVATE $<$<C_COMPILER_ID:Clang,AppleClang>:-Wno-quoted-include-in-framework-header -Wno-comma>
  )
  set_target_properties(
    ${CMAKE_PROJECT_NAME}
    PROPERTIES AUTOMOC ON AUTOUIC ON AUTORCC ON
  )
endif()

# FFmpeg decodes the H.264 and HEVC the phone sends. obs-deps ships no pkg-config
# or CMake package for it, so it is searched for by hand there. The libraries are
# the ones OBS itself loads at runtime, so the plugin must be built against the
# same OBS line it runs on.
if(OS_LINUX)
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(FFMPEG REQUIRED IMPORTED_TARGET libavcodec libavutil)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE PkgConfig::FFMPEG)
else()
  find_path(FFMPEG_INCLUDE_DIR libavcodec/avcodec.h REQUIRED)
  find_library(AVCODEC_LIBRARY avcodec REQUIRED)
  find_library(AVUTIL_LIBRARY avutil REQUIRED)
  target_include_directories(${CMAKE_PROJECT_NAME} PRIVATE ${FFMPEG_INCLUDE_DIR})
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE ${AVCODEC_LIBRARY} ${AVUTIL_LIBRARY})
endif()

# MobCam's own code is Rust, built by cargo into a static library that is linked
# into the module below. Everything around it - bundling, signing and packaging -
# is left to set_target_properties_plugin(), so the CI scripts and the installers
# are unaffected by any of this.

# The rustup managed cargo is preferred over any other on PATH: it is the one
# that honours rust-toolchain.toml, and the only one that can build the second
# architecture of the macOS universal binary.
find_program(
  CARGO_EXECUTABLE
  cargo
  REQUIRED
  HINTS
    "$ENV{CARGO_HOME}/bin"
    "$ENV{HOME}/.cargo/bin"
    "$ENV{USERPROFILE}/.cargo/bin"
    "$ENV{HOMEBREW_PREFIX}/opt/rustup/bin"
    /opt/homebrew/opt/rustup/bin
    /usr/local/opt/rustup/bin
  NO_SYSTEM_ENVIRONMENT_PATH
)

# A cargo that is not rustup managed cannot add the second architecture's
# standard library, so fall back to PATH only if none of the above matched.
if(NOT CARGO_EXECUTABLE)
  find_program(CARGO_EXECUTABLE cargo REQUIRED)
endif()

# The cargo shim runs a bare "rustc" from PATH, so a different rust installation
# earlier on PATH would quietly supply the compiler - and with it a toolchain
# that has no second architecture installed. Put cargo's own directory first.
cmake_path(GET CARGO_EXECUTABLE PARENT_PATH MOBCAM_CARGO_BIN_DIR)

set(MOBCAM_CARGO_PATH "--modify" "PATH=path_list_prepend:${MOBCAM_CARGO_BIN_DIR}")

set(MOBCAM_CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/rust-target")
set(
  MOBCAM_RUST_LIB
  "${CMAKE_BINARY_DIR}/rust/$<CONFIG>/${CMAKE_STATIC_LIBRARY_PREFIX}mobcam_rs${CMAKE_STATIC_LIBRARY_SUFFIX}"
)

# cargo has one optimized profile and one unoptimized one; the CMake
# configurations map onto that pair. Both are generator expressions because the
# Xcode and Visual Studio generators only know the configuration at build time.
# The profile is always named rather than passed as a bare --release, so that
# neither configuration ends up with an empty argument on the command line.
set(MOBCAM_CARGO_PROFILE "$<IF:$<CONFIG:Debug>,dev,release>")
set(MOBCAM_CARGO_PROFILE_DIR "$<IF:$<CONFIG:Debug>,debug,release>")

if(OS_LINUX)
  set(MOBCAM_FFMPEG_INCLUDE_DIRS "$<TARGET_PROPERTY:PkgConfig::FFMPEG,INTERFACE_INCLUDE_DIRECTORIES>")
else()
  set(MOBCAM_FFMPEG_INCLUDE_DIRS "${FFMPEG_INCLUDE_DIR}")
endif()

# bindgen reads the headers CMake resolved above rather than headers of its own.
# That is what keeps the generated bindings matched to the libraries the plugin
# is linked against, and in particular to the FFmpeg that OBS loads at runtime.
# The lists are joined with a bar rather than the usual semicolon so that they
# survive as one argument each on the way to build.rs.
set(
  MOBCAM_CARGO_ENV
  "MOBCAM_OBS_INCLUDE_DIRS=$<JOIN:$<TARGET_PROPERTY:OBS::libobs,INTERFACE_INCLUDE_DIRECTORIES>,|>"
  "MOBCAM_FFMPEG_INCLUDE_DIRS=$<JOIN:${MOBCAM_FFMPEG_INCLUDE_DIRS},|>"
  "CARGO_TARGET_DIR=${MOBCAM_CARGO_TARGET_DIR}"
)

if(OS_WINDOWS)
  # Rust links the static CRT by default on MSVC while the C half and libobs use
  # the dynamic one, which ends in duplicate CRT symbols. Put Rust on /MD too.
  list(APPEND MOBCAM_CARGO_ENV "RUSTFLAGS=-C target-feature=-crt-static")
endif()

if(OS_MACOS)
  # cargo builds one architecture at a time, so the universal binary the macos
  # preset asks for is one build per architecture followed by a lipo.
  set(_cargo_build_commands "")
  set(_cargo_arch_libs "")

  foreach(_arch IN LISTS CMAKE_OSX_ARCHITECTURES)
    if(_arch STREQUAL "arm64")
      set(_triple "aarch64-apple-darwin")
    else()
      set(_triple "${_arch}-apple-darwin")
    endif()

    list(
      APPEND _cargo_build_commands
      COMMAND
      ${CMAKE_COMMAND}
      -E
      env
      ${MOBCAM_CARGO_PATH}
      ${MOBCAM_CARGO_ENV}
      ${CARGO_EXECUTABLE}
      build
      --locked
      --profile
      ${MOBCAM_CARGO_PROFILE}
      --target
      ${_triple}
    )
    list(APPEND _cargo_arch_libs "${MOBCAM_CARGO_TARGET_DIR}/${_triple}/${MOBCAM_CARGO_PROFILE_DIR}/libmobcam_rs.a")
  endforeach()

  add_custom_target(
    mobcam-rust
    ALL
    ${_cargo_build_commands}
    COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/rust/$<CONFIG>"
    COMMAND lipo -create ${_cargo_arch_libs} -output "${MOBCAM_RUST_LIB}"
    BYPRODUCTS "${MOBCAM_RUST_LIB}"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    COMMENT "Building Rust static library"
    VERBATIM
  )
else()
  add_custom_target(
    mobcam-rust
    ALL
    COMMAND
      ${CMAKE_COMMAND} -E env ${MOBCAM_CARGO_PATH} ${MOBCAM_CARGO_ENV} ${CARGO_EXECUTABLE} build --locked --profile
      ${MOBCAM_CARGO_PROFILE}
    COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/rust/$<CONFIG>"
    COMMAND
      ${CMAKE_COMMAND} -E copy_if_different
      "${MOBCAM_CARGO_TARGET_DIR}/${MOBCAM_CARGO_PROFILE_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}mobcam_rs${CMAKE_STATIC_LIBRARY_SUFFIX}"
      "${MOBCAM_RUST_LIB}"
    BYPRODUCTS "${MOBCAM_RUST_LIB}"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    COMMENT "Building Rust static library"
    VERBATIM
  )
endif()

add_dependencies(${CMAKE_PROJECT_NAME} mobcam-rust)

# Every source file is Rust, so the module has nothing of its own to compile and
# CMake has no language to pick a linker from.
set_target_properties(${CMAKE_PROJECT_NAME} PROPERTIES LINKER_LANGUAGE C)

# A static library only contributes the object files that resolve an undefined
# symbol. Nothing in the module references the entry points libobs looks up, so
# without this the archive would contribute nothing and the plugin would load
# but register no source. The archive is therefore pulled in whole.
if(OS_WINDOWS)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE "${MOBCAM_RUST_LIB}")
  target_link_options(${CMAKE_PROJECT_NAME} PRIVATE "/WHOLEARCHIVE:${MOBCAM_RUST_LIB}")

  # Being in the archive is not the same as being exported from the DLL, which a
  # Rust #[no_mangle] does not arrange on its own. libobs finds these by name.
  foreach(
    _export
    IN
    ITEMS
      obs_module_load
      obs_module_unload
      obs_module_set_pointer
      obs_module_ver
      obs_module_set_locale
      obs_module_free_locale
      obs_module_name
      obs_module_description
  )
    target_link_options(${CMAKE_PROJECT_NAME} PRIVATE "/EXPORT:${_export}")
  endforeach()
elseif(OS_MACOS)
  target_link_options(${CMAKE_PROJECT_NAME} PRIVATE "-Wl,-force_load,${MOBCAM_RUST_LIB}")
else()
  target_link_options(${CMAKE_PROJECT_NAME} PRIVATE "-Wl,--whole-archive" "${MOBCAM_RUST_LIB}" "-Wl,--no-whole-archive")
endif()

# A static library does not carry the system libraries the Rust standard library
# needs, so they are named here. The authoritative list for a toolchain comes
# from `cargo rustc -- --print native-static-libs`; macOS needs nothing beyond
# libSystem, which the linker driver already supplies.
if(OS_LINUX)
  find_package(Threads REQUIRED)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE Threads::Threads ${CMAKE_DL_LIBS} m rt)
elseif(OS_WINDOWS)
  target_link_libraries(
    ${CMAKE_PROJECT_NAME}
    PRIVATE advapi32 bcrypt userenv ntdll dbghelp ole32 oleaut32 shell32
  )
endif()

set_target_properties_plugin(${CMAKE_PROJECT_NAME} PROPERTIES OUTPUT_NAME ${_name})
