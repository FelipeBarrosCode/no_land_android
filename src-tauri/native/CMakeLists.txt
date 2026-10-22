cmake_minimum_required(VERSION 3.20)
project(noland_moonlight_native C)

if(APPLE)
  enable_language(OBJC)
endif()
if(WIN32)
  enable_language(CXX)
endif()

option(BUILD_NOLAND_MOONLIGHT_HARNESS "Build the standalone native harness" ON)
option(BUILD_NOLAND_MOONLIGHT_TESTS "Build native tests" ON)

set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build native dependencies as static libraries" FORCE)

if(CMAKE_SYSTEM_NAME STREQUAL "iOS" OR ANDROID)
  set(CMAKE_POSITION_INDEPENDENT_CODE ON)
  include(FetchContent)
  set(FETCHCONTENT_UPDATES_DISCONNECTED ON)

  set(ENABLE_PROGRAMS OFF CACHE BOOL "" FORCE)
  set(ENABLE_TESTING OFF CACHE BOOL "" FORCE)
  set(MBEDTLS_FATAL_WARNINGS OFF CACHE BOOL "" FORCE)
  FetchContent_Declare(
    mbedtls
    # Mbed TLS 3.6.4
    GIT_REPOSITORY https://github.com/Mbed-TLS/mbedtls.git
    GIT_TAG c765c831e5c2a0971410692f92f7a81d6ec65ec2
    GIT_SHALLOW FALSE
  )
  FetchContent_MakeAvailable(mbedtls)

  set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
  set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
  set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
  set(OPUS_INSTALL_PKG_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
  set(OPUS_INSTALL_CMAKE_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
  FetchContent_Declare(
    opus
    # Opus 1.5.2
    GIT_REPOSITORY https://github.com/xiph/opus.git
    GIT_TAG ddbe48383984d56acd9e1ab6a090c54ca6b735a6
    GIT_SHALLOW FALSE
  )
  FetchContent_MakeAvailable(opus)

  set(USE_MBEDTLS ON CACHE BOOL "" FORCE)
endif()

if(BUILD_NOLAND_MOONLIGHT_TESTS)
  enable_testing()
endif()

add_subdirectory(moonlight-common-c)
add_subdirectory(noland-moonlight)

if(BUILD_NOLAND_MOONLIGHT_HARNESS)
  add_subdirectory(harness/noland-stream-harness)
endif()
