# Build the Rust SDL3 host crate (host-rs) as a staticlib and expose it as the
# imported target `ps2_host_rs`, carrying the SDL3 + platform framework link
# requirements. The C++ runtime links this in place of raylib.

find_program(PS2X_CARGO cargo REQUIRED)

set(PS2X_HOST_RS_DIR "${CMAKE_CURRENT_SOURCE_DIR}/host-rs")

# Map the CMake build type onto a cargo profile.
if(CMAKE_BUILD_TYPE STREQUAL "Release" OR CMAKE_BUILD_TYPE STREQUAL "RelWithDebInfo")
    set(PS2X_CARGO_PROFILE "release")
    set(PS2X_CARGO_FLAGS "--release")
else()
    set(PS2X_CARGO_PROFILE "debug")
    set(PS2X_CARGO_FLAGS "")
endif()

set(PS2X_HOST_RS_LIB
    "${PS2X_HOST_RS_DIR}/target/${PS2X_CARGO_PROFILE}/${CMAKE_STATIC_LIBRARY_PREFIX}ps2_host${CMAKE_STATIC_LIBRARY_SUFFIX}")

# Locate SDL3 so we can pass its pkg-config dir to cargo and link the dylib.
find_package(PkgConfig QUIET)
set(PS2X_SDL3_PKGCONFIG_DIR "")
if(APPLE)
    execute_process(
        COMMAND brew --prefix sdl3
        OUTPUT_VARIABLE PS2X_BREW_SDL3
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET)
    if(PS2X_BREW_SDL3 AND EXISTS "${PS2X_BREW_SDL3}/lib/pkgconfig")
        set(PS2X_SDL3_PKGCONFIG_DIR "${PS2X_BREW_SDL3}/lib/pkgconfig")
    endif()
endif()

# Compose the environment cargo runs under so sdl3-sys' pkg-config probe finds
# the system SDL3 even when it is off the default search path (macOS/Homebrew).
set(PS2X_CARGO_ENV "")
if(PS2X_SDL3_PKGCONFIG_DIR)
    set(PS2X_CARGO_ENV ${CMAKE_COMMAND} -E env "PKG_CONFIG_PATH=${PS2X_SDL3_PKGCONFIG_DIR}")
endif()

add_custom_command(
    OUTPUT "${PS2X_HOST_RS_LIB}"
    COMMAND ${PS2X_CARGO_ENV} ${PS2X_CARGO} build ${PS2X_CARGO_FLAGS}
    WORKING_DIRECTORY "${PS2X_HOST_RS_DIR}"
    BYPRODUCTS "${PS2X_HOST_RS_LIB}"
    COMMENT "Building Rust SDL3 host crate (ps2-host, ${PS2X_CARGO_PROFILE})"
    VERBATIM
    USES_TERMINAL)

add_custom_target(ps2_host_rs_build DEPENDS "${PS2X_HOST_RS_LIB}")

add_library(ps2_host_rs STATIC IMPORTED GLOBAL)
add_dependencies(ps2_host_rs ps2_host_rs_build)
set_target_properties(ps2_host_rs PROPERTIES
    IMPORTED_LOCATION "${PS2X_HOST_RS_LIB}")

# System dependencies the staticlib pulls in.
set(PS2X_HOST_RS_LINK_LIBS "")

# SDL3 dynamic library.
if(PS2X_BREW_SDL3 AND EXISTS "${PS2X_BREW_SDL3}/lib")
    find_library(PS2X_SDL3_LIB SDL3 PATHS "${PS2X_BREW_SDL3}/lib" NO_DEFAULT_PATH)
endif()
if(NOT PS2X_SDL3_LIB)
    find_library(PS2X_SDL3_LIB SDL3)
endif()
if(PS2X_SDL3_LIB)
    list(APPEND PS2X_HOST_RS_LINK_LIBS "${PS2X_SDL3_LIB}")
else()
    message(FATAL_ERROR "Could not locate the SDL3 library for linking the Rust host crate.")
endif()

if(APPLE)
    # Frameworks the Rust std + SDL3 Metal/Cocoa backends require.
    foreach(fw Cocoa Metal QuartzCore IOKit CoreAudio AudioToolbox CoreHaptics
               ForceFeedback GameController CoreFoundation Foundation Security)
        list(APPEND PS2X_HOST_RS_LINK_LIBS "-framework ${fw}")
    endforeach()
    list(APPEND PS2X_HOST_RS_LINK_LIBS "-liconv")
elseif(UNIX)
    list(APPEND PS2X_HOST_RS_LINK_LIBS dl pthread m)
endif()

set_target_properties(ps2_host_rs PROPERTIES
    INTERFACE_LINK_LIBRARIES "${PS2X_HOST_RS_LINK_LIBS}")
