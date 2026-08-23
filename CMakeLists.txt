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

if(OS_WINDOWS)
  target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE ws2_32)

  # libobs bundles the pthreads implementation this plugin's threading uses.
  if(TARGET OBS::w32-pthreads)
    target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE OBS::w32-pthreads)
  endif()
endif()

target_sources(
  ${CMAKE_PROJECT_NAME}
  PRIVATE
    src/mobcam-decoder.c
    src/mobcam-decoder.h
    src/mobcam-protocol.c
    src/mobcam-protocol.h
    src/mobcam-source.c
    src/mobcam-source.h
    src/plist.c
    src/plist.h
    src/plugin-main.c
    src/socket-compat.c
    src/socket-compat.h
    src/usbmux.c
    src/usbmux.h
)

set_target_properties_plugin(${CMAKE_PROJECT_NAME} PROPERTIES OUTPUT_NAME ${_name})
