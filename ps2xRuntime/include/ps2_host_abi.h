/* SPDX-License-Identifier: GPL-3.0-or-later */
/*
 * ps2_host_abi.h - Flat C ABI between the C++ recompiler core and the Rust
 * SDL3 host crate. The C++ side keeps CPU/VU/VIF/GIF/GS-software-raster and the
 * kernel HLE; everything below the line - window/event pump, video present,
 * audio output, pad input, and memory-card backing - lives in the host crate.
 *
 * Rules for this boundary: C linkage, opaque handles, no STL, no C++ objects,
 * no exceptions. Errors are int returns. See notes/documentation/host-seam.md.
 */
#ifndef PS2_HOST_ABI_H
#define PS2_HOST_ABI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C"
{
#endif

    /* Opaque host instance, owned by the Rust side. */
    typedef struct PS2Host PS2Host;

    /* ---- A.1 Lifecycle, window, event pump, clock ---- */

    /* Create the window and GPU device. Returns NULL on failure. The window is
     * resizable; the host owns its size and performs letterboxed present. */
    PS2Host *ps2_host_create(const char *window_title, uint32_t win_w, uint32_t win_h);
    void ps2_host_destroy(PS2Host *host);

    /* Service the OS event queue once. Returns 0 when the user has requested
     * quit (window close), non-zero otherwise. Replaces WindowShouldClose(). */
    int ps2_host_pump_events(PS2Host *host);

    /* Monotonic clock in seconds since host creation. Replaces GetTime(). */
    double ps2_host_seconds(PS2Host *host);

    /* ---- A.2 Video (finished-framebuffer seam) ---- */

    /* Upload a tightly packed RGBA8888 frame (w*h*4 bytes) and present it
     * letterboxed to the window. The host owns the GPU texture lifecycle and
     * resizes its upload texture as the guest resolution changes. */
    void ps2_host_present_frame(PS2Host *host, const uint8_t *rgba, uint32_t w, uint32_t h);

    /* ---- A.3 Audio (continuous interleaved-stereo PCM) ---- */

    /* Open an output stream. channels is 1 (mono) or 2 (stereo); submitted PCM
     * is signed 16-bit interleaved. Returns 0 on failure. */
    int ps2_host_audio_open(PS2Host *host, uint32_t sample_rate, uint32_t channels);

    /* Queue `frames` sample-frames of interleaved S16 PCM (frames * channels
     * int16 values). Thread-safe; called from the audio/VSync worker. */
    void ps2_host_audio_submit(PS2Host *host, const int16_t *interleaved, uint32_t frames);

    /* Sample-frames still queued in the output stream, for backpressure. */
    uint32_t ps2_host_audio_queued_frames(PS2Host *host);

    /* Drop all queued audio (e.g. scene/track change). */
    void ps2_host_audio_clear(PS2Host *host);

    /* ---- A.4 Input (pad state polling) ---- */

    /* PS2 DualShock2 state for one port/slot, filled by the host from keyboard
     * and gamepad. `buttons` is the PS2 button bitmask, ACTIVE-LOW (a 0 bit
     * means pressed), matching the layout in Pad.cpp. Sticks are 0..255 with
     * 0x80 centered. `pressure` is DS2 per-button pressure (0 when N/A). */
    typedef struct PS2PadState
    {
        uint16_t buttons;
        uint8_t lx, ly, rx, ry;
        uint8_t pressure[12];
    } PS2PadState;

    /* Fill *out for the given port/slot. Returns 0 if no device is connected. */
    int ps2_host_pad_read(PS2Host *host, int port, int slot, PS2PadState *out);

    /* ---- A.5 Saves (memory-card byte-range access) ---- */

    /* PS2 memory card is 8MB of 0x400-byte pages. Byte-range read/write against
     * the host-owned, sandboxed save file. Return 0 on success, non-zero on
     * error. flush forces durability; info reports card metadata. */
    int ps2_host_mc_read(PS2Host *host, int port, int slot, uint64_t offset, uint8_t *dst, uint32_t len);
    int ps2_host_mc_write(PS2Host *host, int port, int slot, uint64_t offset, const uint8_t *src, uint32_t len);
    int ps2_host_mc_flush(PS2Host *host, int port, int slot);
    int ps2_host_mc_info(PS2Host *host, int port, int slot, uint32_t *type, uint32_t *free_blocks, uint32_t *format);

#ifdef __cplusplus
}
#endif

#endif /* PS2_HOST_ABI_H */
