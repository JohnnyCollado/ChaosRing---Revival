/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Utilities for presenting frames to the window using an abstract OpenGL ES
//! implementation.

use super::gles11_raw as gles11; // constants and types only
use super::GLES;
use crate::matrix::Matrix;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

// RPG-style cursor sprites baked into the binary, used to render the
// right-stick virtual cursor (replaces the earlier colored-square dot).
// The "Pointer" hand is shown while the cursor is just hovering / moving;
// "Select" (the thumb-down hand) is shown while the click button is held
// so the user has unambiguous visual feedback for a tap.
const CURSOR_POINTER_PNG: &[u8] = include_bytes!("../../res/RPG Cursors/Cursor Pointer.png");
const CURSOR_SELECT_PNG: &[u8] = include_bytes!("../../res/RPG Cursors/Cursor Select.png");

/// Decoded RGBA pixels + dimensions for a cursor sprite. Decoded once on
/// first use (cheap PNG, tiny pixel-art file), cached for the lifetime of
/// the process.
struct CursorPixels {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

static CURSOR_POINTER_PIXELS: OnceLock<CursorPixels> = OnceLock::new();
static CURSOR_SELECT_PIXELS: OnceLock<CursorPixels> = OnceLock::new();
/// GL texture name for the cursor. 0 = not yet uploaded into the current
/// GL context. We lazy-upload on the first frame after a context is
/// available. Reset to 0 by the consumer if a context is destroyed (not
/// currently exercised — Chaos Rings keeps one context alive throughout).
static CURSOR_POINTER_TEX: AtomicU32 = AtomicU32::new(0);
static CURSOR_SELECT_TEX: AtomicU32 = AtomicU32::new(0);

fn cursor_pixels(slot: &'static OnceLock<CursorPixels>, png: &[u8]) -> &'static CursorPixels {
    slot.get_or_init(|| {
        let img = crate::image::Image::from_bytes(png).expect("baked-in cursor PNG should decode");
        let (w, h) = img.dimensions();
        CursorPixels {
            pixels: img.pixels().to_vec(),
            width: w,
            height: h,
        }
    })
}

unsafe fn cursor_texture(
    gles: &mut dyn GLES,
    slot: &'static OnceLock<CursorPixels>,
    tex_slot: &'static AtomicU32,
    png: &[u8],
) -> gles11::types::GLuint {
    let existing = tex_slot.load(Ordering::Relaxed);
    if existing != 0 {
        return existing;
    }
    let cp = cursor_pixels(slot, png);
    let mut name: gles11::types::GLuint = 0;
    gles.GenTextures(1, &mut name);
    gles.BindTexture(gles11::TEXTURE_2D, name);
    gles.TexImage2D(
        gles11::TEXTURE_2D,
        0,
        gles11::RGBA as _,
        cp.width as _,
        cp.height as _,
        0,
        gles11::RGBA,
        gles11::UNSIGNED_BYTE,
        cp.pixels.as_ptr() as *const _,
    );
    // Pixel-art sprites: NEAREST keeps the chunky-pixel look instead of
    // smearing the design into a blur when scaled up.
    gles.TexParameteri(
        gles11::TEXTURE_2D,
        gles11::TEXTURE_MIN_FILTER,
        gles11::NEAREST as _,
    );
    gles.TexParameteri(
        gles11::TEXTURE_2D,
        gles11::TEXTURE_MAG_FILTER,
        gles11::NEAREST as _,
    );
    gles.TexParameteri(
        gles11::TEXTURE_2D,
        gles11::TEXTURE_WRAP_S,
        gles11::CLAMP_TO_EDGE as _,
    );
    gles.TexParameteri(
        gles11::TEXTURE_2D,
        gles11::TEXTURE_WRAP_T,
        gles11::CLAMP_TO_EDGE as _,
    );
    tex_slot.store(name, Ordering::Relaxed);
    name
}

pub struct FpsCounter {
    time: std::time::Instant,
    frames: u32,
}
impl FpsCounter {
    pub fn start() -> Self {
        FpsCounter {
            time: Instant::now(),
            frames: 0,
        }
    }

    pub fn count_frame(&mut self, label: std::fmt::Arguments<'_>) {
        self.frames += 1;
        let now = Instant::now();
        let duration = now - self.time;
        if duration >= Duration::from_secs(1) {
            self.time = now;
            echo!(
                "touchHLE: {} FPS: {:.2}",
                label,
                std::mem::take(&mut self.frames) as f32 / duration.as_secs_f32()
            );
        }
    }
}

/// Present the the latest frame (e.g. the app's splash screen or rendering
/// output), provided as a texture bound to `GL_TEXTURE_2D`, by drawing it on
/// the window. It may be rotated, scaled and/or letterboxed as necessary. The
/// virtual cursor is also drawn if it should be currently visible.
///
/// The provided context must be current.
pub unsafe fn present_frame(
    gles: &mut dyn GLES,
    viewport: (u32, u32, u32, u32),
    rotation_matrix: Matrix<2>,
    virtual_cursor_visible_at: Option<(f32, f32, bool)>,
) {
    // While this is a generic utility, it is closely tied to
    // crate::frameworks::opengles::eagl::present_renderbuffer, which handles
    // backing up and restoring OpenGL ES state that this function might touch,
    // so these need to be updated in tandem.

    use gles11::types::*;

    // Draw the quad
    gles.Viewport(
        viewport.0 as _,
        viewport.1 as _,
        viewport.2 as _,
        viewport.3 as _,
    );
    gles.ClearColor(0.0, 0.0, 0.0, 1.0);
    gles.Clear(gles11::COLOR_BUFFER_BIT | gles11::DEPTH_BUFFER_BIT | gles11::STENCIL_BUFFER_BIT);
    gles.BindBuffer(gles11::ARRAY_BUFFER, 0);
    let vertices: [f32; 12] = [
        -1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0,
    ];
    gles.EnableClientState(gles11::VERTEX_ARRAY);
    gles.VertexPointer(2, gles11::FLOAT, 0, vertices.as_ptr() as *const GLvoid);
    let tex_coords: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    gles.EnableClientState(gles11::TEXTURE_COORD_ARRAY);
    gles.TexCoordPointer(2, gles11::FLOAT, 0, tex_coords.as_ptr() as *const GLvoid);
    let matrix = Matrix::<4>::from(&rotation_matrix);
    gles.MatrixMode(gles11::TEXTURE);
    gles.LoadMatrixf(matrix.columns().as_ptr() as *const _);
    gles.Enable(gles11::TEXTURE_2D);
    gles.DrawArrays(gles11::TRIANGLES, 0, 6);
    // clean this up so we don't need to worry about it in e.g. Core Animation
    gles.LoadIdentity();

    // Display virtual cursor using the RPG-style pixel-art sprite
    // (res/RPG Cursors/Cursor Pointer.png while moving; Cursor Select.png
    // while the click button is held). NEAREST filtering preserves the
    // chunky pixel look at any scale.
    if let Some((x, y, pressed)) = virtual_cursor_visible_at {
        let (vx, vy, vw, vh) = viewport;
        let x = x - vx as f32;
        let y = y - vy as f32;

        let tex = cursor_texture(
            gles,
            if pressed {
                &CURSOR_SELECT_PIXELS
            } else {
                &CURSOR_POINTER_PIXELS
            },
            if pressed {
                &CURSOR_SELECT_TEX
            } else {
                &CURSOR_POINTER_TEX
            },
            if pressed {
                CURSOR_SELECT_PNG
            } else {
                CURSOR_POINTER_PNG
            },
        );
        gles.BindTexture(gles11::TEXTURE_2D, tex);
        gles.Enable(gles11::TEXTURE_2D);
        gles.EnableClientState(gles11::TEXTURE_COORD_ARRAY);

        // Pixels are premultiplied by Image::from_bytes (see image.rs),
        // so we blend with (ONE, ONE_MINUS_SRC_ALPHA) — straight-alpha
        // blending would darken anti-aliased edges.
        gles.Enable(gles11::BLEND);
        gles.BlendFunc(gles11::ONE, gles11::ONE_MINUS_SRC_ALPHA);
        gles.Color4f(1.0, 1.0, 1.0, 1.0);

        // 30px half-size square => the sprite renders at 60x60 on screen,
        // which is roughly the apparent size of a mouse cursor on a 1080p
        // monitor — big enough to spot, small enough not to dominate.
        // Hotspot is the visual "tip" of the hand sprite; for these
        // particular cursors the tip is at the top, so we offset the
        // quad downward by half its size so (x, y) corresponds to the
        // sprite's TOP rather than its center.
        let radius: f32 = 30.0;
        let cx = x;
        let cy = y + radius; // shift sprite down so (x, y) is the tip

        let mut v = vertices;
        for i in (0..v.len()).step_by(2) {
            v[i] = (v[i] * radius + cx) / (vw as f32 / 2.0) - 1.0;
            v[i + 1] = 1.0 - (v[i + 1] * radius + cy) / (vh as f32 / 2.0);
        }
        gles.VertexPointer(2, gles11::FLOAT, 0, v.as_ptr() as *const GLvoid);
        // Map the sprite atlas 0..1 onto the quad (full sprite, not flipped).
        let cursor_tc: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        gles.TexCoordPointer(2, gles11::FLOAT, 0, cursor_tc.as_ptr() as *const GLvoid);
        gles.DrawArrays(gles11::TRIANGLES, 0, 6);
    }
}
