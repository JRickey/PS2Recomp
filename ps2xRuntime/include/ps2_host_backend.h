#pragma once

// The host backend is the Rust + SDL3 crate behind a flat C ABI. This header is
// the single include the C++ core uses to reach it; all window/video/audio/
// input/save calls go through ps2_host_abi.h.
#include "ps2_host_abi.h"
