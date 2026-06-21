// SPDX-License-Identifier: GPL-3.0-or-later
//
// ps2-host: the SDL3 host backend for the PS2 recompiler runtime, exported as a
// flat C ABI (see include/ps2_host_abi.h). The C++ core keeps CPU/VU/VIF/GIF and
// the GS software rasterizer; this crate owns the window, GPU present, audio
// output, pad input, and memory-card files.
//
// Threading (per the ABI contract): present + pump_events run on the C++ main
// thread; audio_submit runs on the audio/VSync worker; pad_read runs on the
// guest HLE thread. Audio, input, and savecard state therefore sit behind
// mutexes. Video is touched only from the main thread.

// The C ABI exports take and dereference raw pointers supplied by the C++ core.
// C callers cannot invoke `unsafe fn`, so these are plain `extern "C"` and the
// pointer contracts are documented in ps2_host_abi.h; suppress the lint that
// would otherwise demand `unsafe fn` here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod audio;
mod input;
mod savecard;
mod video;

use core::ffi::{c_char, c_int};
use std::sync::Mutex;
use std::time::Instant;

use sdl3_sys::events::*;
use sdl3_sys::init::*;
use sdl3_sys::video::*;

use audio::AudioOutput;
use input::PadInput;
use savecard::SaveCards;
use video::Video;

/// PS2 DualShock2 pad state, matching `PS2PadState` in ps2_host_abi.h.
#[repr(C)]
pub struct PS2PadState {
    pub buttons: u16,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
    pub pressure: [u8; 12],
}

/// Opaque host instance handed back to C++ as `PS2Host*`.
pub struct Host {
    window: *mut SDL_Window,
    // Video, audio, and input hold SDL resources that must be released BEFORE
    // SDL_DestroyWindow/SDL_Quit. They are Options so Drop for Host can take and
    // drop them in the correct order; field drops afterwards become no-ops.
    video: Option<Video>,
    audio: Mutex<Option<AudioOutput>>,
    input: Mutex<Option<PadInput>>,
    cards: Mutex<SaveCards>,
    start: Instant,
    quit: std::sync::atomic::AtomicBool,
}

unsafe impl Send for Host {}
unsafe impl Sync for Host {}

impl Host {
    fn create(title: &str, win_w: u32, win_h: u32) -> Option<Box<Host>> {
        unsafe {
            if !SDL_Init(SDL_InitFlags(
                SDL_INIT_VIDEO.0 | SDL_INIT_AUDIO.0 | SDL_INIT_GAMEPAD.0,
            )) {
                return None;
            }

            let c_title = std::ffi::CString::new(title).unwrap_or_default();
            let window = SDL_CreateWindow(
                c_title.as_ptr(),
                win_w as c_int,
                win_h as c_int,
                SDL_WINDOW_RESIZABLE,
            );
            if window.is_null() {
                SDL_Quit();
                return None;
            }

            let Some(video) = Video::new(window) else {
                SDL_DestroyWindow(window);
                SDL_Quit();
                return None;
            };

            Some(Box::new(Host {
                window,
                video: Some(video),
                audio: Mutex::new(None),
                input: Mutex::new(Some(PadInput::new())),
                cards: Mutex::new(SaveCards::new()),
                start: Instant::now(),
                quit: std::sync::atomic::AtomicBool::new(false),
            }))
        }
    }

    fn pump_events(&self) -> bool {
        unsafe {
            let mut ev: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut ev) {
                let kind = SDL_EventType(ev.r#type);
                if kind == SDL_EVENT_QUIT || kind == SDL_EVENT_WINDOW_CLOSE_REQUESTED {
                    self.quit.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        !self.quit.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Release SDL-backed resources (GPU device, audio stream, gamepad)
        // BEFORE tearing down the window and SDL itself; otherwise their own
        // Drop impls would call into an already-quit SDL.
        self.video.take();
        if let Ok(mut slot) = self.audio.lock() {
            slot.take();
        }
        if let Ok(mut slot) = self.input.lock() {
            slot.take();
        }
        unsafe {
            if !self.window.is_null() {
                SDL_DestroyWindow(self.window);
                self.window = core::ptr::null_mut();
            }
            SDL_Quit();
        }
    }
}

// ---- C ABI ----
//
// Every entry point catches panics so a Rust panic can never unwind across the
// FFI boundary into C++.

macro_rules! host_ref {
    ($host:expr, $ret:expr) => {{
        if $host.is_null() {
            return $ret;
        }
        unsafe { &*$host }
    }};
}

fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

#[no_mangle]
pub extern "C" fn ps2_host_create(
    window_title: *const c_char,
    win_w: u32,
    win_h: u32,
) -> *mut Host {
    guard(core::ptr::null_mut(), || {
        let title = if window_title.is_null() {
            String::from("PS2 Game")
        } else {
            unsafe { std::ffi::CStr::from_ptr(window_title) }
                .to_string_lossy()
                .into_owned()
        };
        match Host::create(&title, win_w.max(1), win_h.max(1)) {
            Some(host) => Box::into_raw(host),
            None => core::ptr::null_mut(),
        }
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_destroy(host: *mut Host) {
    if host.is_null() {
        return;
    }
    guard((), || {
        // Reclaim ownership and drop.
        let _ = unsafe { Box::from_raw(host) };
    });
}

#[no_mangle]
pub extern "C" fn ps2_host_pump_events(host: *mut Host) -> c_int {
    guard(0, || {
        let h = host_ref!(host, 0);
        if h.pump_events() {
            1
        } else {
            0
        }
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_seconds(host: *mut Host) -> f64 {
    guard(0.0, || {
        let h = host_ref!(host, 0.0);
        h.start.elapsed().as_secs_f64()
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_present_frame(host: *mut Host, rgba: *const u8, w: u32, h: u32) {
    guard((), || {
        if host.is_null() {
            return;
        }
        // present mutates Video; safe because it is only ever called from the
        // single main/present thread, as documented in the ABI.
        let host_mut = unsafe { &mut *host };
        if let Some(video) = host_mut.video.as_mut() {
            video.present(rgba, w, h);
        }
    });
}

#[no_mangle]
pub extern "C" fn ps2_host_audio_open(host: *mut Host, sample_rate: u32, channels: u32) -> c_int {
    guard(0, || {
        let h = host_ref!(host, 0);
        let ch = if channels == 0 { 2 } else { channels };
        let rate = if sample_rate == 0 { 48000 } else { sample_rate };
        match AudioOutput::open(rate, ch) {
            Some(out) => {
                if let Ok(mut slot) = h.audio.lock() {
                    *slot = Some(out);
                    return 1;
                }
                0
            }
            None => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_audio_submit(host: *mut Host, interleaved: *const i16, frames: u32) {
    guard((), || {
        if host.is_null() {
            return;
        }
        let h = unsafe { &*host };
        if let Ok(slot) = h.audio.lock() {
            if let Some(out) = slot.as_ref() {
                out.submit(interleaved, frames);
            }
        }
    });
}

#[no_mangle]
pub extern "C" fn ps2_host_audio_queued_frames(host: *mut Host) -> u32 {
    guard(0, || {
        let h = host_ref!(host, 0);
        if let Ok(slot) = h.audio.lock() {
            if let Some(out) = slot.as_ref() {
                return out.queued_frames();
            }
        }
        0
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_audio_clear(host: *mut Host) {
    guard((), || {
        if host.is_null() {
            return;
        }
        let h = unsafe { &*host };
        if let Ok(slot) = h.audio.lock() {
            if let Some(out) = slot.as_ref() {
                out.clear();
            }
        }
    });
}

#[no_mangle]
pub extern "C" fn ps2_host_pad_read(
    host: *mut Host,
    port: c_int,
    slot: c_int,
    out: *mut PS2PadState,
) -> c_int {
    guard(0, || {
        let h = host_ref!(host, 0);
        if out.is_null() {
            return 0;
        }
        // Only port 0, slot 0 is a connected device.
        if port != 0 || slot != 0 {
            return 0;
        }
        if let Ok(mut input) = h.input.lock() {
            if let Some(pad) = input.as_mut() {
                let state = pad.read();
                unsafe {
                    *out = state;
                }
                return 1;
            }
        }
        0
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_mc_read(
    host: *mut Host,
    port: c_int,
    slot: c_int,
    offset: u64,
    dst: *mut u8,
    len: u32,
) -> c_int {
    guard(1, || {
        let h = host_ref!(host, 1);
        if dst.is_null() {
            return 1;
        }
        let buf = unsafe { std::slice::from_raw_parts_mut(dst, len as usize) };
        if let Ok(mut cards) = h.cards.lock() {
            if cards.read(port, slot, offset, buf) {
                return 0;
            }
        }
        1
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_mc_write(
    host: *mut Host,
    port: c_int,
    slot: c_int,
    offset: u64,
    src: *const u8,
    len: u32,
) -> c_int {
    guard(1, || {
        let h = host_ref!(host, 1);
        if src.is_null() {
            return 1;
        }
        let buf = unsafe { std::slice::from_raw_parts(src, len as usize) };
        if let Ok(mut cards) = h.cards.lock() {
            if cards.write(port, slot, offset, buf) {
                return 0;
            }
        }
        1
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_mc_flush(host: *mut Host, port: c_int, slot: c_int) -> c_int {
    guard(1, || {
        let h = host_ref!(host, 1);
        if let Ok(mut cards) = h.cards.lock() {
            if cards.flush(port, slot) {
                return 0;
            }
        }
        1
    })
}

#[no_mangle]
pub extern "C" fn ps2_host_mc_info(
    host: *mut Host,
    port: c_int,
    slot: c_int,
    type_out: *mut u32,
    free_blocks: *mut u32,
    format: *mut u32,
) -> c_int {
    guard(1, || {
        let h = host_ref!(host, 1);
        if let Ok(mut cards) = h.cards.lock() {
            if let Some((t, free, fmt)) = cards.info(port, slot) {
                unsafe {
                    if !type_out.is_null() {
                        *type_out = t;
                    }
                    if !free_blocks.is_null() {
                        *free_blocks = free;
                    }
                    if !format.is_null() {
                        *format = fmt;
                    }
                }
                return 0;
            }
        }
        1
    })
}
