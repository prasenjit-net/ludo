//! Procedural artwork for the board: a wooden table with ivory inlaid tiles,
//! stained yards, a brass-studded frame and soft lighting, painted on the CPU
//! at startup so there are no image files to ship.
//!
//! Shapes are signed distance functions in board units (one unit is one grid
//! cell, the 15x15 board starting at 0,0) and are shaded with a light from the
//! top left.

use bevy::math::Vec2;
use ludo_core::{
    Player,
    board::{GRID, TRACK},
};
use std::f32::consts::TAU;

/// Pixels per grid cell, and the image side: the 15 cells plus half a cell of
/// frame on each side.
const PPC: f32 = 64.0;
pub const SIDE: usize = 16 * PPC as usize;
const MARGIN: f32 = 0.5;

type Rgb = [f32; 3];

/// Direction towards the light, for bevels.
const TO_LIGHT: Vec2 = Vec2::new(-0.6, -0.8);

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn scale(a: Rgb, k: f32) -> Rgb {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---- noise ----------------------------------------------------------------

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h as f32 / u32::MAX as f32
}

fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xi as i32, yi as i32);
    let top = hash(xi, yi, seed) + (hash(xi + 1, yi, seed) - hash(xi, yi, seed)) * sx;
    let bottom =
        hash(xi, yi + 1, seed) + (hash(xi + 1, yi + 1, seed) - hash(xi, yi + 1, seed)) * sx;
    top + (bottom - top) * sy
}

fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise(x * freq, y * freq, seed.wrapping_add(o * 31));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

// ---- materials ------------------------------------------------------------

/// Plank grain: how dark the wood is at `p` (0 = light, 1 = dark).
fn grain(p: Vec2, seed: u32) -> f32 {
    let warp = fbm(p.x * 0.35, p.y * 1.1, seed, 3) * 3.0;
    let ring = (p.y * 2.6 + warp).fract();
    let band = smoothstep(0.0, 0.8, ring) * (1.0 - smoothstep(0.8, 1.0, ring));
    let fine = fbm(p.x * 0.9, p.y * 16.0, seed + 7, 3);
    (0.5 * band + 0.55 * fine).clamp(0.0, 1.0)
}

fn wood(p: Vec2, light: Rgb, dark: Rgb, seed: u32) -> Rgb {
    mix(light, dark, grain(p, seed))
}

/// A colour stained into wood: the grain still shows through.
fn stain(color: Rgb, p: Vec2, seed: u32) -> Rgb {
    let g = grain(p, seed);
    scale(color, 1.12 - 0.34 * g)
}

fn ivory(p: Vec2) -> Rgb {
    let veins = fbm(p.x * 3.0, p.y * 3.0, 91, 3);
    let speck = noise(p.x * 40.0, p.y * 40.0, 5);
    scale([0.96, 0.93, 0.84], 0.93 + 0.06 * veins + 0.02 * speck)
}

const OAK_LIGHT: Rgb = [0.84, 0.64, 0.40];
const OAK_DARK: Rgb = [0.62, 0.42, 0.22];
const WALNUT_LIGHT: Rgb = [0.42, 0.26, 0.15];
const WALNUT_DARK: Rgb = [0.22, 0.12, 0.07];
const BRASS: Rgb = [0.86, 0.66, 0.26];

// ---- shapes ---------------------------------------------------------------

fn circle(p: Vec2, c: Vec2, r: f32) -> f32 {
    (p - c).length() - r
}

fn rbox(p: Vec2, c: Vec2, half: Vec2, r: f32) -> f32 {
    let q = (p - c).abs() - half + Vec2::splat(r);
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - r
}

fn poly(p: Vec2, v: &[Vec2]) -> f32 {
    let mut d = (p - v[0]).length_squared();
    let mut s = 1.0;
    let mut j = v.len() - 1;
    for i in 0..v.len() {
        let e = v[j] - v[i];
        let w = p - v[i];
        let b = w - e * (w.dot(e) / e.length_squared()).clamp(0.0, 1.0);
        d = d.min(b.length_squared());
        let c = [p.y >= v[i].y, p.y < v[j].y, e.x * w.y > e.y * w.x];
        if c.iter().all(|&x| x) || c.iter().all(|&x| !x) {
            s = -s;
        }
        j = i;
    }
    s * d.sqrt()
}

fn star(c: Vec2, points: usize, outer: f32, inner: f32) -> Vec<Vec2> {
    (0..points * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = i as f32 / (points * 2) as f32 * TAU - TAU / 4.0;
            c + Vec2::from_angle(a) * r
        })
        .collect()
}

// ---- canvas ---------------------------------------------------------------

struct Canvas {
    rgb: Vec<Rgb>,
    alpha: Vec<f32>,
}

/// Board position of a pixel centre.
fn board_pos(x: usize, y: usize) -> Vec2 {
    Vec2::new(
        (x as f32 + 0.5) / PPC - MARGIN,
        (y as f32 + 0.5) / PPC - MARGIN,
    )
}

/// Pixel range covering `[min, max]` in board units.
fn pixel_range(min: f32, max: f32) -> (usize, usize) {
    let lo = ((min + MARGIN) * PPC).floor().max(0.0) as usize;
    let hi = (((max + MARGIN) * PPC).ceil() as usize).min(SIDE);
    (lo.min(SIDE), hi)
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            rgb: vec![[0.0; 3]; SIDE * SIDE],
            alpha: vec![0.0; SIDE * SIDE],
        }
    }

    fn over(&mut self, i: usize, color: Rgb, a: f32) {
        // The new layer `a` goes over what is already there.
        let new = a + self.alpha[i] * (1.0 - a);
        if new <= 0.0 {
            return;
        }
        self.rgb[i] = mix(self.rgb[i], color, a / new);
        self.alpha[i] = new;
    }

    /// Fill a shape. `bevel` is the width of the edge shading in cells: positive
    /// for a raised edge, negative for a recessed one.
    fn paint(
        &mut self,
        (min, max): (Vec2, Vec2),
        sdf: impl Fn(Vec2) -> f32,
        fill: impl Fn(Vec2) -> Rgb,
        alpha: f32,
        bevel: f32,
    ) {
        let (x0, x1) = pixel_range(min.x, max.x);
        let (y0, y1) = pixel_range(min.y, max.y);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = board_pos(x, y);
                let d = sdf(p);
                let cover = (0.5 - d * PPC).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let mut color = fill(p);
                let depth = -d;
                let width = bevel.abs();
                if width > 0.0 && depth < width {
                    let e = 0.5 / PPC;
                    let n = Vec2::new(
                        sdf(p + Vec2::X * e) - sdf(p - Vec2::X * e),
                        sdf(p + Vec2::Y * e) - sdf(p - Vec2::Y * e),
                    )
                    .normalize_or_zero();
                    let lit = n.dot(TO_LIGHT) * bevel.signum();
                    let edge = 1.0 - (depth / width).clamp(0.0, 1.0);
                    let k = lit * edge;
                    color = if k >= 0.0 {
                        mix(color, [1.0, 0.97, 0.88], 0.55 * k)
                    } else {
                        scale(color, 1.0 + 0.55 * k)
                    };
                }
                self.over(y * SIDE + x, color, cover * alpha);
            }
        }
    }

    /// A soft shadow under a shape, offset towards the bottom right.
    fn shadow(
        &mut self,
        (min, max): (Vec2, Vec2),
        sdf: impl Fn(Vec2) -> f32,
        offset: Vec2,
        blur: f32,
        strength: f32,
    ) {
        let pad = blur + offset.abs().max_element();
        let (x0, x1) = pixel_range(min.x - pad, max.x + pad);
        let (y0, y1) = pixel_range(min.y - pad, max.y + pad);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = sdf(board_pos(x, y) - offset);
                let a = strength * (1.0 - d / blur).clamp(0.0, 1.0).powi(2);
                if a > 0.0 {
                    self.over(y * SIDE + x, [0.05, 0.03, 0.02], a);
                }
            }
        }
    }
}

fn bounds(c: Vec2, half: Vec2) -> (Vec2, Vec2) {
    (c - half, c + half)
}

fn cell_center(col: i32, row: i32) -> Vec2 {
    Vec2::new(col as f32 + 0.5, row as f32 + 0.5)
}

/// A raised, shadowed rounded tile.
fn tile(canvas: &mut Canvas, c: Vec2, half: f32, radius: f32, fill: impl Fn(Vec2) -> Rgb) {
    let b = bounds(c, Vec2::splat(half));
    let sdf = |p| rbox(p, c, Vec2::splat(half), radius);
    canvas.shadow(b, sdf, Vec2::new(0.03, 0.05), 0.07, 0.5);
    canvas.paint(b, sdf, fill, 1.0, 0.07);
}

/// Direction from one track cell to the next, as a unit vector.
fn track_dir(i: usize) -> Vec2 {
    let (a, b) = (TRACK[i], TRACK[(i + 1) % TRACK.len()]);
    Vec2::new((b.0 - a.0) as f32, (b.1 - a.1) as f32).normalize()
}

fn arrow(c: Vec2, dir: Vec2) -> Vec<Vec2> {
    let side = dir.perp();
    [
        (0.3, 0.0),
        (-0.05, 0.27),
        (-0.05, 0.11),
        (-0.3, 0.11),
        (-0.3, -0.11),
        (-0.05, -0.11),
        (-0.05, -0.27),
    ]
    .iter()
    .map(|&(f, s)| c + dir * f + side * s)
    .collect()
}

fn chevron(c: Vec2, dir: Vec2) -> Vec<Vec2> {
    let side = dir.perp();
    [
        (0.2, 0.0),
        (-0.05, 0.25),
        (-0.2, 0.25),
        (0.05, 0.0),
        (-0.2, -0.25),
        (-0.05, -0.25),
    ]
    .iter()
    .map(|&(f, s)| c + dir * f + side * s)
    .collect()
}

fn engrave(canvas: &mut Canvas, shape: &[Vec2], color: Rgb, alpha: f32) {
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for &v in shape {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    canvas.paint(
        (lo - Vec2::splat(0.05), hi + Vec2::splat(0.05)),
        |p| poly(p, shape),
        |_| color,
        alpha,
        -0.03,
    );
}

/// Paint the whole board. `colors` are the four player colours (sRGB 0..1),
/// in `Player::ALL` order. Returns `SIDE * SIDE` RGBA pixels.
pub fn render(colors: [Rgb; 4]) -> Vec<u8> {
    let mut canvas = Canvas::new();
    let grid = GRID as f32;
    let mid = Vec2::splat(grid / 2.0);

    // Frame: dark walnut with a bevelled outer edge, studded with brass.
    let whole = (Vec2::splat(-MARGIN), Vec2::splat(grid + MARGIN));
    canvas.paint(
        whole,
        |p| rbox(p, mid, Vec2::splat(grid / 2.0 + MARGIN), 0.45),
        |p| wood(Vec2::new(p.y, p.x), WALNUT_LIGHT, WALNUT_DARK, 11),
        1.0,
        0.14,
    );

    // The playing surface: oak, set slightly below the frame.
    let face = (Vec2::ZERO, Vec2::splat(grid));
    canvas.paint(
        face,
        |p| rbox(p, mid, Vec2::splat(grid / 2.0), 0.12),
        |p| wood(p, OAK_LIGHT, OAK_DARK, 23),
        1.0,
        -0.12,
    );
    // A fine inlaid line just inside the edge.
    canvas.paint(
        face,
        |p| (rbox(p, mid, Vec2::splat(grid / 2.0 - 0.2), 0.08)).abs() - 0.012,
        |_| WALNUT_DARK,
        0.7,
        0.0,
    );

    let color_of = |player: Player| colors[player.index()];

    // Yards: stained plate, ivory inset, a diamond inlay and four nests.
    for player in Player::ALL {
        let color = color_of(player);
        let (yc, yr) = player.yard_origin();
        let c = Vec2::new(yc as f32 + 3.0, yr as f32 + 3.0);
        let seed = 40 + player.index() as u32;

        let plate = bounds(c, Vec2::splat(3.0));
        let plate_sdf = |p| rbox(p, c, Vec2::splat(2.95), 0.3);
        canvas.shadow(plate, plate_sdf, Vec2::new(0.05, 0.08), 0.12, 0.5);
        canvas.paint(plate, plate_sdf, |p| stain(color, p, seed), 1.0, 0.1);

        let inset = |p| rbox(p, c, Vec2::splat(2.05), 0.22);
        canvas.paint(plate, inset, ivory, 1.0, -0.1);

        // Diamond inlay linking the nests.
        let diamond = [
            c + Vec2::new(0.0, -1.7),
            c + Vec2::new(1.7, 0.0),
            c + Vec2::new(0.0, 1.7),
            c + Vec2::new(-1.7, 0.0),
        ];
        canvas.paint(
            plate,
            |p| poly(p, &diamond).abs() - 0.025,
            |_| color,
            0.55,
            0.0,
        );
        engrave(&mut canvas, &star(c, 4, 0.5, 0.16), color, 0.7);

        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let n = c + Vec2::new(dx, dy);
            canvas.paint(
                bounds(n, Vec2::splat(0.6)),
                |p| circle(p, n, 0.46),
                |p| scale(stain(color, p, seed + 5), 0.82),
                1.0,
                -0.12,
            );
            canvas.paint(
                bounds(n, Vec2::splat(0.6)),
                |p| circle(p, n, 0.53).abs() - 0.012,
                |_| WALNUT_DARK,
                0.45,
                0.0,
            );
        }
    }

    // The shared track: ivory tiles, coloured start cells, engraved stars.
    for (i, &(col, row)) in TRACK.iter().enumerate() {
        let c = cell_center(col, row);
        if i % 13 == 0 {
            let player = Player::ALL[i / 13];
            let color = color_of(player);
            tile(&mut canvas, c, 0.46, 0.1, |p| {
                stain(color, p, 60 + i as u32)
            });
            engrave(&mut canvas, &arrow(c, track_dir(i)), [1.0, 0.97, 0.88], 0.8);
        } else {
            tile(&mut canvas, c, 0.46, 0.1, ivory);
            if i % 13 == 8 {
                engrave(
                    &mut canvas,
                    &star(c, 5, 0.32, 0.14),
                    [0.72, 0.62, 0.42],
                    1.0,
                );
            }
        }
    }

    // Home columns, tinted, with chevrons towards the centre.
    for player in Player::ALL {
        let color = color_of(player);
        let column = player.home_column();
        for (k, &(col, row)) in column.iter().enumerate() {
            let c = cell_center(col, row);
            tile(&mut canvas, c, 0.46, 0.1, |p| {
                stain(color, p, 70 + player.index() as u32)
            });
            let next = column
                .get(k + 1)
                .map_or(mid, |&(nc, nr)| cell_center(nc, nr));
            let dir = (next - c).normalize();
            engrave(&mut canvas, &chevron(c, dir), [1.0, 0.97, 0.88], 0.6);
        }
    }

    // Centre: one triangle per player meeting at a brass medallion.
    let (tl, tr, br, bl) = (
        Vec2::new(6.0, 6.0),
        Vec2::new(9.0, 6.0),
        Vec2::new(9.0, 9.0),
        Vec2::new(6.0, 9.0),
    );
    let centre = Vec2::new(7.5, 7.5);
    let triangles = [
        (Player::Red, [tl, bl, centre]),
        (Player::Green, [tl, tr, centre]),
        (Player::Yellow, [tr, br, centre]),
        (Player::Blue, [bl, br, centre]),
    ];
    for (player, tri) in triangles {
        let color = color_of(player);
        let b = (Vec2::splat(6.0), Vec2::splat(9.0));
        canvas.paint(
            b,
            |p| poly(p, &tri) + 0.015,
            |p| stain(color, p, 80 + player.index() as u32),
            1.0,
            0.08,
        );
    }
    let medal = bounds(centre, Vec2::splat(0.4));
    canvas.shadow(
        medal,
        |p| circle(p, centre, 0.3),
        Vec2::new(0.02, 0.04),
        0.06,
        0.5,
    );
    canvas.paint(
        medal,
        |p| circle(p, centre, 0.3),
        |p| {
            mix(
                scale(BRASS, 1.1),
                scale(BRASS, 0.7),
                (p - centre).length() / 0.3,
            )
        },
        1.0,
        0.1,
    );
    canvas.paint(
        medal,
        |p| circle(p, centre, 0.15),
        |_| scale(BRASS, 0.8),
        1.0,
        -0.06,
    );

    // Brass studs around the frame.
    let edge = -MARGIN / 2.0;
    let far = grid + MARGIN / 2.0;
    for (x, y) in [
        (edge, edge),
        (far, edge),
        (edge, far),
        (far, far),
        (grid / 2.0, edge),
        (grid / 2.0, far),
        (edge, grid / 2.0),
        (far, grid / 2.0),
    ] {
        let c = Vec2::new(x, y);
        let b = bounds(c, Vec2::splat(0.25));
        canvas.shadow(b, |p| circle(p, c, 0.11), Vec2::new(0.02, 0.03), 0.05, 0.6);
        canvas.paint(
            b,
            |p| circle(p, c, 0.11),
            |p| {
                mix(
                    scale(BRASS, 1.2),
                    scale(BRASS, 0.75),
                    (p - c).length() / 0.11,
                )
            },
            1.0,
            0.07,
        );
    }

    finish(canvas)
}

/// Apply the lighting over everything and convert to 8-bit RGBA.
fn finish(canvas: Canvas) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIDE * SIDE * 4);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let i = y * SIDE + x;
            let p = board_pos(x, y);
            // Warm light from the top left, darker towards the far corner, and
            // a faint varnish sheen across the middle.
            let from_light = (p - Vec2::new(4.0, 2.0)).length() / 22.0;
            let mut k = 1.08 - 0.28 * from_light * from_light;
            let sheen = 1.0 - ((p.x + p.y - 15.0).abs() / 5.0).min(1.0);
            k += 0.04 * sheen * sheen;
            // A little noise hides banding in the gradients.
            let dither = (hash(x as i32, y as i32, 3) - 0.5) / 255.0;
            for channel in canvas.rgb[i] {
                out.push((((channel * k) + dither).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            out.push((canvas.alpha[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    out
}

/// Size in pixels of a pawn image (width x height); it is drawn 0.72 x 0.96 cells.
pub const PAWN_W: usize = 96;
pub const PAWN_H: usize = 128;

/// Radius of the pawn's body at height `h` (units of the image width), for
/// the base, the bell and the collar; the head is a sphere on top.
fn pawn_radius(h: f32) -> f32 {
    if h < 0.0 {
        0.0
    } else if h < 0.14 {
        // Rounded foot.
        let t = (h - 0.07) / 0.07;
        0.44 + 0.05 * (1.0 - t * t).max(0.0).sqrt()
    } else if h < 0.62 {
        let t = 1.0 - (h - 0.14) / 0.48;
        0.15 + 0.29 * t * t
    } else if h < 0.74 {
        0.15 + 0.1 * ((h - 0.62) / 0.12 * std::f32::consts::PI).sin()
    } else {
        0.0
    }
}

const HEAD_H: f32 = 0.93;
const HEAD_R: f32 = 0.23;

/// A lacquered pawn in `color`, lit from the top left, as straight RGBA.
pub fn pawn(color: Rgb) -> Vec<u8> {
    let light = [-0.45f32, 0.65, 0.62];
    let len = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
    let l = [light[0] / len, light[1] / len, light[2] / len];
    let half = [l[0], l[1], l[2] + 1.0];
    let hl = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let half = [half[0] / hl, half[1] / hl, half[2] / hl];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let norm = |v: [f32; 3]| {
        let n = dot(v, v).sqrt().max(1e-6);
        [v[0] / n, v[1] / n, v[2] / n]
    };

    let mut out = vec![0u8; PAWN_W * PAWN_H * 4];
    let px = PAWN_W as f32;
    for y in 0..PAWN_H {
        for x in 0..PAWN_W {
            let u = (x as f32 + 0.5) / px - 0.5;
            // Height above the base, in image widths; the base sits a few pixels up.
            let h = (PAWN_H as f32 - y as f32 - 0.5 - 3.0) / px;

            // Distance from the pawn's outline (negative inside), and its normal.
            let head_d = ((u * u) + (h - HEAD_H) * (h - HEAD_H)).sqrt() - HEAD_R;
            let r = pawn_radius(h);
            let body_d = u.abs() - r;
            let (dist, normal) = if head_d < 0.0 {
                let dh = h - HEAD_H;
                let nz = (HEAD_R * HEAD_R - u * u - dh * dh).max(0.0).sqrt();
                (head_d, norm([u, dh, nz]))
            } else if r > 0.0 && body_d < 0.0 {
                let nx = (u / r).clamp(-1.0, 1.0);
                let nz = (1.0 - nx * nx).max(0.0).sqrt();
                let slope = -(pawn_radius(h + 0.01) - pawn_radius(h - 0.01)) / 0.02;
                (body_d, norm([nx, slope, nz]))
            } else {
                (
                    head_d.min(if r > 0.0 { body_d } else { f32::MAX }),
                    [0.0, 0.0, 1.0],
                )
            };
            let cover = (0.5 - dist * px).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let diffuse = dot(normal, l).max(0.0);
            let spec = dot(normal, half).max(0.0).powi(26);
            let rim = smoothstep(0.55, 1.0, (1.0 - normal[2]).max(0.0));
            let k = (0.32 + 0.8 * diffuse) * (1.0 - 0.4 * rim);
            // A little darker towards the foot, as if the table shadows it.
            let k = k * (0.88 + 0.12 * smoothstep(0.0, 0.4, h));
            let shade = scale(color, k);
            let rgb = mix(shade, [1.0, 1.0, 0.95], 0.6 * spec);
            let i = (y * PAWN_W + x) * 4;
            for c in 0..3 {
                out[i + c] = (rgb[c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = (cover * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> [Rgb; 4] {
        [
            [0.85, 0.20, 0.20],
            [0.20, 0.70, 0.30],
            [0.95, 0.80, 0.20],
            [0.20, 0.40, 0.85],
        ]
    }

    #[test]
    fn board_is_opaque_inside_and_rounded_at_the_corners() {
        let img = render(colors());
        assert_eq!(img.len(), SIDE * SIDE * 4);
        let alpha = |x: usize, y: usize| img[(y * SIDE + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "outer corner is cut away");
        assert_eq!(alpha(SIDE / 2, SIDE / 2), 255);
        assert_eq!(alpha(SIDE / 2, 6), 255, "frame is solid");
    }

    #[test]
    fn pawn_has_a_body_and_clear_corners() {
        let img = pawn(colors()[0]);
        assert_eq!(img.len(), PAWN_W * PAWN_H * 4);
        let alpha = |x: usize, y: usize| img[(y * PAWN_W + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(PAWN_W / 2, PAWN_H / 2), 255);
        assert_eq!(alpha(PAWN_W / 2, PAWN_H - 8), 255, "the foot");
    }

    /// Set `LUDO_DUMP_BOARD=/path/board.rgba` to write the raw pixels out and look at them.
    #[test]
    fn dump_for_inspection() {
        if let Ok(path) = std::env::var("LUDO_DUMP_BOARD") {
            std::fs::write(path, render(colors())).unwrap();
        }
    }
}
