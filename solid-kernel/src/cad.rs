//! Backend-neutral sketch contracts shared by the native modeller (`native_brep`) and its callers.
//! Kernel objects and IDs never cross this boundary.

/// One closed loop of a sketch profile, in the plane of the profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Loop {
    Polygon(Vec<[f64; 2]>),
    Circle { centre: [f64; 2], radius: f64 },
}

/// An outer loop and the holes cut out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Loop,
    pub holes: Vec<Loop>,
}
