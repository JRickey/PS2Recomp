// SPDX-License-Identifier: GPL-3.0-or-later
//
// Audio output: a single SDL3 audio stream fed interleaved S16 PCM by the C++
// SPU2 path. SDL_AudioStream resamples/queues internally and is thread-safe, so
// submit may be called from the audio/VSync worker.

use core::ffi::{c_int, c_void};
use sdl3_sys::audio::*;

pub struct AudioOutput {
    stream: *mut SDL_AudioStream,
    channels: u32,
}

// SDL_AudioStream calls are internally locked, so it is safe to submit from a
// different thread than the one that opened it.
unsafe impl Send for AudioOutput {}

impl AudioOutput {
    /// Open the default playback device with the requested format and start it.
    /// Returns None on failure.
    pub fn open(sample_rate: u32, channels: u32) -> Option<AudioOutput> {
        let spec = SDL_AudioSpec {
            format: SDL_AUDIO_S16,
            channels: channels as c_int,
            freq: sample_rate as c_int,
        };
        let stream = unsafe {
            SDL_OpenAudioDeviceStream(
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &spec,
                None,
                core::ptr::null_mut(),
            )
        };
        if stream.is_null() {
            return None;
        }
        // Streams open paused; resume so queued data plays.
        unsafe {
            SDL_ResumeAudioStreamDevice(stream);
        }
        Some(AudioOutput { stream, channels })
    }

    /// Queue `frames` sample-frames of interleaved S16 (frames * channels i16).
    pub fn submit(&self, interleaved: *const i16, frames: u32) {
        if self.stream.is_null() || interleaved.is_null() || frames == 0 {
            return;
        }
        let bytes = frames as usize * self.channels as usize * core::mem::size_of::<i16>();
        unsafe {
            SDL_PutAudioStreamData(self.stream, interleaved as *const c_void, bytes as c_int);
        }
    }

    /// Sample-frames currently queued (bytes / frame size), for backpressure.
    pub fn queued_frames(&self) -> u32 {
        if self.stream.is_null() {
            return 0;
        }
        let queued = unsafe { SDL_GetAudioStreamQueued(self.stream) };
        if queued <= 0 {
            return 0;
        }
        let frame_bytes = self.channels as usize * core::mem::size_of::<i16>();
        (queued as usize / frame_bytes.max(1)) as u32
    }

    pub fn clear(&self) {
        if !self.stream.is_null() {
            unsafe {
                SDL_ClearAudioStream(self.stream);
            }
        }
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            unsafe {
                SDL_DestroyAudioStream(self.stream);
            }
            self.stream = core::ptr::null_mut();
        }
    }
}
