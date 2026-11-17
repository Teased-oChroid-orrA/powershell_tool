//! Section library for frame members (#234: "Section library"): the properties of common member cross-sections from their
//! dimensions, so a [`SectionSpec`] and the fibre distances of a [`MemberAllowable`] need not be typed from a handbook.
//!
//! Only shapes with two axes of symmetry are offered. The frame element (`fea_core::structural`) takes the local `y` / `z` axes as
//! the principal axes and puts the shear centre at the centroid; an angle, channel or tee breaks one or both, and would be solved
//! wrongly without saying so.
//!
//! Axes: `width` runs along the member's local `y`, `height` along local `z`. `iy` is the second moment for bending about `y`
//! (it grows with `height`), `iz` about `z`. `shear_areas` is `[along y, along z]`, the order `FrameSection` reads.
//!
//! What is exact and what is an estimate:
//! - Area and second moments: exact for the stated outline (no fillets or corner radii).
//! - Torsion constant: exact for the round shapes and the rectangle (Saint-Venant series); thin-wall formulas for the box
//!   (Bredt, mid-line) and the I-beam (sum of `b t^3 / 3`), which are accurate for thin walls and ignore fillets. Warping restraint
//!   of the I-beam is not modelled by the frame element.
//! - Shear areas: solid round `0.9 A` and rectangle `5/6 A` (the classical values, the same as [`SectionSpec::solid_round`]);
//!   round tube by Cowper's coefficient (J. Appl. Mech. 33, 1966), which depends on Poisson's ratio and tends to
//!   `2(1+nu)/(4+3nu)` for a thin wall; box and I-beam by the area of the walls parallel to the shear (webs carry `z` shear,
//!   flanges `5/6` of their area in `y`), the usual thin-wall estimate.
use crate::member_checks::MemberAllowable;
use crate::members::SectionSpec;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SectionShape {
    SolidRound { radius: f64 },
    /// `wall` is the wall thickness; it must be less than the outer radius.
    RoundTube { outer_radius: f64, wall: f64 },
    Rectangle { width: f64, height: f64 },
    /// Rectangular tube of uniform wall thickness, outside dimensions `width` x `height`.
    BoxTube { width: f64, height: f64, wall: f64 },
    /// Doubly symmetric I-beam: flange `width` and thickness `flange`, overall `height`, web thickness `web`.
    IBeam { width: f64, height: f64, flange: f64, web: f64 },
}

/// Geometric properties of a section (no material).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionProperties {
    pub area: f64,
    pub iy: f64,
    pub iz: f64,
    pub torsion: f64,
    /// Effective shear areas `[along y, along z]`.
    pub shear_areas: [f64; 2],
    /// Distance from the centroid to the extreme fibre along local `y` (bending about `z`).
    pub fibre_y: f64,
    /// Distance from the centroid to the extreme fibre along local `z` (bending about `y`).
    pub fibre_z: f64,
}

/// Saint-Venant torsion constant of a solid rectangle with sides `a >= b`: `a b^3 [1/3 - (64 b / (pi^5 a)) sum tanh(n pi a / 2b) / n^5]`
/// over odd `n`. The terms fall as `n^-5`; 60 of them are past round-off.
fn rectangle_torsion(width: f64, height: f64) -> f64 {
    let (a, b) = if width >= height { (width, height) } else { (height, width) };
    let sum: f64 = (0..60).map(|k| (2 * k + 1) as f64).map(|n| (n * PI * a / (2.0 * b)).tanh() / n.powi(5)).sum();
    a * b.powi(3) * (1.0 / 3.0 - 64.0 * b / (PI.powi(5) * a) * sum)
}

impl SectionShape {
    /// The shape's name and its dimensions in order, for a picker or a report.
    pub fn label(&self) -> &'static str {
        match self {
            Self::SolidRound { .. } => "Solid round",
            Self::RoundTube { .. } => "Round tube",
            Self::Rectangle { .. } => "Rectangle",
            Self::BoxTube { .. } => "Rectangular tube",
            Self::IBeam { .. } => "I-beam",
        }
    }

    fn validate(&self) -> Result<(), String> {
        let positive = |v: &[(f64, &str)]| -> Result<(), String> {
            match v.iter().find(|(x, _)| !(x.is_finite() && *x > 0.0)) {
                Some((_, what)) => Err(format!("{}: the {what} must be positive", self.label())),
                None => Ok(()),
            }
        };
        match *self {
            Self::SolidRound { radius } => positive(&[(radius, "radius")]),
            Self::RoundTube { outer_radius, wall } => {
                positive(&[(outer_radius, "outer radius"), (wall, "wall thickness")])?;
                if wall >= outer_radius {
                    return Err("Round tube: the wall thickness must be less than the outer radius (use a solid round)".into());
                }
                Ok(())
            }
            Self::Rectangle { width, height } => positive(&[(width, "width"), (height, "height")]),
            Self::BoxTube { width, height, wall } => {
                positive(&[(width, "width"), (height, "height"), (wall, "wall thickness")])?;
                if 2.0 * wall >= width.min(height) {
                    return Err("Rectangular tube: twice the wall thickness must be less than the width and the height (use a rectangle)".into());
                }
                Ok(())
            }
            Self::IBeam { width, height, flange, web } => {
                positive(&[(width, "flange width"), (height, "height"), (flange, "flange thickness"), (web, "web thickness")])?;
                if 2.0 * flange >= height {
                    return Err("I-beam: the two flanges must be thinner than the height".into());
                }
                if web > width {
                    return Err("I-beam: the web must not be thicker than the flange is wide".into());
                }
                Ok(())
            }
        }
    }

    /// Properties of the shape. `nu` is Poisson's ratio (it enters only the round tube's shear area); dimensions that do not make
    /// the shape are refused with the reason.
    pub fn properties(&self, nu: f64) -> Result<SectionProperties, String> {
        self.validate()?;
        if !(nu.is_finite() && nu > -1.0 && nu < 0.5) {
            return Err("Poisson's ratio must be between -1 and 0.5".into());
        }
        Ok(match *self {
            Self::SolidRound { radius: r } => {
                let (area, i) = (PI * r * r, PI * r.powi(4) / 4.0);
                SectionProperties { area, iy: i, iz: i, torsion: 2.0 * i, shear_areas: [0.9 * area; 2], fibre_y: r, fibre_z: r }
            }
            Self::RoundTube { outer_radius: ro, wall } => {
                let ri = ro - wall;
                let (area, i) = (PI * (ro * ro - ri * ri), PI * (ro.powi(4) - ri.powi(4)) / 4.0);
                // Cowper: k = 6 (1 + nu) (1 + m^2)^2 / [(7 + 6 nu) (1 + m^2)^2 + (20 + 12 nu) m^2], m = ri / ro.
                let m2 = (ri / ro).powi(2);
                let k = 6.0 * (1.0 + nu) * (1.0 + m2).powi(2) / ((7.0 + 6.0 * nu) * (1.0 + m2).powi(2) + (20.0 + 12.0 * nu) * m2);
                SectionProperties { area, iy: i, iz: i, torsion: 2.0 * i, shear_areas: [k * area; 2], fibre_y: ro, fibre_z: ro }
            }
            Self::Rectangle { width: b, height: h } => {
                let area = b * h;
                SectionProperties {
                    area,
                    iy: b * h.powi(3) / 12.0,
                    iz: h * b.powi(3) / 12.0,
                    torsion: rectangle_torsion(b, h),
                    shear_areas: [5.0 / 6.0 * area; 2],
                    fibre_y: b / 2.0,
                    fibre_z: h / 2.0,
                }
            }
            Self::BoxTube { width: b, height: h, wall: t } => {
                let (bi, hi) = (b - 2.0 * t, h - 2.0 * t);
                // Bredt: 4 A_m^2 t / (mid-line perimeter), A_m the area inside the wall mid-line.
                let (bm, hm) = (b - t, h - t);
                SectionProperties {
                    area: b * h - bi * hi,
                    iy: (b * h.powi(3) - bi * hi.powi(3)) / 12.0,
                    iz: (h * b.powi(3) - hi * bi.powi(3)) / 12.0,
                    torsion: 2.0 * t * (bm * hm).powi(2) / (bm + hm),
                    shear_areas: [2.0 * b * t, 2.0 * h * t],
                    fibre_y: b / 2.0,
                    fibre_z: h / 2.0,
                }
            }
            Self::IBeam { width: b, height: h, flange: tf, web: tw } => {
                let hw = h - 2.0 * tf;
                SectionProperties {
                    area: 2.0 * b * tf + hw * tw,
                    iy: (b * h.powi(3) - (b - tw) * hw.powi(3)) / 12.0,
                    iz: (2.0 * tf * b.powi(3) + hw * tw.powi(3)) / 12.0,
                    torsion: (2.0 * b * tf.powi(3) + (h - tf) * tw.powi(3)) / 3.0,
                    shear_areas: [5.0 / 6.0 * 2.0 * b * tf, h * tw],
                    fibre_y: b / 2.0,
                    fibre_z: h / 2.0,
                }
            }
        })
    }

    /// The frame section of this shape in an isotropic material (`G = E / 2(1 + nu)`), with Timoshenko shear areas.
    pub fn spec(&self, e: f64, nu: f64) -> Result<SectionSpec, String> {
        if !(e.is_finite() && e > 0.0) {
            return Err("the elastic modulus must be positive".into());
        }
        let p = self.properties(nu)?;
        Ok(SectionSpec { e, g: e / (2.0 * (1.0 + nu)), area: p.area, iy: p.iy, iz: p.iz, torsion: p.torsion, shear_areas: Some(p.shear_areas) })
    }
}

impl SectionProperties {
    /// The margin inputs of a member with this section: its fibre distances with the given allowable stress and
    /// effective-length factor.
    pub fn allowable(&self, allowable_stress: f64, effective_length_factor: f64) -> MemberAllowable {
        MemberAllowable { allowable_stress, effective_length_factor, fibre_y: self.fibre_y, fibre_z: self.fibre_z }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member_checks::margins;
    use crate::members::{Edit, MemberKind, MemberProblem};

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * b.abs(), "{a} vs {b}");
    }

    #[test]
    fn the_solid_round_is_the_existing_section_and_a_thin_tube_matches_the_thin_wall_forms() {
        let s = SectionShape::SolidRound { radius: 1.0 }.spec(1.0e7, 0.33).unwrap();
        assert_eq!(s, SectionSpec::solid_round(1.0e7, 0.33, 1.0), "one answer for the solid round");
        // Thin tube, mean radius 1, wall 0.001: A = 2 pi r t, I = pi r^3 t, J = 2 pi r^3 t, shear area -> 2(1+nu)/(4+3nu) A.
        let (r, t, nu) = (1.0, 1.0e-3, 0.3);
        let p = SectionShape::RoundTube { outer_radius: r + t / 2.0, wall: t }.properties(nu).unwrap();
        close(p.area, 2.0 * PI * r * t, 1e-9);
        close(p.iy, PI * r.powi(3) * t, 1e-6);
        close(p.torsion, 2.0 * PI * r.powi(3) * t, 1e-6);
        close(p.shear_areas[0], 2.0 * (1.0 + nu) / (4.0 + 3.0 * nu) * p.area, 1e-6);
        assert_eq!(p.fibre_y, r + t / 2.0);
        // A hand value: OD 2, wall 0.25: A = pi (1 - 0.5625) = 1.37445, I = pi (1 - 0.31640625) / 4 = 0.536893.
        let p = SectionShape::RoundTube { outer_radius: 1.0, wall: 0.25 }.properties(0.3).unwrap();
        close(p.area, 1.374447, 1e-6);
        close(p.iz, 0.536893, 1e-6);
        // A wall that fills the tube is the solid round's area and inertia; Cowper's solid value is 6(1+nu)/(7+6nu).
        let p = SectionShape::RoundTube { outer_radius: 1.0, wall: 1.0 - 1e-9 }.properties(0.3).unwrap();
        close(p.iy, PI / 4.0, 1e-8);
        close(p.shear_areas[1] / p.area, 6.0 * 1.3 / (7.0 + 1.8), 1e-8);
    }

    #[test]
    fn rectangle_inertias_follow_the_axes_and_torsion_matches_the_tabulated_coefficients() {
        // 2 wide (y), 6 high (z): bending about y uses the height.
        let p = SectionShape::Rectangle { width: 2.0, height: 6.0 }.properties(0.3).unwrap();
        close(p.area, 12.0, 1e-15);
        close(p.iy, 36.0, 1e-15);
        close(p.iz, 4.0, 1e-15);
        assert_eq!((p.fibre_y, p.fibre_z), (1.0, 3.0));
        close(p.shear_areas[0], 10.0, 1e-15);
        // Saint-Venant coefficients beta in J = beta a b^3 (Timoshenko & Goodier): a/b = 1: 0.1406, 2: 0.229, 3: 0.263, 10: 0.312.
        for (ratio, beta) in [(1.0, 0.1406), (2.0, 0.229), (3.0, 0.263), (10.0, 0.312)] {
            let j = SectionShape::Rectangle { width: ratio, height: 1.0 }.properties(0.3).unwrap().torsion;
            close(j / ratio, beta, 2e-3);
            let turned = SectionShape::Rectangle { width: 1.0, height: ratio }.properties(0.3).unwrap().torsion;
            close(turned, j, 1e-14);
        }
        // A thin strip tends to a b^3 / 3 (1 - 0.630 b / a).
        let j = SectionShape::Rectangle { width: 100.0, height: 1.0 }.properties(0.3).unwrap().torsion;
        close(j, 100.0 / 3.0 * (1.0 - 0.630 / 100.0), 1e-4);
    }

    #[test]
    fn box_and_i_beam_match_hand_values() {
        // Box 4 x 6, wall 0.25: outer minus inner rectangle.
        let p = SectionShape::BoxTube { width: 4.0, height: 6.0, wall: 0.25 }.properties(0.3).unwrap();
        close(p.area, 24.0 - 3.5 * 5.5, 1e-14);
        close(p.iy, (4.0 * 216.0 - 3.5 * 166.375) / 12.0, 1e-14);
        close(p.iz, (6.0 * 64.0 - 5.5 * 42.875) / 12.0, 1e-14);
        // Bredt: A_m = 3.75 * 5.75 = 21.5625, perimeter 19: 4 * 21.5625^2 * 0.25 / 19 = 24.4706.
        close(p.torsion, 4.0 * 21.5625f64.powi(2) * 0.25 / 19.0, 1e-14);
        assert_eq!(p.shear_areas, [2.0, 3.0]);
        // I-beam 4 wide, 8 deep, flange 0.5, web 0.25: A = 4 + 7 * 0.25 = 5.75; Iy = (4 * 512 - 3.75 * 343) / 12 = 63.4792;
        // Iz = (2 * 0.5 * 64 + 7 * 0.015625) / 12 = 5.34245.
        let p = SectionShape::IBeam { width: 4.0, height: 8.0, flange: 0.5, web: 0.25 }.properties(0.3).unwrap();
        close(p.area, 5.75, 1e-14);
        close(p.iy, 63.479167, 1e-8);
        close(p.iz, 5.342448, 1e-7);
        close(p.torsion, (2.0 * 4.0 * 0.125 + 7.5 * 0.015625) / 3.0, 1e-14);
        assert_eq!((p.fibre_y, p.fibre_z), (2.0, 4.0));
        // A web as wide as the flange and flanges meeting the mid-height would be a rectangle: the inertias agree in that limit.
        let full = SectionShape::IBeam { width: 2.0, height: 6.0, flange: 1.0, web: 2.0 }.properties(0.3).unwrap();
        let rect = SectionShape::Rectangle { width: 2.0, height: 6.0 }.properties(0.3).unwrap();
        close(full.area, rect.area, 1e-14);
        close(full.iy, rect.iy, 1e-14);
        close(full.iz, rect.iz, 1e-14);
    }

    #[test]
    fn dimensions_that_do_not_make_the_shape_are_refused_with_the_reason() {
        let e = |s: SectionShape| s.properties(0.3).unwrap_err();
        assert!(e(SectionShape::SolidRound { radius: 0.0 }).contains("radius must be positive"));
        assert!(e(SectionShape::RoundTube { outer_radius: 1.0, wall: 1.0 }).contains("less than the outer radius"));
        assert!(e(SectionShape::Rectangle { width: f64::NAN, height: 1.0 }).contains("width must be positive"));
        assert!(e(SectionShape::BoxTube { width: 1.0, height: 2.0, wall: 0.5 }).contains("twice the wall"));
        assert!(e(SectionShape::IBeam { width: 4.0, height: 1.0, flange: 0.5, web: 0.25 }).contains("flanges"));
        assert!(e(SectionShape::IBeam { width: 1.0, height: 8.0, flange: 0.5, web: 2.0 }).contains("web"));
        assert!(SectionShape::Rectangle { width: 1.0, height: 1.0 }.properties(0.5).unwrap_err().contains("Poisson"));
        assert!(SectionShape::Rectangle { width: 1.0, height: 1.0 }.spec(0.0, 0.3).unwrap_err().contains("modulus"));
    }

    #[test]
    fn a_cantilever_of_a_library_section_deflects_and_stresses_by_beam_theory() {
        // Rectangle 1 (y) x 3 (z), L = 30, tip load 200 lbf along local z, which is global z here: bending about y (the strong axis).
        let (e, nu, l, f) = (1.0e7, 0.3, 30.0, 200.0);
        let shape = SectionShape::Rectangle { width: 1.0, height: 3.0 };
        let (props, section) = (shape.properties(nu).unwrap(), shape.spec(e, nu).unwrap());
        let mut p = MemberProblem::new("library cantilever");
        let a = p.apply(Edit::AddNode { pos: [0.0; 3] }).unwrap().unwrap();
        let b = p.apply(Edit::AddNode { pos: [l, 0.0, 0.0] }).unwrap().unwrap();
        // reference y: local z = x cross reference = global z, local y = z cross x = global y.
        p.apply(Edit::AddMember { a, b, kind: MemberKind::Frame { section, reference: [0.0, 1.0, 0.0], uniform_load: [0.0; 3] } }).unwrap();
        p.apply(Edit::SetSupport { node: a, fixed: [true; 6] }).unwrap();
        p.apply(Edit::SetLoad { node: b, force: [0.0, 0.0, f], moment: [0.0; 3] }).unwrap();
        let solved = p.solve().unwrap();
        // Timoshenko tip deflection: F L^3 / (3 E Iy) + F L / (G As_z).
        let expect = f * l.powi(3) / (3.0 * e * props.iy) + f * l / (section.g * props.shear_areas[1]);
        close(solved.displacement[1][2], expect, 1e-9);
        // Root stress M c / I with the library's fibre distance: 200 * 30 * 1.5 / 2.25 = 4000 psi.
        let m = margins(&p, &solved, |_| Some(props.allowable(40_000.0, 1.0))).unwrap();
        close(m[0].stress.sigma_max, 4000.0, 1e-9);
        close(m[0].stress.margin, 9.0, 1e-9);
        // The same load along y bends the weak axis: 200 * 30 * 0.5 / 0.25 = 12000 psi.
        p.apply(Edit::SetLoad { node: b, force: [0.0, f, 0.0], moment: [0.0; 3] }).unwrap();
        let weak = margins(&p, &p.solve().unwrap(), |_| Some(props.allowable(40_000.0, 1.0))).unwrap();
        close(weak[0].stress.sigma_max, 12_000.0, 1e-9);
    }
}
