//! A small software rasterizer for the character preview: textured triangles with a depth
//! buffer and one directional light, drawn into an RGBA image. A few thousand triangles at a
//! few hundred pixels don't need the GPU.

use image::RgbaImage;

use super::model::{Mesh, Vertex, normalize};

/// A mesh with the texture and tint it is drawn with.
pub struct Part {
    pub mesh: Mesh,
    pub texture: Option<RgbaImage>,
    /// Multiplies the texture: the game tints white hair textures this way.
    pub tint: [f32; 3],
}

/// Draws `parts` turned `yaw` radians around the vertical axis, fitted to a `width` by
/// `height` image with a transparent background. `smooth` draws at twice the size and averages,
/// for soft edges at four times the cost.
pub fn render(parts: &[Part], yaw: f32, width: u32, height: u32, smooth: bool) -> RgbaImage {
    let s = if smooth { 2 } else { 1 };
    let (w, h) = (width * s, height * s);
    let (min, max) = bounds(parts);
    let center = [
        (min[0] + max[0]) / 2.,
        (min[1] + max[1]) / 2.,
        (min[2] + max[2]) / 2.,
    ];
    // The tallest or widest side fills 90% of the image whatever the turn.
    let reach = ((max[0] - min[0]).max(max[2] - min[2])).max(1e-3);
    let scale = 0.9 * (h as f32 / (max[1] - min[1]).max(1e-3)).min(w as f32 / reach);
    let distance = 4. * (max[1] - min[1]).max(reach);
    let (sin, cos) = yaw.sin_cos();
    let turn = |v: [f32; 3]| [v[0] * cos + v[2] * sin, v[1], -v[0] * sin + v[2] * cos];
    // Left-handed like Direct3D: +Z goes into the screen, so the camera looks down +Z.
    let light = normalize([-0.4, 0.6, -0.7]);

    let mut color = vec![[0u8; 4]; (w * h) as usize];
    let mut depth = vec![f32::INFINITY; (w * h) as usize];
    for part in parts {
        let tint = part.tint;
        for triangle in &part.mesh.triangles {
            // Screen x, y, the view depth, and 1/depth for perspective-correct texturing.
            let corner = |v: &Vertex| {
                let p = turn([
                    v.pos[0] - center[0],
                    v.pos[1] - center[1],
                    v.pos[2] - center[2],
                ]);
                let z = p[2] + distance;
                let k = scale * distance / z;
                (
                    w as f32 / 2. + p[0] * k,
                    h as f32 / 2. - p[1] * k,
                    z,
                    1. / z,
                    turn(v.normal),
                )
            };
            let [a, b, c] = triangle.each_ref().map(corner);
            let area = edge((a.0, a.1), (b.0, b.1), (c.0, c.1));
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = a.0.min(b.0).min(c.0).floor().max(0.) as u32;
            let x1 = (a.0.max(b.0).max(c.0).ceil() as u32).min(w - 1);
            let y0 = a.1.min(b.1).min(c.1).floor().max(0.) as u32;
            let y1 = (a.1.max(b.1).max(c.1).ceil() as u32).min(h - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = (x as f32 + 0.5, y as f32 + 0.5);
                    let wa = edge((b.0, b.1), (c.0, c.1), p) / area;
                    let wb = edge((c.0, c.1), (a.0, a.1), p) / area;
                    let wc = 1. - wa - wb;
                    if wa < 0. || wb < 0. || wc < 0. {
                        continue;
                    }
                    let inv = wa * a.3 + wb * b.3 + wc * c.3;
                    let z = 1. / inv;
                    let at = (y * w + x) as usize;
                    if z >= depth[at] {
                        continue;
                    }
                    let mix = |f: fn(&Vertex) -> f32| {
                        (wa * a.3 * f(&triangle[0])
                            + wb * b.3 * f(&triangle[1])
                            + wc * c.3 * f(&triangle[2]))
                            * z
                    };
                    let texel = part
                        .texture
                        .as_ref()
                        .map_or([255; 4], |t| sample(t, mix(|v| v.uv[0]), mix(|v| v.uv[1])));
                    // Hair cards cut their strands out with alpha.
                    if texel[3] < 128 {
                        continue;
                    }
                    let mut n = normalize([
                        wa * a.4[0] + wb * b.4[0] + wc * c.4[0],
                        wa * a.4[1] + wb * b.4[1] + wc * c.4[1],
                        wa * a.4[2] + wb * b.4[2] + wc * c.4[2],
                    ]);
                    // Both sides of a face are lit as the side the camera sees.
                    if n[2] > 0. {
                        n = [-n[0], -n[1], -n[2]];
                    }
                    let diffuse = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.);
                    let shade = 0.45 + 0.65 * diffuse;
                    let channel =
                        |i: usize| (texel[i] as f32 * tint[i] * shade).clamp(0., 255.) as u8;
                    depth[at] = z;
                    color[at] = [channel(0), channel(1), channel(2), 255];
                }
            }
        }
    }
    if !smooth {
        return RgbaImage::from_raw(w, h, color.concat()).unwrap_or_default();
    }
    RgbaImage::from_fn(width, height, |x, y| {
        let mut sum = [0u32; 4];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let p = color[((y * 2 + dy) * w + x * 2 + dx) as usize];
            // Weighted by alpha so the transparent background doesn't darken edges.
            for i in 0..3 {
                sum[i] += p[i] as u32 * p[3] as u32;
            }
            sum[3] += p[3] as u32;
        }
        if sum[3] == 0 {
            return image::Rgba([0; 4]);
        }
        image::Rgba([
            (sum[0] / sum[3]) as u8,
            (sum[1] / sum[3]) as u8,
            (sum[2] / sum[3]) as u8,
            (sum[3] / 4) as u8,
        ])
    })
}

/// Twice the signed area of `a b p`: which side of the edge `a b` the point `p` is on.
fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// The nearest texel, wrapping; V runs down the image as in Direct3D.
fn sample(texture: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let (tw, th) = texture.dimensions();
    let x = ((u.rem_euclid(1.) * tw as f32) as u32).min(tw - 1);
    let y = ((v.rem_euclid(1.) * th as f32) as u32).min(th - 1);
    texture.get_pixel(x, y).0
}

/// The box around every vertex. The body sets it; hair and beards sit inside.
fn bounds(parts: &[Part]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in parts
        .iter()
        .flat_map(|p| &p.mesh.triangles)
        .flat_map(|t| t.iter())
    {
        for i in 0..3 {
            min[i] = min[i].min(v.pos[i]);
            max[i] = max[i].max(v.pos[i]);
        }
    }
    if min[0] > max[0] {
        return ([0.; 3], [0.; 3]);
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32, y: f32) -> Vertex {
        Vertex {
            pos: [x, y, 0.],
            normal: [0., 0., -1.],
            uv: [0., 0.],
        }
    }

    #[test]
    fn draws_a_triangle_in_front_of_another() {
        let back = Part {
            mesh: Mesh {
                triangles: vec![[vertex(-1., -1.), vertex(0., 1.), vertex(1., -1.)]],
                texture: None,
            },
            texture: None,
            tint: [0., 0., 1.],
        };
        let mut front = Part {
            mesh: back.mesh.clone(),
            texture: None,
            tint: [1., 0., 0.],
        };
        for v in &mut front.mesh.triangles[0] {
            v.pos[2] = -0.5;
        }
        // The blue one is drawn last but sits behind: the depth buffer keeps the red one.
        let parts = [front, back];
        for smooth in [false, true] {
            let image = render(&parts, 0., 40, 40, smooth);
            assert_eq!(image.dimensions(), (40, 40));
            let middle = image.get_pixel(20, 24).0;
            assert!(middle[0] > 200 && middle[2] == 0, "{middle:?}");
            assert_eq!(image.get_pixel(0, 0).0, [0; 4], "background stays clear");
        }
    }
}
