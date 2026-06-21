// SPDX-License-Identifier: GPL-3.0-or-later
//
// Pad input: keyboard + gamepad polled through SDL3, mapped to the PS2
// DualShock2 button bitmask. The bitmask is ACTIVE-LOW (a cleared bit means
// pressed), matching Kernel/Stubs/Pad.cpp. Stick bytes are 0..255 with 0x80
// centered.

use core::ffi::c_int;
use sdl3_sys::gamepad::*;
use sdl3_sys::joystick::SDL_JoystickID;
use sdl3_sys::keyboard::SDL_GetKeyboardState;
use sdl3_sys::scancode::*;
use sdl3_sys::stdinc::SDL_free;

use crate::PS2PadState;

// PS2 button bits (active-low), matching kPadBtn* in Pad.cpp.
const PAD_SELECT: u16 = 1 << 0;
const PAD_L3: u16 = 1 << 1;
const PAD_R3: u16 = 1 << 2;
const PAD_START: u16 = 1 << 3;
const PAD_UP: u16 = 1 << 4;
const PAD_RIGHT: u16 = 1 << 5;
const PAD_DOWN: u16 = 1 << 6;
const PAD_LEFT: u16 = 1 << 7;
const PAD_L2: u16 = 1 << 8;
const PAD_R2: u16 = 1 << 9;
const PAD_L1: u16 = 1 << 10;
const PAD_R1: u16 = 1 << 11;
const PAD_TRIANGLE: u16 = 1 << 12;
const PAD_CIRCLE: u16 = 1 << 13;
const PAD_CROSS: u16 = 1 << 14;
const PAD_SQUARE: u16 = 1 << 15;

const STICK_CENTER: u8 = 0x80;
const TRIGGER_THRESHOLD: i16 = 8192; // ~25% of i16 range counts as a press

/// Currently-open gamepad for port 0. Reopened lazily as devices appear/leave.
pub struct PadInput {
    gamepad: *mut SDL_Gamepad,
}

// SDL pointers are used only from the thread that owns the Host; the C++ side
// calls pad_read from the guest HLE thread, but the Host wraps this in a mutex.
unsafe impl Send for PadInput {}

impl PadInput {
    pub fn new() -> Self {
        PadInput {
            gamepad: core::ptr::null_mut(),
        }
    }

    /// Ensure `self.gamepad` points at a live, connected gamepad, opening the
    /// first available one if needed.
    fn refresh_gamepad(&mut self) {
        unsafe {
            if !self.gamepad.is_null() && SDL_GamepadConnected(self.gamepad) {
                return;
            }
            if !self.gamepad.is_null() {
                SDL_CloseGamepad(self.gamepad);
                self.gamepad = core::ptr::null_mut();
            }

            let mut count: c_int = 0;
            let ids: *mut SDL_JoystickID = SDL_GetGamepads(&mut count);
            if ids.is_null() {
                return;
            }
            for i in 0..count as isize {
                let id = *ids.offset(i);
                if SDL_IsGamepad(id) {
                    self.gamepad = SDL_OpenGamepad(id);
                    if !self.gamepad.is_null() {
                        break;
                    }
                }
            }
            SDL_free(ids as *mut _);
        }
    }

    /// Poll keyboard + gamepad into a PS2 pad state. Only port 0 is wired; other
    /// ports report disconnected via the return value in lib.rs.
    pub fn read(&mut self) -> PS2PadState {
        let mut buttons: u16 = 0xFFFF;
        let mut lx = STICK_CENTER;
        let mut ly = STICK_CENTER;
        let mut rx = STICK_CENTER;
        let mut ry = STICK_CENTER;

        let clear = |btns: &mut u16, mask: u16| *btns &= !mask;

        self.refresh_gamepad();
        let used_gamepad = !self.gamepad.is_null();
        if used_gamepad {
            let pad = self.gamepad;
            let down = |b: SDL_GamepadButton| unsafe { SDL_GetGamepadButton(pad, b) };

            if down(SDL_GAMEPAD_BUTTON_DPAD_UP) {
                clear(&mut buttons, PAD_UP);
            }
            if down(SDL_GAMEPAD_BUTTON_DPAD_DOWN) {
                clear(&mut buttons, PAD_DOWN);
            }
            if down(SDL_GAMEPAD_BUTTON_DPAD_LEFT) {
                clear(&mut buttons, PAD_LEFT);
            }
            if down(SDL_GAMEPAD_BUTTON_DPAD_RIGHT) {
                clear(&mut buttons, PAD_RIGHT);
            }
            if down(SDL_GAMEPAD_BUTTON_SOUTH) {
                clear(&mut buttons, PAD_CROSS);
            }
            if down(SDL_GAMEPAD_BUTTON_EAST) {
                clear(&mut buttons, PAD_CIRCLE);
            }
            if down(SDL_GAMEPAD_BUTTON_WEST) {
                clear(&mut buttons, PAD_SQUARE);
            }
            if down(SDL_GAMEPAD_BUTTON_NORTH) {
                clear(&mut buttons, PAD_TRIANGLE);
            }
            if down(SDL_GAMEPAD_BUTTON_LEFT_SHOULDER) {
                clear(&mut buttons, PAD_L1);
            }
            if down(SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER) {
                clear(&mut buttons, PAD_R1);
            }
            if down(SDL_GAMEPAD_BUTTON_BACK) {
                clear(&mut buttons, PAD_SELECT);
            }
            if down(SDL_GAMEPAD_BUTTON_START) {
                clear(&mut buttons, PAD_START);
            }
            if down(SDL_GAMEPAD_BUTTON_LEFT_STICK) {
                clear(&mut buttons, PAD_L3);
            }
            if down(SDL_GAMEPAD_BUTTON_RIGHT_STICK) {
                clear(&mut buttons, PAD_R3);
            }

            let axis = |a: SDL_GamepadAxis| unsafe { SDL_GetGamepadAxis(pad, a) };
            if axis(SDL_GAMEPAD_AXIS_LEFT_TRIGGER) > TRIGGER_THRESHOLD {
                clear(&mut buttons, PAD_L2);
            }
            if axis(SDL_GAMEPAD_AXIS_RIGHT_TRIGGER) > TRIGGER_THRESHOLD {
                clear(&mut buttons, PAD_R2);
            }

            lx = axis_to_byte(axis(SDL_GAMEPAD_AXIS_LEFTX));
            ly = axis_to_byte(axis(SDL_GAMEPAD_AXIS_LEFTY));
            rx = axis_to_byte(axis(SDL_GAMEPAD_AXIS_RIGHTX));
            ry = axis_to_byte(axis(SDL_GAMEPAD_AXIS_RIGHTY));
        }

        // Keyboard is always merged so it works alongside or without a gamepad.
        let keys = KeyboardState::snapshot();
        if keys.down(SDL_SCANCODE_UP) || keys.down(SDL_SCANCODE_W) {
            clear(&mut buttons, PAD_UP);
        }
        if keys.down(SDL_SCANCODE_DOWN) || keys.down(SDL_SCANCODE_S) {
            clear(&mut buttons, PAD_DOWN);
        }
        if keys.down(SDL_SCANCODE_LEFT) || keys.down(SDL_SCANCODE_A) {
            clear(&mut buttons, PAD_LEFT);
        }
        if keys.down(SDL_SCANCODE_RIGHT) || keys.down(SDL_SCANCODE_D) {
            clear(&mut buttons, PAD_RIGHT);
        }
        if keys.down(SDL_SCANCODE_X) || keys.down(SDL_SCANCODE_SPACE) {
            clear(&mut buttons, PAD_CROSS);
        }
        if keys.down(SDL_SCANCODE_C) || keys.down(SDL_SCANCODE_ESCAPE) {
            clear(&mut buttons, PAD_CIRCLE);
        }
        if keys.down(SDL_SCANCODE_Z) {
            clear(&mut buttons, PAD_SQUARE);
        }
        if keys.down(SDL_SCANCODE_V) {
            clear(&mut buttons, PAD_TRIANGLE);
        }
        if keys.down(SDL_SCANCODE_Q) {
            clear(&mut buttons, PAD_L1);
        }
        if keys.down(SDL_SCANCODE_E) {
            clear(&mut buttons, PAD_R1);
        }
        if keys.down(SDL_SCANCODE_1) {
            clear(&mut buttons, PAD_L2);
        }
        if keys.down(SDL_SCANCODE_3) {
            clear(&mut buttons, PAD_R2);
        }
        if keys.down(SDL_SCANCODE_RETURN) {
            clear(&mut buttons, PAD_START);
        }
        if keys.down(SDL_SCANCODE_RSHIFT) {
            clear(&mut buttons, PAD_SELECT);
        }
        if keys.down(SDL_SCANCODE_LCTRL) {
            clear(&mut buttons, PAD_L3);
        }
        if keys.down(SDL_SCANCODE_RCTRL) {
            clear(&mut buttons, PAD_R3);
        }

        // WASD doubles as left stick when no gamepad is steering it.
        if !used_gamepad {
            let mut ax = 0i32;
            let mut ay = 0i32;
            if keys.down(SDL_SCANCODE_D) {
                ax += 1;
            }
            if keys.down(SDL_SCANCODE_A) {
                ax -= 1;
            }
            if keys.down(SDL_SCANCODE_S) {
                ay += 1;
            }
            if keys.down(SDL_SCANCODE_W) {
                ay -= 1;
            }
            if ax != 0 || ay != 0 {
                lx = unit_to_byte(ax);
                ly = unit_to_byte(ay);
            }
        }

        PS2PadState {
            buttons,
            lx,
            ly,
            rx,
            ry,
            pressure: [0; 12],
        }
    }
}

impl Drop for PadInput {
    fn drop(&mut self) {
        unsafe {
            if !self.gamepad.is_null() {
                SDL_CloseGamepad(self.gamepad);
            }
        }
    }
}

/// Borrowed view of SDL's keyboard state array, indexed by scancode.
struct KeyboardState {
    state: *const bool,
    len: usize,
}

impl KeyboardState {
    fn snapshot() -> Self {
        let mut numkeys: c_int = 0;
        let state = unsafe { SDL_GetKeyboardState(&mut numkeys) };
        KeyboardState {
            state,
            len: numkeys.max(0) as usize,
        }
    }

    fn down(&self, sc: SDL_Scancode) -> bool {
        if self.state.is_null() {
            return false;
        }
        let idx = sc.0 as usize;
        if idx >= self.len {
            return false;
        }
        unsafe { *self.state.add(idx) }
    }
}

/// Map an SDL i16 stick axis to a 0..255 byte with 0x80 center.
fn axis_to_byte(axis: i16) -> u8 {
    let normalized = (axis as f32) / 32767.0;
    unit_to_byte_f(normalized)
}

fn unit_to_byte(unit: i32) -> u8 {
    unit_to_byte_f(unit as f32)
}

fn unit_to_byte_f(unit: f32) -> u8 {
    let clamped = unit.clamp(-1.0, 1.0);
    let mapped = (clamped + 1.0) * 127.5;
    mapped.round().clamp(0.0, 255.0) as u8
}
