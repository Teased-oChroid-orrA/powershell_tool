//! Vessel geometry, validation, and the thin-wall/thick-wall
//! classification issue #11 asks for - **for engineering interpretation
//! only**. "The geometry classification shall not cause the authoritative
//! stress calculation to switch to a simplified thin-wall stress
//! equation" - `classify` never gates which stress formula
//! `crate::stress` runs; both classifications use the exact same full
//! Lamé solution.

/// A cylindrical vessel's radial geometry. `inner_radius`/`outer_radius`,
/// not diameter or thickness, because [`mechanics_core::lame`]'s own
/// functions all take radii - storing what the physics actually consumes
/// avoids a diameter/radius mixup at every call site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CylinderGeometry {
    pub inner_radius: f64,
    pub outer_radius: f64,
}

/// Why a [`CylinderGeometry`] was rejected - issue #11's own "Geometry
/// validation shall detect: impossible dimensions; zero or negative wall
/// thickness; contradictory inputs."
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeometryError {
    NonPositiveInnerRadius,
    OuterNotGreaterThanInner,
}

impl CylinderGeometry {
    pub fn new(inner_radius: f64, outer_radius: f64) -> Result<Self, GeometryError> {
        if !(inner_radius > 0.0) {
            return Err(GeometryError::NonPositiveInnerRadius);
        }
        if !(outer_radius > inner_radius) {
            return Err(GeometryError::OuterNotGreaterThanInner);
        }
        Ok(Self { inner_radius, outer_radius })
    }

    pub fn wall_thickness(&self) -> f64 {
        self.outer_radius - self.inner_radius
    }

    pub fn mean_radius(&self) -> f64 {
        (self.inner_radius + self.outer_radius) / 2.0
    }
}

/// Engineering-interpretation-only geometry classification - never feeds
/// back into which stress formula gets used (see this module's own doc
/// comment).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeometryClassification {
    ThinWall,
    ThickWall,
}

/// Classifies wall thickness relative to inner radius using the
/// criterion stated in Shigley's *Mechanical Engineering Design*
/// (thick-wall cylinder chapter): thin-wall theory is considered
/// acceptable when the wall thickness is no more than 1/10 of the inside
/// radius (`t / r_i <= 0.1`); thicker walls are classified thick-wall.
/// This single-threshold criterion is used verbatim rather than inventing
/// a "transition" band with no cited basis - issue #11 itself warns
/// "Thresholds shall not be treated as universal physical boundaries,"
/// and a fabricated transition zone would be exactly that.
pub fn classify(geometry: &CylinderGeometry) -> GeometryClassification {
    let ratio = geometry.wall_thickness() / geometry.inner_radius;
    if ratio <= 0.1 {
        GeometryClassification::ThinWall
    } else {
        GeometryClassification::ThickWall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_geometry_is_accepted() {
        let g = CylinderGeometry::new(2.0, 3.0).unwrap();
        assert_eq!(g.wall_thickness(), 1.0);
        assert_eq!(g.mean_radius(), 2.5);
    }

    #[test]
    fn zero_or_negative_inner_radius_is_rejected() {
        assert_eq!(CylinderGeometry::new(0.0, 3.0), Err(GeometryError::NonPositiveInnerRadius));
        assert_eq!(CylinderGeometry::new(-1.0, 3.0), Err(GeometryError::NonPositiveInnerRadius));
    }

    #[test]
    fn outer_not_greater_than_inner_is_rejected() {
        assert_eq!(CylinderGeometry::new(3.0, 3.0), Err(GeometryError::OuterNotGreaterThanInner));
        assert_eq!(CylinderGeometry::new(3.0, 2.0), Err(GeometryError::OuterNotGreaterThanInner));
    }

    #[test]
    fn thin_ratio_classifies_as_thin_wall() {
        // t/r_i = 0.05 <= 0.1
        let g = CylinderGeometry::new(20.0, 21.0).unwrap();
        assert_eq!(classify(&g), GeometryClassification::ThinWall);
    }

    #[test]
    fn exactly_the_threshold_classifies_as_thin_wall() {
        // t/r_i = 0.1 exactly - boundary is inclusive on the thin-wall side.
        let g = CylinderGeometry::new(10.0, 11.0).unwrap();
        assert_eq!(classify(&g), GeometryClassification::ThinWall);
    }

    #[test]
    fn thick_ratio_classifies_as_thick_wall() {
        // t/r_i = 0.5, the bushing base fixture's own rough proportions.
        let g = CylinderGeometry::new(2.0, 3.0).unwrap();
        assert_eq!(classify(&g), GeometryClassification::ThickWall);
    }
}
