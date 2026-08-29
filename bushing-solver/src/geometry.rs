//! Ported from engineering.toolbox's
//! `src/lib/core/shared/bushingProfileGeometry.ts` - resolves a bushing's
//! axial cross-section (straight/flanged/countersink OD, straight/
//! countersink ID) into a set of geometric parameters, then scans it for
//! the minimum wall thickness. Solver mode only: the TS source's `render`
//! mode branches exist for that project's 3D viewer, which this port has
//! no equivalent of, so `mode` is dropped as a parameter entirely rather
//! than stubbed.

/// The bushing's outer-diameter geometry (`solveEngine.ts`'s
/// `bushingType`). `Countersink` here means an external chamfer/
/// countersink cut into the OD, not the more common internal-ID
/// countersink (`IdType::Countersink`) - a bushing can have either, both,
/// or neither independently.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BushingType {
    #[default]
    Straight,
    Flanged,
    Countersink,
}

/// The bushing's inner-diameter (bore-facing) geometry.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum IdType {
    #[default]
    Straight,
    Countersink,
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Raw geometry inputs for one section resolve, mirroring
/// `SharedBushingProfileInput` (`bushingProfileGeometry.ts:1-15`).
#[derive(Debug, Clone, Copy)]
pub struct BushingSectionInput {
    pub bore_dia: f64,
    pub housing_len: f64,
    pub housing_width: f64,
    pub id_bushing: f64,
    pub bushing_type: BushingType,
    pub id_type: IdType,
    pub flange_od: f64,
    pub flange_thk: f64,
    pub od_bushing: f64,
    /// External (OD-side) countersink (dia, depth) - only meaningful when
    /// `bushing_type == Countersink`.
    pub cs_external: Option<(f64, f64)>,
    /// Internal (ID-side) countersink (dia, depth) - only meaningful when
    /// `id_type == Countersink`.
    pub cs_internal: Option<(f64, f64)>,
}

/// Resolved section parameters - 1:1 with `SharedBushingSectionParams`
/// (`bushingProfileGeometry.ts:19-41`), minus the `mode`-dependent
/// render-only fields this port doesn't need.
#[derive(Debug, Clone, Copy)]
pub struct BushingSectionParams {
    pub l: f64,
    pub z_top: f64,
    pub z_bottom: f64,
    pub r_outer: f64,
    pub r_inner: f64,
    pub ext_top: f64,
    pub z_ext: f64,
    pub int_top: f64,
    pub z_int: f64,
    pub flange_t: f64,
    pub flange_r: f64,
    pub z_flange_top: f64,
    pub inner_top_z: f64,
    pub bushing_type: BushingType,
    pub id_type: IdType,
}

/// Ported from `resolveBushingSectionParams` (`bushingProfileGeometry.ts:51-123`).
pub fn resolve_bushing_section_params(input: &BushingSectionInput) -> BushingSectionParams {
    let d = input.bore_dia.max(1e-6);
    let l = input.housing_len.max(1e-6);
    let id = clamp(input.id_bushing, d * 0.3, d * 0.98);
    let od = clamp(input.od_bushing, d * 0.95, d * 1.15);
    let z_top = -l / 2.0;
    let z_bottom = l / 2.0;
    let r_outer = od / 2.0;
    let r_inner = id / 2.0;

    let flange_t = if input.bushing_type == BushingType::Flanged { clamp(input.flange_thk.max(0.0), 0.0, l * 0.35) } else { 0.0 };
    let z_flange_top = z_top - flange_t;
    let inner_top_z = if input.bushing_type == BushingType::Flanged { z_flange_top } else { z_top };
    let flange_raw = r_outer.max(input.flange_od.max(od) / 2.0);
    let flange_r = if input.bushing_type == BushingType::Flanged { flange_raw } else { r_outer };

    let ext_raw = if input.bushing_type == BushingType::Countersink {
        r_outer.max(input.cs_external.map(|(dia, _)| dia).unwrap_or(r_outer * 2.0) / 2.0)
    } else {
        r_outer
    };
    let ext_top = ext_raw;
    let ext_depth_raw = if input.bushing_type == BushingType::Countersink { input.cs_external.map(|(_, depth)| depth).unwrap_or(0.0).max(0.0) } else { 0.0 };
    let ext_depth = clamp(ext_depth_raw, 0.0, l);
    let z_ext = z_bottom.min(z_top + ext_depth);

    let int_raw = if input.id_type == IdType::Countersink {
        r_inner.max(input.cs_internal.map(|(dia, _)| dia).unwrap_or(r_inner * 2.0) / 2.0)
    } else {
        r_inner
    };
    let int_top = int_raw;
    let int_depth_raw = if input.id_type == IdType::Countersink { input.cs_internal.map(|(_, depth)| depth).unwrap_or(0.0).max(0.0) } else { 0.0 };
    let max_int_depth = (z_bottom - inner_top_z).max(0.0);
    let int_depth = if input.id_type == IdType::Countersink {
        clamp(int_depth_raw, (l * 0.04).min(d * 0.05), max_int_depth)
    } else {
        clamp(int_depth_raw, 0.0, max_int_depth)
    };
    let z_int = z_bottom.min(inner_top_z + int_depth);

    BushingSectionParams {
        l,
        z_top,
        z_bottom,
        r_outer,
        r_inner,
        ext_top,
        z_ext,
        int_top,
        z_int,
        flange_t,
        flange_r,
        z_flange_top,
        inner_top_z,
        bushing_type: input.bushing_type,
        id_type: input.id_type,
    }
}

/// Ported from `evaluateBushingOuterRadius` (`bushingProfileGeometry.ts:125-135`).
pub fn evaluate_bushing_outer_radius(p: &BushingSectionParams, z: f64) -> f64 {
    if p.bushing_type == BushingType::Flanged {
        return if z <= p.z_top { p.flange_r } else { p.r_outer };
    }
    if p.bushing_type == BushingType::Countersink && z <= p.z_ext && p.z_ext > p.z_top {
        let t = clamp((z - p.z_top) / (p.z_ext - p.z_top).max(1e-9), 0.0, 1.0);
        return lerp(p.ext_top, p.r_outer, t);
    }
    p.r_outer
}

/// Ported from `evaluateBushingInnerRadius` (`bushingProfileGeometry.ts:137-143`).
pub fn evaluate_bushing_inner_radius(p: &BushingSectionParams, z: f64) -> f64 {
    if p.id_type == IdType::Countersink && z <= p.z_int && p.z_int > p.inner_top_z {
        let t = clamp((z - p.inner_top_z) / (p.z_int - p.inner_top_z).max(1e-9), 0.0, 1.0);
        return lerp(p.int_top, p.r_inner, t);
    }
    p.r_inner
}

/// Scans the resolved section for its minimum wall thickness - samples
/// every geometric breakpoint (countersink/flange transitions), their
/// midpoints, and epsilon-offset points just inside each breakpoint (so a
/// sharp lerp transition's true minimum, which can sit exactly at a
/// breakpoint, isn't missed by only sampling midpoints). Ported from
/// `computeMinimumBushingWall` (`bushingProfileGeometry.ts:145-167`).
pub fn compute_minimum_bushing_wall(p: &BushingSectionParams) -> f64 {
    let material_top = if p.bushing_type == BushingType::Flanged { p.z_flange_top } else { p.z_top };
    let mut raw_points: Vec<f64> = [material_top, p.z_top, p.z_ext, p.inner_top_z, p.z_int, p.z_bottom].into_iter().filter(|v| v.is_finite()).collect();
    raw_points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut points: Vec<f64> = Vec::with_capacity(raw_points.len());
    for (i, &v) in raw_points.iter().enumerate() {
        if i == 0 || (v - raw_points[i - 1]).abs() > 1e-9 {
            points.push(v);
        }
    }

    let epsilon = p.l.max(1.0) * 1e-6;
    let mut samples: Vec<f64> = points.clone();
    for w in points.windows(2) {
        samples.push((w[0] + w[1]) / 2.0);
    }
    for &point in &points {
        if point > material_top + epsilon {
            samples.push(point - epsilon);
        }
        if point < p.z_bottom - epsilon {
            samples.push(point + epsilon);
        }
    }

    let mut minimum = f64::INFINITY;
    for &z in &samples {
        let wall = evaluate_bushing_outer_radius(p, z) - evaluate_bushing_inner_radius(p, z);
        if wall < minimum {
            minimum = wall;
        }
    }
    if minimum.is_finite() {
        minimum
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight_input() -> BushingSectionInput {
        BushingSectionInput {
            bore_dia: 0.5,
            housing_len: 0.5,
            housing_width: 1.5,
            id_bushing: 0.375,
            bushing_type: BushingType::Straight,
            id_type: IdType::Straight,
            flange_od: 0.0,
            flange_thk: 0.0,
            od_bushing: 0.5015,
            cs_external: None,
            cs_internal: None,
        }
    }

    #[test]
    fn straight_bushing_minimum_wall_equals_the_uniform_straight_wall() {
        let params = resolve_bushing_section_params(&straight_input());
        let wall = compute_minimum_bushing_wall(&params);
        let expected = (0.5015 - 0.375) / 2.0;
        assert!((wall - expected).abs() < 1e-9);
    }

    #[test]
    fn internal_countersink_thins_the_wall_at_the_countersink_end() {
        let mut input = straight_input();
        input.id_type = IdType::Countersink;
        input.cs_internal = Some((0.45, 0.1)); // wider ID at the countersink end
        let params = resolve_bushing_section_params(&input);
        let wall = compute_minimum_bushing_wall(&params);
        let straight_wall = (0.5015 - 0.375) / 2.0;
        assert!(wall < straight_wall, "countersunk end must be thinner than the uniform straight wall");
    }

    #[test]
    fn flanged_bushing_still_reports_the_straight_section_wall() {
        let mut input = straight_input();
        input.bushing_type = BushingType::Flanged;
        input.flange_od = 0.75;
        input.flange_thk = 0.06;
        let params = resolve_bushing_section_params(&input);
        let wall = compute_minimum_bushing_wall(&params);
        let straight_wall = (0.5015 - 0.375) / 2.0;
        assert!((wall - straight_wall).abs() < 1e-9, "flange only extends axially beyond the housing, doesn't thin the in-housing wall");
    }
}
