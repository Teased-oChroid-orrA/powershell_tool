//! Globally bounded world-space point-to-p-curve-trace distance on source spline
//! surfaces. Not a unique surface projection, finite trim or body certificate.
pub use crate::nurbs_distance::{Options as SearchOptions, Progress};
use crate::{
    geometry_bounds::Bounds3,
    geometry_distance::{self, Geometry},
    nurbs,
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_pcurve::{self, ParametricCurve, PcurveDeclaration},
    step_spline_map::SplineParameterMap,
    V3,
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Distance tolerance is physical mm; parameter values remain raw.
    pub search: SearchOptions,
    pub curve_limits: nurbs::Limits,
    pub max_curve_spans_per_enclosure: usize,
    pub max_surface_patches_per_enclosure: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            search: SearchOptions::default(),
            curve_limits: nurbs::Limits::default(),
            max_curve_spans_per_enclosure: 3,
            max_surface_patches_per_enclosure: 64,
        }
    }
}
#[derive(Debug)]
pub struct DistanceResult {
    pub face: EntityReference,
    pub surface: EntityReference,
    pub pcurve: EntityReference,
    pub parameter_interval: [f64; 2],
    pub distance_bounds_mm: [f64; 2],
    pub candidate_parameter: f64,
    /// Contains the mathematical world point at the candidate parameter,
    /// including source Cartesian-to-mm rounding. No parameter error is bounded.
    pub candidate_enclosure_mm: Bounds3,
    /// Midpoint of the candidate enclosure, not an asserted exact surface point.
    pub enclosure_midpoint_mm: V3,
    pub enclosures: usize,
}
struct Trace<'a> {
    map: &'a SplineParameterMap,
    data: &'a Inventory,
    declaration: PcurveDeclaration,
    interval: [f64; 2],
    options: Options,
}
impl Geometry<1> for Trace<'_> {
    fn domain(&self) -> [[f64; 2]; 1] {
        [self.interval]
    }
    fn cells(&self, maximum: usize) -> Result<Vec<[[f64; 2]; 1]>, Error> {
        let mut cells = Vec::new();
        if let ParametricCurve::Spline(spline) = &self.declaration.geometry {
            if self.interval[0] != self.interval[1] {
                for k in spline.curve().knots().windows(2) {
                    let a = k[0].max(self.interval[0]);
                    let b = k[1].min(self.interval[1]);
                    if a < b {
                        geometry_distance::push_cell(&mut cells, [[a, b]], maximum)?;
                    }
                }
                return Ok(cells);
            }
        }
        geometry_distance::push_cell(&mut cells, [self.interval], maximum)?;
        Ok(cells)
    }
    fn enclose(&self, domain: [[f64; 2]; 1]) -> Result<Bounds3, Error> {
        self.map.bounds_for_declaration(
            self.data,
            &self.declaration,
            domain[0],
            self.options.max_curve_spans_per_enclosure,
            self.options.max_surface_patches_per_enclosure,
        )
    }
    fn representative(&self, _: [f64; 1], enclosure: Bounds3) -> Result<V3, Error> {
        Ok(std::array::from_fn(|i| {
            enclosure.min[i] + (enclosure.max[i] - enclosure.min[i]) * 0.5
        }))
    }
}
/// Bound the global distance over a caller-specified complete finite parameter
/// interval. No trim endpoints are inferred. All point/interval boxes must map
/// inside the surface domain, otherwise this conservative diagnostic refuses.
/// The candidate objective is within the requested distance gap of the global
/// minimum; parameter/position uniqueness, regularity and master semantics are
/// unverified. Query/world bounds are mm; raw LINE/CIRCLE/spline parameters and
/// the source UV/knots never receive the body length conversion.
pub fn distance(
    map: &SplineParameterMap,
    data: &Inventory,
    pcurve: EntityReference,
    query_mm: V3,
    parameter_interval: [f64; 2],
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<DistanceResult, Error> {
    data.resolve_reference(map.face_reference())?;
    data.resolve_reference(map.length_context())?;
    let entity = data.resolve_reference(pcurve)?;
    let fail = |kind, message: &str| Error {
        offset: entity.offset,
        kind,
        detail: message.into(),
    };
    if !progress(Progress {
        enclosures: 0,
        pending_intervals: 0,
        distance_bounds: [0., f64::INFINITY],
    }) {
        return Err(fail(
            ErrorKind::Cancelled,
            "source p-curve distance cancelled",
        ));
    }
    if parameter_interval.iter().any(|t| !t.is_finite())
        || parameter_interval[0] > parameter_interval[1]
    {
        return Err(fail(
            ErrorKind::Geometry,
            "invalid finite p-curve query interval",
        ));
    }
    let declaration = step_pcurve::read_pcurve(data, pcurve, options.curve_limits)?;
    if declaration.basis_surface != map.surface_reference() {
        return Err(fail(
            ErrorKind::Geometry,
            "distance p-curve basis does not match source spline surface",
        ));
    }
    if let ParametricCurve::Spline(spline) = &declaration.geometry {
        let domain = spline.curve().domain();
        if parameter_interval[0] < domain[0] || parameter_interval[1] > domain[1] {
            return Err(fail(
                ErrorKind::Geometry,
                "p-curve query outside finite active spline domain",
            ));
        }
    }
    let trace = Trace {
        map,
        data,
        declaration,
        interval: parameter_interval,
        options,
    };
    let bracket = geometry_distance::distance(&trace, query_mm, options.search, progress).map_err(
        |mut e| {
            e.offset = entity.offset;
            e
        },
    )?;
    Ok(DistanceResult {
        face: map.face_reference(),
        surface: map.surface_reference(),
        pcurve,
        parameter_interval,
        distance_bounds_mm: bracket.bounds,
        candidate_parameter: bracket.parameter[0],
        candidate_enclosure_mm: bracket.enclosure,
        enclosure_midpoint_mm: bracket.point,
        enclosures: bracket.calls,
    })
}
