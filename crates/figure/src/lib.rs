//! The game's characters in 3D, without a GPU: [`model`] reads its DirectX `.x` models and
//! animations, [`pose`] moves skinned meshes with a clip, and [`render`] draws them into an
//! image. Kept apart from the app so dev builds can optimize it: unoptimized, a picture takes
//! about fifteen times longer.

pub mod model;
pub mod pose;
pub mod render;

pub use model::Clip;
pub use pose::pose;
pub use render::{Bounds, Part};
