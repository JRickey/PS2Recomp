#include "runtime/ps2_pad.h"
#include "ps2_host_backend.h"
#include <cstring>

namespace
{
    constexpr uint8_t kPadAnalogMarker = 0x73;
    constexpr uint8_t kPadStickCenter = 0x80;
}

// Marshal host pad state into the legacy 32-byte SCE pad-data layout used by the
// simple pad path. Buttons are active-low, matching PS2PadState.buttons and the
// kPadBtn* layout shared with Kernel/Stubs/Pad.cpp.
bool PSPadBackend::readState(int port, int slot, uint8_t *data, size_t size)
{
    if (!data || size < 32)
        return false;

    std::memset(data, 0, 32);
    data[0] = 0x01;
    data[1] = kPadAnalogMarker;
    data[2] = 0xFF;
    data[3] = 0xFF;
    data[4] = data[5] = data[6] = data[7] = kPadStickCenter;

    if (!m_host)
        return false;

    PS2PadState state{};
    if (!ps2_host_pad_read(m_host, port, slot, &state))
        return false;

    data[2] = static_cast<uint8_t>(state.buttons & 0xFFu);
    data[3] = static_cast<uint8_t>(state.buttons >> 8);
    // Layout matches the original raylib path: data[4..5] = right stick,
    // data[6..7] = left stick.
    data[4] = state.rx;
    data[5] = state.ry;
    data[6] = state.lx;
    data[7] = state.ly;
    return true;
}
