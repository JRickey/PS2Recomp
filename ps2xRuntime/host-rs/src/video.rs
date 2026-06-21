// SPDX-License-Identifier: GPL-3.0-or-later
//
// Video present via the SDL3 GPU API (Metal on macOS, D3D12/Vulkan elsewhere).
// The C++ GS software rasterizer hands us a finished RGBA8888 frame; we upload
// it to a sampler texture and blit it letterboxed onto the swapchain. No
// shaders are needed - SDL_BlitGPUTexture does the scaled copy.

use sdl3_sys::gpu::*;
use sdl3_sys::pixels::SDL_FColor;
use sdl3_sys::video::SDL_Window;

pub struct Video {
    device: *mut SDL_GPUDevice,
    window: *mut SDL_Window,
    // Source texture sized to the current guest frame; recreated on resize.
    frame_tex: *mut SDL_GPUTexture,
    frame_w: u32,
    frame_h: u32,
    // Upload staging buffer, recreated to match the frame texture's byte size.
    transfer: *mut SDL_GPUTransferBuffer,
    transfer_bytes: u32,
}

unsafe impl Send for Video {}

impl Video {
    /// Create a GPU device and bind it to `window`. Returns None on failure.
    pub fn new(window: *mut SDL_Window) -> Option<Video> {
        // Request all shader formats so SDL picks the platform's native backend;
        // we don't ship shaders, but the device still needs a format hint.
        let format_flags = SDL_GPUShaderFormat(
            SDL_GPU_SHADERFORMAT_SPIRV.0 | SDL_GPU_SHADERFORMAT_MSL.0 | SDL_GPU_SHADERFORMAT_DXIL.0,
        );
        let device = unsafe { SDL_CreateGPUDevice(format_flags, false, core::ptr::null()) };
        if device.is_null() {
            return None;
        }
        if !unsafe { SDL_ClaimWindowForGPUDevice(device, window) } {
            unsafe { SDL_DestroyGPUDevice(device) };
            return None;
        }
        Some(Video {
            device,
            window,
            frame_tex: core::ptr::null_mut(),
            frame_w: 0,
            frame_h: 0,
            transfer: core::ptr::null_mut(),
            transfer_bytes: 0,
        })
    }

    /// Ensure the source texture and transfer buffer match `w`x`h`.
    fn ensure_frame_resources(&mut self, w: u32, h: u32) -> bool {
        if !self.frame_tex.is_null() && self.frame_w == w && self.frame_h == h {
            return true;
        }
        unsafe {
            if !self.frame_tex.is_null() {
                SDL_ReleaseGPUTexture(self.device, self.frame_tex);
                self.frame_tex = core::ptr::null_mut();
            }
            let info = SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format: SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                // SAMPLER so the blit can read it; COLOR_TARGET is implied by blit src.
                usage: SDL_GPU_TEXTUREUSAGE_SAMPLER,
                width: w,
                height: h,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                props: sdl3_sys::properties::SDL_PropertiesID(0),
            };
            self.frame_tex = SDL_CreateGPUTexture(self.device, &info);
            if self.frame_tex.is_null() {
                return false;
            }
            self.frame_w = w;
            self.frame_h = h;

            let needed = w * h * 4;
            if self.transfer.is_null() || self.transfer_bytes < needed {
                if !self.transfer.is_null() {
                    SDL_ReleaseGPUTransferBuffer(self.device, self.transfer);
                    self.transfer = core::ptr::null_mut();
                }
                let tinfo = SDL_GPUTransferBufferCreateInfo {
                    usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
                    size: needed,
                    props: sdl3_sys::properties::SDL_PropertiesID(0),
                };
                self.transfer = SDL_CreateGPUTransferBuffer(self.device, &tinfo);
                if self.transfer.is_null() {
                    return false;
                }
                self.transfer_bytes = needed;
            }
        }
        true
    }

    /// Upload an RGBA8888 frame and present it letterboxed.
    pub fn present(&mut self, rgba: *const u8, w: u32, h: u32) {
        if rgba.is_null() || w == 0 || h == 0 {
            return;
        }
        if !self.ensure_frame_resources(w, h) {
            return;
        }
        let frame_bytes = (w * h * 4) as usize;

        unsafe {
            // Copy guest pixels into the mapped transfer buffer.
            let mapped = SDL_MapGPUTransferBuffer(self.device, self.transfer, true);
            if mapped.is_null() {
                return;
            }
            core::ptr::copy_nonoverlapping(rgba, mapped as *mut u8, frame_bytes);
            SDL_UnmapGPUTransferBuffer(self.device, self.transfer);

            let cmd = SDL_AcquireGPUCommandBuffer(self.device);
            if cmd.is_null() {
                return;
            }

            // transfer buffer -> frame texture
            let copy_pass = SDL_BeginGPUCopyPass(cmd);
            if !copy_pass.is_null() {
                let src = SDL_GPUTextureTransferInfo {
                    transfer_buffer: self.transfer,
                    offset: 0,
                    pixels_per_row: w,
                    rows_per_layer: h,
                };
                let dst = SDL_GPUTextureRegion {
                    texture: self.frame_tex,
                    mip_level: 0,
                    layer: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    w,
                    h,
                    d: 1,
                };
                SDL_UploadToGPUTexture(copy_pass, &src, &dst, false);
                SDL_EndGPUCopyPass(copy_pass);
            }

            // Acquire the swapchain target. WaitAndAcquire blocks until one is
            // free, giving us vsync-paced present without a manual frame cap.
            let mut swap_tex: *mut SDL_GPUTexture = core::ptr::null_mut();
            let mut sw: u32 = 0;
            let mut sh: u32 = 0;
            let ok = SDL_WaitAndAcquireGPUSwapchainTexture(
                cmd,
                self.window,
                &mut swap_tex,
                &mut sw,
                &mut sh,
            );
            if !ok || swap_tex.is_null() || sw == 0 || sh == 0 {
                // No target this frame (e.g. minimized); still submit to release cmd.
                SDL_SubmitGPUCommandBuffer(cmd);
                return;
            }

            let (dx, dy, dw, dh) = letterbox(w, h, sw, sh);

            let blit = SDL_GPUBlitInfo {
                source: SDL_GPUBlitRegion {
                    texture: self.frame_tex,
                    mip_level: 0,
                    layer_or_depth_plane: 0,
                    x: 0,
                    y: 0,
                    w,
                    h,
                },
                destination: SDL_GPUBlitRegion {
                    texture: swap_tex,
                    mip_level: 0,
                    layer_or_depth_plane: 0,
                    x: dx,
                    y: dy,
                    w: dw,
                    h: dh,
                },
                // CLEAR fills the letterbox bars black before the blit.
                load_op: SDL_GPU_LOADOP_CLEAR,
                clear_color: SDL_FColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
                flip_mode: sdl3_sys::surface::SDL_FLIP_NONE,
                filter: SDL_GPU_FILTER_LINEAR,
                cycle: false,
                ..Default::default()
            };
            SDL_BlitGPUTexture(cmd, &blit);
            SDL_SubmitGPUCommandBuffer(cmd);
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        unsafe {
            if !self.transfer.is_null() {
                SDL_ReleaseGPUTransferBuffer(self.device, self.transfer);
            }
            if !self.frame_tex.is_null() {
                SDL_ReleaseGPUTexture(self.device, self.frame_tex);
            }
            if !self.device.is_null() {
                // Releasing the window association happens in DestroyGPUDevice.
                SDL_DestroyGPUDevice(self.device);
            }
        }
    }
}

/// Compute the letterboxed destination rect for `src` inside `dst`.
fn letterbox(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    let sw = src_w.max(1) as f32;
    let sh = src_h.max(1) as f32;
    let dw = dst_w as f32;
    let dh = dst_h as f32;
    let scale = (dw / sw).min(dh / sh);
    let out_w = (sw * scale).round().max(1.0);
    let out_h = (sh * scale).round().max(1.0);
    let x = ((dw - out_w) * 0.5).max(0.0);
    let y = ((dh - out_h) * 0.5).max(0.0);
    (x as u32, y as u32, out_w as u32, out_h as u32)
}
