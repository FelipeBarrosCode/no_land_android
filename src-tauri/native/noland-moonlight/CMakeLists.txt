set(NOLAND_NATIVE_PREFIX "" CACHE PATH "Prefix containing native SDL2/Opus dependencies")

set(NOLAND_NATIVE_INCLUDE_HINTS)
set(NOLAND_NATIVE_LIB_HINTS)
if(NOLAND_NATIVE_PREFIX)
  list(APPEND NOLAND_NATIVE_INCLUDE_HINTS "${NOLAND_NATIVE_PREFIX}/include")
  list(APPEND NOLAND_NATIVE_LIB_HINTS "${NOLAND_NATIVE_PREFIX}/lib")
endif()

add_library(noland_moonlight STATIC
  src/noland_moonlight.c
  src/noland_controller_manager.c
  src/noland_video_renderer.c
  src/noland_latency_telemetry.c
  src/noland_frame_deadline_policy.c
)

if(NOT CMAKE_SYSTEM_NAME STREQUAL "iOS" AND NOT ANDROID)
  find_path(SDL2_INCLUDE_ROOT SDL2/SDL.h HINTS ${NOLAND_NATIVE_INCLUDE_HINTS} REQUIRED)
  find_library(SDL2_LIBRARY NAMES SDL2 SDL2-static HINTS ${NOLAND_NATIVE_LIB_HINTS} REQUIRED)
  target_include_directories(noland_moonlight PRIVATE ${SDL2_INCLUDE_ROOT} ${SDL2_INCLUDE_ROOT}/SDL2)
  target_link_libraries(noland_moonlight PRIVATE ${SDL2_LIBRARY})
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "iOS")
  target_compile_definitions(noland_moonlight PRIVATE NOLAND_IOS_RENDERER)
  target_sources(noland_moonlight PRIVATE
    src/noland_video_renderer_ios.m
    src/noland_stream_controls_ios.m
    src/noland_audio_renderer_ios.m
    src/noland_microphone_ios.m
    src/noland_input_ios.c
  )
elseif(ANDROID)
  target_compile_definitions(noland_moonlight PRIVATE NOLAND_ANDROID_RENDERER)
  target_sources(noland_moonlight PRIVATE
    src/noland_video_renderer_android.c
    src/noland_audio_renderer_android.c
  )
  # Android input may be supplied by the Java/JNI bridge instead.
  if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/noland_input_android.c")
    target_sources(noland_moonlight PRIVATE src/noland_input_android.c)
  endif()
  target_link_libraries(noland_moonlight PRIVATE android log mediandk OpenSLES m)
elseif(APPLE)
  target_sources(noland_moonlight PRIVATE src/noland_video_renderer_macos.m src/noland_audio_renderer_macos.m)
elseif(UNIX)
  target_sources(noland_moonlight PRIVATE
    src/noland_audio_renderer_sdl.c
    src/noland_desktop_input_sdl.c
    src/noland_video_renderer_linux.c
  )
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(GSTREAMER REQUIRED gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0)
  target_include_directories(noland_moonlight PRIVATE ${GSTREAMER_INCLUDE_DIRS})
  target_link_directories(noland_moonlight PRIVATE ${GSTREAMER_LIBRARY_DIRS})
  target_link_libraries(noland_moonlight PRIVATE ${GSTREAMER_LIBRARIES})
elseif(WIN32)
  target_sources(noland_moonlight PRIVATE
    src/noland_audio_renderer_sdl.c
    src/noland_desktop_input_sdl.c
    src/noland_video_renderer_windows.cpp
  )
  target_compile_features(noland_moonlight PRIVATE cxx_std_17)
  target_link_libraries(noland_moonlight PRIVATE d3d11 dxgi mf mfplat mfuuid ole32 user32 gdi32)
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "iOS" OR ANDROID)
  set(OPUS_INCLUDE_DIR "${opus_SOURCE_DIR}/include")
  set(OPUS_LIBRARY_TARGET opus)
else()
  find_path(OPUS_INCLUDE_DIR opus/opus_multistream.h HINTS ${NOLAND_NATIVE_INCLUDE_HINTS})
  find_library(OPUS_LIBRARY opus HINTS ${NOLAND_NATIVE_LIB_HINTS})
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "iOS" OR ANDROID)
  # The source-built target is linked below.
elseif(WIN32)
  if(OPUS_INCLUDE_DIR AND OPUS_LIBRARY)
    set(OPUS_LIBRARY_TARGET ${OPUS_LIBRARY})
  else()
    include(FetchContent)
    set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
    set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
    set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
    set(OPUS_STACK_PROTECTOR OFF CACHE BOOL "" FORCE)
    set(OPUS_DISABLE_INTRINSICS ON CACHE BOOL "" FORCE)
    FetchContent_Declare(
      opus_dep
      URL https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz
      DOWNLOAD_EXTRACT_TIMESTAMP true
    )
    FetchContent_MakeAvailable(opus_dep)
    set(OPUS_INCLUDE_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated-opus-include")
    file(MAKE_DIRECTORY "${OPUS_INCLUDE_DIR}/opus")
    file(COPY "${opus_dep_SOURCE_DIR}/include/" DESTINATION "${OPUS_INCLUDE_DIR}/opus")
    set(OPUS_LIBRARY_TARGET opus)
  endif()
else()
  if(NOT OPUS_INCLUDE_DIR)
    find_path(OPUS_INCLUDE_DIR opus/opus_multistream.h HINTS ${NOLAND_NATIVE_INCLUDE_HINTS} REQUIRED)
  endif()
  if(NOT OPUS_LIBRARY)
    find_library(OPUS_LIBRARY opus HINTS ${NOLAND_NATIVE_LIB_HINTS} REQUIRED)
  endif()
endif()

target_include_directories(noland_moonlight
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
  PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../moonlight-common-c/src
    ${OPUS_INCLUDE_DIR}
)

target_compile_features(noland_moonlight PRIVATE c_std_11)
set_target_properties(noland_moonlight PROPERTIES POSITION_INDEPENDENT_CODE ON)
if(CMAKE_SYSTEM_NAME STREQUAL "iOS" OR ANDROID)
  target_link_libraries(noland_moonlight PRIVATE moonlight-common-c ${OPUS_LIBRARY_TARGET})
elseif(WIN32)
  target_link_libraries(noland_moonlight PRIVATE moonlight-common-c ${OPUS_LIBRARY_TARGET})
else()
  target_link_libraries(noland_moonlight PRIVATE moonlight-common-c ${OPUS_LIBRARY})
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "iOS")
  find_library(UIKIT_FRAMEWORK UIKit REQUIRED)
  find_library(AVFOUNDATION_FRAMEWORK AVFoundation REQUIRED)
  find_library(AUDIOTOOLBOX_FRAMEWORK AudioToolbox REQUIRED)
  find_library(COREMEDIA_FRAMEWORK CoreMedia REQUIRED)
  find_library(COREVIDEO_FRAMEWORK CoreVideo REQUIRED)
  find_library(QUARTZCORE_FRAMEWORK QuartzCore REQUIRED)
  find_library(GAMECONTROLLER_FRAMEWORK GameController REQUIRED)
  find_library(COREHAPTICS_FRAMEWORK CoreHaptics REQUIRED)
  target_link_libraries(noland_moonlight PRIVATE
    ${UIKIT_FRAMEWORK}
    ${AVFOUNDATION_FRAMEWORK}
    ${AUDIOTOOLBOX_FRAMEWORK}
    ${COREMEDIA_FRAMEWORK}
    ${COREVIDEO_FRAMEWORK}
    ${QUARTZCORE_FRAMEWORK}
    ${GAMECONTROLLER_FRAMEWORK}
    ${COREHAPTICS_FRAMEWORK}
  )
  set_source_files_properties(
    src/noland_video_renderer_ios.m
    src/noland_stream_controls_ios.m
    src/noland_audio_renderer_ios.m
    src/noland_microphone_ios.m
    PROPERTIES COMPILE_FLAGS "-fobjc-arc"
  )
elseif(APPLE)
  find_library(APPKIT_FRAMEWORK AppKit REQUIRED)
  find_library(AVFOUNDATION_FRAMEWORK AVFoundation REQUIRED)
  find_library(COREMEDIA_FRAMEWORK CoreMedia REQUIRED)
  find_library(COREVIDEO_FRAMEWORK CoreVideo REQUIRED)
  find_library(QUARTZCORE_FRAMEWORK QuartzCore REQUIRED)
  target_link_libraries(noland_moonlight PRIVATE
    ${APPKIT_FRAMEWORK}
    ${AVFOUNDATION_FRAMEWORK}
    ${COREMEDIA_FRAMEWORK}
    ${COREVIDEO_FRAMEWORK}
    ${QUARTZCORE_FRAMEWORK}
  )
  set_source_files_properties(
    src/noland_video_renderer_macos.m
    src/noland_audio_renderer_macos.m
    PROPERTIES COMPILE_FLAGS "-fobjc-arc"
  )
endif()

install(TARGETS noland_moonlight ARCHIVE DESTINATION lib LIBRARY DESTINATION lib RUNTIME DESTINATION bin)
install(FILES include/noland_moonlight.h DESTINATION include)

if(BUILD_NOLAND_MOONLIGHT_TESTS)
  enable_testing()
  add_executable(noland_moonlight_smoke_test tests/smoke_test.c)
  target_link_libraries(noland_moonlight_smoke_test PRIVATE noland_moonlight)
  add_test(NAME noland_moonlight_smoke_test COMMAND noland_moonlight_smoke_test)

  add_executable(noland_latency_policy_test tests/latency_policy_test.c)
  target_include_directories(noland_latency_policy_test PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${CMAKE_CURRENT_SOURCE_DIR}/../moonlight-common-c/src
  )
  target_link_libraries(noland_latency_policy_test PRIVATE noland_moonlight)
  add_test(NAME noland_latency_policy_test COMMAND noland_latency_policy_test)
endif()
