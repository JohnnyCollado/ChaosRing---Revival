/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CPU emulation of `GL_OES_matrix_palette` (hardware vertex skinning).
//!
//! OpenGL 2.1 has no matrix palette, so [super::gles1_on_gl2] transforms
//! skinned vertices on the CPU: each vertex is moved into eye space by the
//! weighted sum of its palette matrices, and the result is drawn with an
//! identity modelview matrix. See the
//! [extension spec](https://registry.khronos.org/OpenGL/extensions/OES/OES_matrix_palette.txt).

use super::gles11_raw as gles11; // constants only
use super::gles11_raw::types::{GLenum, GLfloat, GLsizei};
use super::util::fixed_to_float;

/// Value reported for `GL_MAX_PALETTE_MATRICES_OES`. The spec minimum is 9;
/// the CPU path has no real limit, so be generous.
pub const MAX_PALETTE_MATRICES: usize = 32;
/// Value reported for `GL_MAX_VERTEX_UNITS_OES` (spec minimum is 3).
pub const MAX_VERTEX_UNITS: usize = 4;

/// Column-major 4x4 matrix, as used by OpenGL.
pub type Matrix = [GLfloat; 16];

pub const IDENTITY: Matrix = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0, //
];

/// The palette of matrices selected with `glCurrentPaletteMatrixOES` and
/// loaded while `glMatrixMode(GL_MATRIX_PALETTE_OES)` is active.
pub struct Palette {
    pub matrices: [Matrix; MAX_PALETTE_MATRICES],
    pub current: usize,
}
impl Default for Palette {
    fn default() -> Self {
        Palette {
            matrices: [IDENTITY; MAX_PALETTE_MATRICES],
            current: 0,
        }
    }
}
impl Palette {
    pub fn load(&mut self, m: &Matrix) {
        self.matrices[self.current] = *m;
    }
    pub fn load_identity(&mut self) {
        self.matrices[self.current] = IDENTITY;
    }
    /// Post-multiply the current matrix, like `glMultMatrixf`.
    pub fn mult(&mut self, m: &Matrix) {
        let current = &mut self.matrices[self.current];
        *current = mult(current, m);
    }
}

/// `a * b` for column-major matrices.
pub fn mult(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

/// Inverse-transpose of the upper-left 3x3 of `m` (column-major 3x3), used to
/// transform normals. Falls back to the plain 3x3 if it is singular.
fn normal_matrix(m: &Matrix) -> [GLfloat; 9] {
    let [a, b, c] = [m[0], m[1], m[2]];
    let [d, e, f] = [m[4], m[5], m[6]];
    let [g, h, i] = [m[8], m[9], m[10]];
    // Cofactors of the column-major matrix [a d g; b e h; c f i].
    let cof = [
        e * i - f * h,
        f * g - d * i,
        d * h - e * g,
        c * h - b * i,
        a * i - c * g,
        b * g - a * h,
        b * f - c * e,
        c * d - a * f,
        a * e - b * d,
    ];
    let det = a * cof[0] + d * cof[3] + g * cof[6];
    if det.abs() < f32::EPSILON {
        return [a, b, c, d, e, f, g, h, i];
    }
    // inverse = adjugate / det, and the adjugate is the transposed cofactor
    // matrix, so the inverse-transpose is just the cofactors / det.
    cof.map(|x| x / det)
}

/// Layout of one OpenGL client array (or buffer object region).
#[derive(Clone, Copy, Debug)]
pub struct ArrayLayout {
    pub size: usize,
    pub type_: GLenum,
    pub stride: GLsizei,
    /// Signed integer types are mapped to [-1, 1] (normals do this).
    pub normalized: bool,
}

/// Decode elements `first..first + count` of an array into `out`, `size`
/// floats per element.
///
/// # Safety
/// `base` must point to element 0 of an array with this layout that is valid
/// for at least `first + count` elements.
pub unsafe fn decode_array(
    base: *const u8,
    layout: ArrayLayout,
    first: usize,
    count: usize,
    out: &mut Vec<GLfloat>,
) {
    let component_size = match layout.type_ {
        gles11::BYTE | gles11::UNSIGNED_BYTE => 1,
        gles11::SHORT => 2,
        gles11::FLOAT | gles11::FIXED => 4,
        other => panic!("Unsupported matrix palette array type {other:#x}"),
    };
    let stride = if layout.stride == 0 {
        layout.size * component_size
    } else {
        usize::try_from(layout.stride).unwrap()
    };
    out.clear();
    out.reserve(count * layout.size);
    for j in first..first + count {
        let element = base.add(j * stride);
        for k in 0..layout.size {
            let p = element.add(k * component_size);
            let value = match layout.type_ {
                gles11::BYTE => {
                    let v = p.cast::<i8>().read_unaligned() as f32;
                    if layout.normalized {
                        (2.0 * v + 1.0) / 255.0
                    } else {
                        v
                    }
                }
                gles11::UNSIGNED_BYTE => p.read() as f32,
                gles11::SHORT => {
                    let v = p.cast::<i16>().read_unaligned() as f32;
                    if layout.normalized {
                        (2.0 * v + 1.0) / 65535.0
                    } else {
                        v
                    }
                }
                gles11::FLOAT => p.cast::<f32>().read_unaligned(),
                gles11::FIXED => fixed_to_float(p.cast::<i32>().read_unaligned()),
                _ => unreachable!(),
            };
            out.push(value);
        }
    }
}

/// Decoded per-vertex inputs for [skin], all covering the same vertices.
pub struct SkinInput<'a> {
    /// Object-space positions, `position_size` (2, 3 or 4) floats each.
    pub positions: &'a [GLfloat],
    pub position_size: usize,
    /// Object-space normals, 3 floats each, if the normal array is enabled.
    pub normals: Option<&'a [GLfloat]>,
    /// Palette indices, `units` per vertex.
    pub matrix_indices: &'a [GLfloat],
    /// Blend weights, `units` per vertex.
    pub weights: &'a [GLfloat],
    pub units: usize,
}

/// Transform vertices into eye space with the matrix palette. Positions are
/// written as 4 floats per vertex and normals as 3.
pub fn skin(
    palette: &[Matrix],
    input: &SkinInput,
    out_positions: &mut Vec<GLfloat>,
    out_normals: &mut Vec<GLfloat>,
) {
    let normal_matrices: Vec<[GLfloat; 9]> = if input.normals.is_some() {
        palette.iter().map(normal_matrix).collect()
    } else {
        Vec::new()
    };
    let vertex_count = input.positions.len() / input.position_size;
    out_positions.clear();
    out_normals.clear();
    for v in 0..vertex_count {
        let p = &input.positions[v * input.position_size..][..input.position_size];
        let pos = [
            p[0],
            p[1],
            p.get(2).copied().unwrap_or(0.0),
            p.get(3).copied().unwrap_or(1.0),
        ];
        let mut out_pos = [0.0; 4];
        let mut out_norm = [0.0; 3];
        for unit in 0..input.units {
            let weight = input.weights[v * input.units + unit];
            let index = input.matrix_indices[v * input.units + unit] as usize;
            let Some(m) = palette.get(index) else {
                // Out-of-range indices are undefined behavior in the spec.
                continue;
            };
            for (row, out) in out_pos.iter_mut().enumerate() {
                let transformed: GLfloat = (0..4).map(|k| m[k * 4 + row] * pos[k]).sum();
                *out += weight * transformed;
            }
            if let Some(normals) = input.normals {
                let n = &normals[v * 3..][..3];
                let nm = &normal_matrices[index];
                for (row, out) in out_norm.iter_mut().enumerate() {
                    let transformed: GLfloat = (0..3).map(|k| nm[k * 3 + row] * n[k]).sum();
                    *out += weight * transformed;
                }
            }
        }
        out_positions.extend_from_slice(&out_pos);
        if input.normals.is_some() {
            out_normals.extend_from_slice(&out_norm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = IDENTITY;
        m[12] = x;
        m[13] = y;
        m[14] = z;
        m
    }

    fn scale(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = IDENTITY;
        m[0] = x;
        m[5] = y;
        m[10] = z;
        m
    }

    #[test]
    fn mult_composes_like_gl() {
        // glLoadMatrix(T); glMultMatrix(S) transforms by T * S: scale first.
        let m = mult(&translation(1.0, 0.0, 0.0), &scale(2.0, 2.0, 2.0));
        assert_eq!(m, {
            let mut e = scale(2.0, 2.0, 2.0);
            e[12] = 1.0;
            e
        });
        assert_eq!(mult(&IDENTITY, &m), m);
    }

    #[test]
    fn single_bone_matches_matrix() {
        let palette = [IDENTITY, translation(10.0, 0.0, 0.0)];
        let input = SkinInput {
            positions: &[1.0, 2.0, 3.0],
            position_size: 3,
            normals: Some(&[0.0, 1.0, 0.0]),
            matrix_indices: &[1.0],
            weights: &[1.0],
            units: 1,
        };
        let (mut pos, mut norm) = (Vec::new(), Vec::new());
        skin(&palette, &input, &mut pos, &mut norm);
        assert_eq!(pos, [11.0, 2.0, 3.0, 1.0]);
        // Translation doesn't affect normals.
        assert_eq!(norm, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn weights_blend_bones() {
        let palette = [translation(-2.0, 0.0, 0.0), translation(2.0, 4.0, 0.0)];
        let input = SkinInput {
            positions: &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            position_size: 3,
            normals: None,
            matrix_indices: &[0.0, 1.0, 1.0, 1.0],
            weights: &[0.5, 0.5, 0.25, 0.75],
            units: 2,
        };
        let (mut pos, mut norm) = (Vec::new(), Vec::new());
        skin(&palette, &input, &mut pos, &mut norm);
        assert_eq!(pos, [0.0, 2.0, 0.0, 1.0, 3.0, 5.0, 1.0, 1.0]);
        assert!(norm.is_empty());
    }

    #[test]
    fn normals_use_inverse_transpose() {
        // Non-uniform scale: a normal must scale by the inverse.
        let palette = [scale(2.0, 1.0, 1.0)];
        let input = SkinInput {
            positions: &[0.0, 0.0, 0.0],
            position_size: 3,
            normals: Some(&[1.0, 1.0, 0.0]),
            matrix_indices: &[0.0],
            weights: &[1.0],
            units: 1,
        };
        let (mut pos, mut norm) = (Vec::new(), Vec::new());
        skin(&palette, &input, &mut pos, &mut norm);
        assert_eq!(norm, [0.5, 1.0, 0.0]);
    }

    #[test]
    fn out_of_range_index_is_skipped() {
        let palette = [IDENTITY];
        let input = SkinInput {
            positions: &[1.0, 1.0, 1.0],
            position_size: 3,
            normals: None,
            matrix_indices: &[0.0, 200.0],
            weights: &[1.0, 1.0],
            units: 2,
        };
        let (mut pos, mut norm) = (Vec::new(), Vec::new());
        skin(&palette, &input, &mut pos, &mut norm);
        assert_eq!(pos, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn decode_strided_and_typed_arrays() {
        // Interleaved: 3 floats then 2 padding bytes per element.
        let mut bytes = Vec::new();
        for v in [[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]] {
            for c in v {
                bytes.extend_from_slice(&c.to_ne_bytes());
            }
            bytes.extend_from_slice(&[0xAA, 0xBB]);
        }
        let layout = ArrayLayout {
            size: 3,
            type_: gles11::FLOAT,
            stride: 14,
            normalized: false,
        };
        let mut out = Vec::new();
        unsafe { decode_array(bytes.as_ptr(), layout, 1, 1, &mut out) };
        assert_eq!(out, [4.0, 5.0, 6.0]);

        let indices = [3u8, 7, 1, 0];
        let layout = ArrayLayout {
            size: 2,
            type_: gles11::UNSIGNED_BYTE,
            stride: 0,
            normalized: false,
        };
        unsafe { decode_array(indices.as_ptr(), layout, 0, 2, &mut out) };
        assert_eq!(out, [3.0, 7.0, 1.0, 0.0]);

        let fixed = [0x10000i32, -0x8000];
        let layout = ArrayLayout {
            size: 2,
            type_: gles11::FIXED,
            stride: 0,
            normalized: false,
        };
        unsafe { decode_array(fixed.as_ptr().cast(), layout, 0, 1, &mut out) };
        assert_eq!(out, [1.0, -0.5]);
    }
}
