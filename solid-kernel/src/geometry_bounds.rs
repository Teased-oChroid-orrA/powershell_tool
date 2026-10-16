//! Shared axis-aligned bounds for exact-real geometry on accepted binary64 data.
/// Does not imply regularity, injectivity, trimming or a valid CAD solid.
#[derive(Debug, Clone, Copy)]
pub struct Bounds3 {
    pub min: crate::V3,
    pub max: crate::V3,
}
