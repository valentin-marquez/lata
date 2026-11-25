//! The game's character models: DirectX `.x` files in text format (`xof 0303txt`). Only what
//! a still preview needs is read: the frame hierarchy and its transforms, and each mesh's
//! vertices, faces, normals, texture coordinates and texture name. Skin weights and animations
//! are skipped, so a skinned mesh comes out in its bind pose.

/// A corner of a triangle, in the model's space (Y up, left-handed like Direct3D).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub triangles: Vec<[Vertex; 3]>,
    /// The texture its first material names; the game usually swaps in its own.
    pub texture: Option<String>,
}

/// Every mesh in `text`, placed by the frames around it. Empty when nothing reads as one.
pub fn parse(text: &str) -> Vec<Mesh> {
    let tokens = tokens(text.split_once('\n').map_or("", |(_, rest)| rest));
    let root = object(&tokens, &mut 0, "", "");
    let mut meshes = Vec::new();
    walk(&root, &root, IDENTITY, &mut meshes);
    meshes
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token<'a> {
    Open,
    Close,
    Word(&'a str),
    Str(&'a str),
    Num(f32),
}

/// Commas and semicolons only separate values, so they are dropped; template GUIDs too.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let stop = |c: u8| c.is_ascii_whitespace() || b",;{}\"<".contains(&c);
    while i < b.len() {
        match b[i] {
            b'{' => out.push(Token::Open),
            b'}' => out.push(Token::Close),
            b'"' => {
                let end = text[i + 1..].find('"').map_or(b.len(), |e| i + 1 + e);
                out.push(Token::Str(&text[i + 1..end]));
                i = end;
            }
            b'<' => i = text[i..].find('>').map_or(b.len(), |e| i + e),
            b'/' | b'#' if b[i] == b'#' || b.get(i + 1) == Some(&b'/') => {
                i = text[i..].find('\n').map_or(b.len(), |e| i + e);
            }
            c if stop(c) => {}
            _ => {
                let start = i;
                while i < b.len() && !stop(b[i]) {
                    i += 1;
                }
                let word = &text[start..i];
                let numeric = word.starts_with(|c: char| c.is_ascii_digit() || c == '-');
                out.push(match word.parse() {
                    Ok(n) if numeric => Token::Num(n),
                    _ => Token::Word(word),
                });
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A data object: its numbers, its strings and references (`{ name }`), and the objects
/// inside it.
#[derive(Debug, Default)]
struct Node<'a> {
    kind: &'a str,
    name: &'a str,
    data: Vec<f32>,
    words: Vec<&'a str>,
    children: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    fn child(&self, kind: &str) -> Option<&Node<'a>> {
        self.children.iter().find(|c| c.kind == kind)
    }
}

/// Reads up to the `}` that closes the object (or the end, for the file itself).
fn object<'a>(t: &[Token<'a>], i: &mut usize, kind: &'a str, name: &'a str) -> Node<'a> {
    let mut node = Node {
        kind,
        name,
        ..Node::default()
    };
    while let Some(token) = t.get(*i) {
        *i += 1;
        match *token {
            Token::Close => break,
            Token::Num(n) => node.data.push(n),
            Token::Str(s) => node.words.push(s),
            Token::Open => node.children.push(object(t, i, "", "")),
            Token::Word(w) => match (t.get(*i), t.get(*i + 1)) {
                (Some(Token::Open), _) => {
                    *i += 1;
                    node.children.push(object(t, i, w, ""));
                }
                (Some(Token::Word(n)), Some(Token::Open)) => {
                    *i += 2;
                    node.children.push(object(t, i, w, n));
                }
                _ => node.words.push(w),
            },
        }
    }
    node
}

/// Row-major, for row vectors (`v * M`), as Direct3D writes them.
type Matrix = [f32; 16];

const IDENTITY: Matrix = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut m = [0.; 16];
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    m
}

fn transform(v: [f32; 3], w: f32, m: &Matrix) -> [f32; 3] {
    let at = |c: usize| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c] + w * m[12 + c];
    [at(0), at(1), at(2)]
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0. {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn walk(node: &Node, root: &Node, parent: Matrix, meshes: &mut Vec<Mesh>) {
    for child in &node.children {
        match child.kind {
            "Frame" => {
                let local = child
                    .child("FrameTransformMatrix")
                    .and_then(|m| m.data.get(..16)?.try_into().ok())
                    .unwrap_or(IDENTITY);
                walk(child, root, multiply(&local, &parent), meshes);
            }
            "Mesh" => meshes.extend(mesh(child, root, &parent)),
            _ => {}
        }
    }
}

/// `n` vectors of `size` floats, then `n` faces (`count; index, index...`), from `at`.
fn faces(d: &[f32], at: &mut usize) -> Option<Vec<Vec<usize>>> {
    let n = *d.get(*at)? as usize;
    *at += 1;
    (0..n)
        .map(|_| {
            let count = *d.get(*at)? as usize;
            let face = d.get(*at + 1..*at + 1 + count)?;
            *at += 1 + count;
            Some(face.iter().map(|&i| i as usize).collect())
        })
        .collect()
}

fn vectors(d: &[f32], at: &mut usize, size: usize) -> Option<Vec<Vec<f32>>> {
    let n = *d.get(*at)? as usize;
    let list = d.get(*at + 1..*at + 1 + n * size)?;
    *at += 1 + n * size;
    Some(list.chunks(size).map(<[f32]>::to_vec).collect())
}

fn mesh(node: &Node, root: &Node, m: &Matrix) -> Option<Mesh> {
    let mut at = 0;
    let positions = vectors(&node.data, &mut at, 3)?;
    let polygons = faces(&node.data, &mut at)?;
    let (normals, normal_faces) = match node.child("MeshNormals") {
        Some(n) => {
            let mut at = 0;
            (vectors(&n.data, &mut at, 3)?, faces(&n.data, &mut at)?)
        }
        None => (Vec::new(), Vec::new()),
    };
    let uvs = match node.child("MeshTextureCoords") {
        Some(t) => vectors(&t.data, &mut 0, 2)?,
        None => Vec::new(),
    };
    let vertex = |face: usize, corner: usize| -> Option<Vertex> {
        let i = *polygons[face].get(corner)?;
        let p = positions.get(i)?;
        let normal = normal_faces
            .get(face)
            .and_then(|f| normals.get(*f.get(corner)?))
            .map_or([0.; 3], |n| normalize(transform([n[0], n[1], n[2]], 0., m)));
        let uv = uvs.get(i).map_or([0.; 2], |uv| [uv[0], uv[1]]);
        Some(Vertex {
            pos: transform([p[0], p[1], p[2]], 1., m),
            normal,
            uv,
        })
    };
    let mut triangles = Vec::new();
    for (f, face) in polygons.iter().enumerate() {
        // A polygon becomes a fan from its first corner.
        for c in 2..face.len() {
            triangles.push([vertex(f, 0)?, vertex(f, c - 1)?, vertex(f, c)?]);
        }
    }
    let material = node.child("MeshMaterialList").and_then(|list| {
        // Inline, or a reference to one defined at the top of the file.
        list.child("Material").or_else(|| {
            let name = list.children.iter().find_map(|c| c.words.first())?;
            root.children
                .iter()
                .find(|c| c.kind == "Material" && c.name == *name)
        })
    });
    let texture = material
        .and_then(|m| m.child("TextureFilename"))
        .and_then(|t| t.words.first())
        .map(|s| s.to_string());
    Some(Mesh { triangles, texture })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cut down from Build 42.20's `MaleBody.x`: the material, a frame inside a rotated frame,
    /// and a mesh of one quad and one triangle (the real one has 617 vertices).
    const SAMPLE: &str = r#"xof 0303txt 0032
template Frame {
 <3d82ab46-62da-11cf-ab39-0020af71e433>
 [...]
}

Material _07_-_Defaultgggg {
 1.000000;1.000000;1.000000;1.000000;;
 100.000000;
 0.200000;0.200000;0.200000;;
 0.000000;0.000000;0.000000;;

 TextureFilename {
  "MaleBody02a.png";
 }
}

Frame Dummy01 {
 FrameTransformMatrix {
  1.000000,0.000000,0.000000,0.000000,0.000000,-0.000000,-1.000000,0.000000,0.000000,1.000000,-0.000000,0.000000,0.000000,0.000000,0.000000,1.000000;;
 }

 Frame Body {
  FrameTransformMatrix {
   1.000000,0.000000,0.000000,0.000000,0.000000,1.000000,0.000000,0.000000,0.000000,0.000000,1.000000,0.000000,0.000000,0.500000,0.000000,1.000000;;
  }

  Mesh Body {
   5;
   0.068560;0.779957;0.011864;,
   0.033278;0.768389;0.006601;,
   0.028068;0.793049;0.029264;,
   0.079726;0.760977;0.003618;,
   0.085701;0.741425;-0.002082;;
   2;
   4;0,1,2,3;,
   3;2,3,4;;

   MeshNormals {
    5;
    -0.098151;0.629525;-0.770756;,
    0.097309;0.636325;-0.765260;,
    0.447259;0.434849;-0.781579;,
    0.077866;0.369274;-0.926053;,
    0.494201;0.075316;-0.866079;;
    2;
    4;0,1,2,3;,
    3;2,3,4;;
   }

   MeshMaterialList {
    1;
    2;
    0,
    0;
    { _07_-_Defaultgggg }
   }

   MeshTextureCoords c1 {
    5;
    0.603976;0.300641;,
    0.536069;0.310339;,
    0.529669;0.295028;,
    0.607193;0.314765;,
    0.606713;0.341151;;
   }

   SkinWeights {
    "Bip01_Spine";
    1;
    5;
    1.000000;
    1.000000,0.000000,0.000000,0.000000,0.000000,1.000000,0.000000,0.000000,0.000000,0.000000,1.000000,0.000000,-0.138405,-0.102098,0.114818,1.000000;;
   }
  }
 }
}
"#;

    #[test]
    fn reads_a_mesh_in_its_frames() {
        let meshes = parse(SAMPLE);
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.texture.as_deref(), Some("MaleBody02a.png"));
        assert_eq!(mesh.triangles.len(), 3, "a quad and a triangle");
        let first = mesh.triangles[0][0];
        // Body lifts it 0.5 in Y, then Dummy01 turns Y into -Z and Z into Y.
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(first.pos[0], 0.068560), "{:?}", first.pos);
        assert!(close(first.pos[1], 0.011864), "{:?}", first.pos);
        assert!(close(first.pos[2], -(0.779957 + 0.5)), "{:?}", first.pos);
        assert_eq!(first.uv, [0.603976, 0.300641]);
        let n = normalize([-0.098151, -0.770756, -0.629525]);
        assert!(first.normal.iter().zip(n).all(|(a, b)| close(*a, b)));
        let last = mesh.triangles[2];
        assert_eq!(last[2].uv, [0.606713, 0.341151]);
    }

    #[test]
    fn skips_what_is_not_a_mesh() {
        assert!(
            parse("xof 0303txt 0032\ntemplate Mesh {\n <3d82ab44>\n DWORD nVertices;\n}\n")
                .is_empty()
        );
        assert!(parse("").is_empty());
    }
}
