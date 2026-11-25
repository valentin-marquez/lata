//! Moves skinned meshes with a clip: each joint's transform at a moment, chained down the
//! skeleton, and every vertex carried by the bones that weigh on it.

use crate::model::{Clip, IDENTITY, Matrix, Mesh, Track, multiply, normalize, transform};

/// Sets the triangles of `mesh` to its pose `time` seconds into `clip`. Joints the clip
/// doesn't move keep the transform their file saved; a mesh without skin stays as it is.
pub fn pose(mesh: &mut Mesh, clip: &Clip, time: f32) {
    let Mesh {
        triangles, skin, ..
    } = mesh;
    let Some(skin) = skin else {
        return;
    };
    let mut world: Vec<Matrix> = Vec::with_capacity(skin.joints.len());
    for joint in &skin.joints {
        let local = clip
            .tracks
            .get(&joint.name)
            .map_or(joint.local, |track| sample(track, time, &joint.local));
        let parent = joint.parent.map_or(IDENTITY, |p| world[p]);
        world.push(multiply(&local, &parent));
    }
    let bones: Vec<Matrix> = skin
        .bones
        .iter()
        .map(|(joint, offset)| multiply(offset, &world[*joint]))
        .collect();
    // One blended matrix per vertex; a vertex no bone holds keeps where the file put it.
    let blended: Vec<Option<Matrix>> = skin
        .weights
        .iter()
        .map(|list| {
            let total: f32 = list.iter().map(|(_, w)| w).sum();
            (total > 1e-6).then(|| {
                let mut m = [0.; 16];
                for (bone, w) in list {
                    for (out, b) in m.iter_mut().zip(&bones[*bone]) {
                        *out += b * w / total;
                    }
                }
                m
            })
        })
        .collect();
    for ((out, rest), corners) in triangles.iter_mut().zip(&skin.rest).zip(&skin.corners) {
        for k in 0..3 {
            // ponytail: normals go through the bone matrix as is, right while the clips scale
            // uniformly (the game's all keep scale 1); the inverse transpose if one doesn't.
            if let Some(m) = blended.get(corners[k]).copied().flatten() {
                out[k].pos = transform(rest[k].pos, 1., &m);
                out[k].normal = normalize(transform(rest[k].normal, 0., &m));
            }
        }
    }
}

/// The joint's transform at `time`: scale, then rotation, then position, for row vectors.
/// What the track has no keys for comes from `saved`, the file's own transform (a missing
/// scale is taken as 1).
fn sample(track: &Track, time: f32, saved: &Matrix) -> Matrix {
    let [px, py, pz] = lerp(&track.position, time, [saved[12], saved[13], saved[14]]);
    if track.rotation.is_empty() && track.scale.is_empty() {
        let mut m = *saved;
        m[12..15].copy_from_slice(&[px, py, pz]);
        return m;
    }
    let [sx, sy, sz] = lerp(&track.scale, time, [1.; 3]);
    let [w, x, y, z] = rotation(&track.rotation, time);
    // The game's files store the conjugate of what Direct3D's `D3DXMatrixRotationQuaternion`
    // expects (checked against their frames), so this is that matrix with x, y, z negated.
    let r = [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ],
    ];
    let row = |i: usize, s: f32| [r[i][0] * s, r[i][1] * s, r[i][2] * s, 0.];
    let [a, b, c] = [row(0, sx), row(1, sy), row(2, sz)];
    [
        a[0], a[1], a[2], 0., b[0], b[1], b[2], 0., c[0], c[1], c[2], 0., px, py, pz, 1.,
    ]
}

/// The keys around `time` and how far between them it is.
fn around<T: Copy>(keys: &[(f32, T)], time: f32) -> Option<(T, T, f32)> {
    let after = keys.iter().position(|(t, _)| *t > time);
    match after {
        None => keys.last().map(|(_, v)| (*v, *v, 0.)),
        Some(0) => Some((keys[0].1, keys[0].1, 0.)),
        Some(i) => {
            let ((t0, a), (t1, b)) = (keys[i - 1], keys[i]);
            Some((a, b, (time - t0) / (t1 - t0)))
        }
    }
}

fn lerp(keys: &[(f32, [f32; 3])], time: f32, none: [f32; 3]) -> [f32; 3] {
    around(keys, time).map_or(none, |(a, b, f)| {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
    })
}

// ponytail: normalized lerp, not slerp; the clips key every 1/30 s, where the two don't differ
// visibly.
fn rotation(keys: &[(f32, [f32; 4])], time: f32) -> [f32; 4] {
    let Some((a, mut b, f)) = around(keys, time) else {
        return [1., 0., 0., 0.];
    };
    // The short way round.
    if (0..4).map(|i| a[i] * b[i]).sum::<f32>() < 0. {
        b = b.map(|v| -v);
    }
    let q: [f32; 4] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
    let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if len == 0. {
        return [1., 0., 0., 0.];
    }
    q.map(|v| v / len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{clip, parse};

    const MEDIA: &str =
        "/mnt/datos/dev/lata/research/demo/home/builds/2978601190/projectzomboid/media";

    /// A file of the B42 build in the demo library; `None` (and the test passes) without it.
    fn read(rel: &str) -> Option<String> {
        std::fs::read_to_string(std::path::Path::new(MEDIA).join(rel)).ok()
    }

    #[test]
    fn a_track_keeps_what_it_has_no_keys_for() {
        // Turned a quarter around Z and 2 along X; the track only moves it along Y.
        let saved = [
            0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 2., 0., 0., 1.,
        ];
        let track = Track {
            position: vec![(0., [2., 0., 0.]), (1., [2., 4., 0.])],
            ..Track::default()
        };
        let m = sample(&track, 0.5, &saved);
        assert_eq!(&m[..12], &saved[..12], "the turn stays");
        assert_eq!(&m[12..15], &[2., 2., 0.]);
        let turned = sample(
            &Track {
                rotation: vec![(0., [1., 0., 0., 0.])],
                ..Track::default()
            },
            0.,
            &saved,
        );
        assert_eq!(&turned[12..15], &[2., 0., 0.], "the place stays");
    }

    /// The hair, skinned in its own file, stays on the body's head through the walk. The
    /// files' frames hold a walk pose, not the bind pose the offsets were taken in, so this
    /// checks the offsets and the keys together.
    #[test]
    fn the_hair_follows_the_head() {
        let (Some(body), Some(hair), Some(walk)) = (
            read("models_X/Skinned/MaleBody.x"),
            read("models_X/Skinned/Hair/Bob_Hair_Donny.x"),
            read("anims_X/Bob/Bob_Walk.x"),
        ) else {
            return;
        };
        let walk = clip(&walk).unwrap();
        let mut body = parse(&body).into_iter().find(|m| m.skin.is_some()).unwrap();
        let skin = body.skin.as_ref().unwrap();
        assert!(skin.bones.len() > 20, "{} bones", skin.bones.len());
        let held = skin.weights.iter().filter(|w| !w.is_empty()).count();
        assert_eq!(held, skin.weights.len(), "every vertex has a bone");
        let mut hair = parse(&hair);
        for t in [0., walk.length / 3., walk.length / 2.] {
            pose(&mut body, &walk, t);
            let top = body
                .triangles
                .iter()
                .flatten()
                .fold(f32::MIN, |m, v| m.max(v.pos[1]));
            let mut sum = [0.; 3];
            let mut n = 0.;
            for mesh in &mut hair {
                pose(mesh, &walk, t);
                for v in mesh.triangles.iter().flatten() {
                    (0..3).for_each(|i| sum[i] += v.pos[i]);
                    n += 1.;
                }
            }
            let y = sum[1] / n;
            assert!(
                top - y < 0.15 && y < top + 0.05,
                "hair at {y}, head top {top}"
            );
        }
    }

    #[test]
    fn the_walk_matches_the_file_frames_and_loops() {
        let (Some(body), Some(walk)) = (
            read("models_X/Skinned/MaleBody.x"),
            read("anims_X/Bob/Bob_Walk.x"),
        ) else {
            return;
        };
        let walk = clip(&walk).unwrap();
        assert!(
            (walk.length - 3200. / 4800.).abs() < 1e-4,
            "{}",
            walk.length
        );
        assert_eq!(walk.tracks.len(), 35);
        let meshes = parse(&body);
        let skin = meshes.iter().find_map(|m| m.skin.as_ref()).unwrap();
        // The rotation and position keys rebuild the frame the model file saved for the hips.
        let hips = skin.joints.iter().find(|j| j.name == "Bip01").unwrap();
        let m = sample(&walk.tracks["Bip01"], 0., &IDENTITY);
        assert!(
            m.iter().zip(hips.local).all(|(a, b)| (a - b).abs() < 1e-3),
            "{m:?}"
        );

        let mut mesh = meshes.into_iter().find(|m| m.skin.is_some()).unwrap();
        let mut at = |t: f32| {
            pose(&mut mesh, &walk, t);
            mesh.triangles.clone()
        };
        let (start, half, end) = (at(0.), at(walk.length / 2.), at(walk.length));
        type Tris = [[crate::model::Vertex; 3]];
        let moved = |a: &Tris, b: &Tris| {
            a.iter()
                .flatten()
                .zip(b.iter().flatten())
                .map(|(a, b)| {
                    (0..3)
                        .map(|i| (a.pos[i] - b.pos[i]).abs())
                        .fold(0., f32::max)
                })
                .fold(0., f32::max)
        };
        assert!(
            moved(&start, &half) > 0.1,
            "half a stride later the legs swapped"
        );
        assert!(
            moved(&start, &end) < 1e-3,
            "the last key is the first again"
        );
    }
}
