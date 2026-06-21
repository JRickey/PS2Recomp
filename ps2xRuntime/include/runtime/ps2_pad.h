#ifndef PS2_PAD_H
#define PS2_PAD_H

#include <cstddef>
#include <cstdint>

struct PS2Host;

class PSPadBackend
{
public:
    PSPadBackend() = default;
    ~PSPadBackend() = default;

    // Bind the SDL3 host that provides pad state via ps2_host_pad_read.
    void setHost(PS2Host *host) { m_host = host; }

    bool readState(int port, int slot, uint8_t *data, size_t size);

private:
    PS2Host *m_host = nullptr;
};

#endif
