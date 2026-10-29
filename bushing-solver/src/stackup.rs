//! Coplanar straight-axis stack sizing. Inches and degrees. Layer thickness is
//! measured along stack z; orientation reference surfaces are independent datums.
use crate::tolerance::{make_range, ToleranceMode, ToleranceRange};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub nominal: f64,
    pub min: f64,
    pub max: f64,
}
impl Band {
    pub fn exact(v: f64) -> Self {
        Self {
            nominal: v,
            min: v,
            max: v,
        }
    }
    fn valid(self) -> bool {
        [self.min, self.nominal, self.max]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
            && self.min <= self.nominal
            && self.nominal <= self.max
    }
    pub fn resolved(self) -> ToleranceRange {
        make_range(
            ToleranceMode::Limits,
            self.min,
            self.max,
            Some(self.nominal),
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngularityMode {
    Degrees,
    CylindricalZone,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Angularity {
    pub degrees: f64,
    pub zone_diameter: f64,
    pub reference_span: f64,
}
impl Angularity {
    fn angle(self, mode: AngularityMode) -> Option<f64> {
        if !self.reference_span.is_finite()
            || self.reference_span <= 0.0
            || !self.degrees.is_finite()
            || !(0.0..90.0).contains(&self.degrees)
            || !self.zone_diameter.is_finite()
            || self.zone_diameter < 0.0
        {
            return None;
        }
        match mode {
            AngularityMode::Degrees
                if self.degrees.is_finite() && (0.0..90.0).contains(&self.degrees) =>
            {
                Some(self.degrees)
            }
            AngularityMode::CylindricalZone
                if self.zone_diameter.is_finite()
                    && self.zone_diameter >= 0.0
                    && self.reference_span.is_finite()
                    && self.reference_span > 0.0 =>
            {
                Some(
                    (self.zone_diameter / self.reference_span)
                        .atan()
                        .to_degrees(),
                )
            }
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WallMode {
    Required,
    Informational,
    Unclassified,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedWall {
    pub value: f64,
    pub units: String,
    pub workbench: String,
    pub source_id: String,
    pub semantics: WallMode,
    pub context: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub thickness: f64,
    #[serde(default = "default_span")]
    pub member_width: f64,
    #[serde(default = "default_span")]
    pub member_depth: f64,
    pub hole: Band,
    #[serde(default)]
    pub finished_id_min: Option<f64>,
    #[serde(default)]
    pub finished_id_max: Option<f64>,
    pub seating_min: f64,
    pub seating_max: f64,
    pub od_tolerance: f64,
    pub reference_normal_deg: f64,
    pub original: Angularity,
    pub finished: Angularity,
    pub wall_override: Option<f64>,
    pub wall_mode: WallMode,
    pub imported_wall: Option<ImportedWall>,
    #[serde(default)]
    pub imported_use: Option<WallMode>,
    pub housing_material: String,
    pub bushing_material: String,
}
fn default_span() -> f64 {
    1.5
}
impl Member {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            name: format!("Member {id}"),
            thickness: 0.5,
            member_width: 1.5,
            member_depth: 1.5,
            hole: Band::exact(0.3),
            finished_id_min: None,
            finished_id_max: None,
            seating_min: 0.0,
            seating_max: 2.0,
            od_tolerance: 0.001,
            reference_normal_deg: 0.0,
            original: Angularity {
                degrees: 1.0,
                zone_diameter: 0.01,
                reference_span: 1.0,
            },
            finished: Angularity {
                degrees: 0.25,
                zone_diameter: 0.005,
                reference_span: 1.0,
            },
            wall_override: None,
            wall_mode: WallMode::Required,
            imported_wall: None,
            imported_use: None,
            housing_material: "al7075".into(),
            bushing_material: "bronze".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    #[serde(default)]
    pub bushing_enabled: bool,
    pub members: Vec<Member>,
    pub mode: AngularityMode,
    pub common_wall: Option<f64>,
    pub finished_id: Band,
    pub uniform_od: bool,
    pub pilot_z: f64,
    /// Maximum relative center separation at pilot. Linear radius, not diameter.
    pub pilot_offset: f64,
}
impl Default for Inputs {
    fn default() -> Self {
        Self {
            bushing_enabled: false,
            members: vec![Member::new(1), Member::new(2)],
            mode: AngularityMode::Degrees,
            common_wall: Some(0.025),
            finished_id: Band::exact(0.25),
            uniform_od: false,
            pilot_z: 0.0,
            pilot_offset: 0.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct WallPolicy {
    pub common: Option<f64>,
    pub override_value: Option<f64>,
    pub effective: Option<f64>,
    pub source: &'static str,
    pub mode: WallMode,
    pub imported: Option<ImportedWall>,
    pub imported_use: Option<WallMode>,
}
pub fn wall_policy(input: &Inputs, m: &Member) -> WallPolicy {
    WallPolicy {
        common: input.common_wall,
        override_value: m.wall_override,
        effective: m.wall_override.or(input.common_wall),
        source: if matches!(
            m.imported_use,
            Some(WallMode::Required | WallMode::Informational)
        ) && m
            .imported_wall
            .as_ref()
            .is_some_and(|w| m.wall_override == Some(w.value))
        {
            "Imported value (explicit use)"
        } else if m.wall_override.is_some() {
            "Member override"
        } else if input.common_wall.is_some() {
            "Common default"
        } else {
            "Not configured"
        },
        mode: m.wall_mode,
        imported: m.imported_wall.clone(),
        imported_use: m.imported_use,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Feasible,
    Informational,
    Incomplete,
    ReviewRequired,
    Infeasible,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LayerResult {
    pub cleanup_diameter: f64,
    pub final_hole: Band,
    pub enlargement_reason: Option<String>,
    pub id: u64,
    pub z: [f64; 2],
    pub od: Band,
    pub center_x: f64,
    pub original_envelope: [f64; 2],
    pub finished_envelope: [f64; 2],
    pub eccentricity_max: f64,
    pub nominal_eccentricity: [f64; 2],
    pub original_angle_max: f64,
    pub finished_angle_max: f64,
    pub original_zone_used: f64,
    pub finished_zone_used: f64,
    pub original_displacement: f64,
    pub finished_displacement: f64,
    pub bore_capacity: f64,
    pub wall_min: f64,
    /// Maximum directional support-plane ligament over all tolerance cases.
    pub wall_max: f64,
    pub policy: WallPolicy,
    pub status: Status,
    pub governing: String,
    pub comparison: String,
    pub imported_comparison: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LayerSection {
    pub od: [[f64; 2]; 5],
    pub original: [[[f64; 2]; 2]; 2],
    pub finished: [[[f64; 2]; 2]; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub bounds: [[f64; 2]; 2],
    pub layers: Vec<LayerSection>,
    pub original_axis: [[f64; 2]; 2],
    pub finished_axis: [[f64; 2]; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub struct SolidLayer {
    pub original_hole: [Vec<[f64; 3]>; 2],
    pub id: u64,
    pub body: [[f64; 3]; 8],
    pub hole: [Vec<[f64; 3]>; 2],
    pub bushing_id: Option<[Vec<[f64; 3]>; 2]>,
    pub axis: [f64; 3],
    pub z: [f64; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResultSet {
    pub solids: Vec<SolidLayer>,
    pub section: Section,
    pub original_angles: [f64; 2],
    pub finished_angles: [f64; 2],
    pub layers: Vec<LayerResult>,
    pub length: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidInput,
    Infeasible,
    NumericalFailure,
}
#[derive(Clone, Debug, PartialEq)]
pub struct InputError {
    pub kind: ErrorKind,
    pub member: Option<u64>,
    pub parameter: &'static str,
    pub message: String,
}
fn error(member: Option<u64>, parameter: &'static str, message: &str) -> InputError {
    InputError {
        kind: ErrorKind::InvalidInput,
        member,
        parameter,
        message: message.into(),
    }
}
fn nonnegative(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

/// Axis intervals are intersected once for the whole stack. The orientation-only
/// cylindrical RFS zone is free to translate; pilot position is a separate rule.
pub fn calculate(input: &Inputs) -> Result<ResultSet, InputError> {
    if input.members.is_empty() {
        return Err(error(None, "members", "Add at least one member"));
    }
    if input.bushing_enabled && !input.finished_id.valid() {
        return Err(error(
            None,
            "finished ID",
            "Positive ordered nominal/min/max dimensions required",
        ));
    }
    if !nonnegative(input.pilot_offset)
        || !nonnegative(input.pilot_z)
        || (input.bushing_enabled && input.common_wall.is_some_and(|v| !nonnegative(v)))
    {
        return Err(error(
            None,
            "pilot / wall",
            "Finite nonnegative values required",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut a: [f64; 2] = [-90.0, 90.0];
    let mut b = a;
    let mut length = 0.0;
    for m in &input.members {
        if !ids.insert(m.id) {
            return Err(error(
                Some(m.id),
                "identifier",
                "Member identifiers must be unique",
            ));
        }
        if !m.thickness.is_finite()
            || m.thickness <= 0.0
            || !m.member_width.is_finite()
            || m.member_width <= 0.0
            || !m.member_depth.is_finite()
            || m.member_depth <= 0.0
            || !m.hole.valid()
            || (input.bushing_enabled
                && (m
                    .finished_id_min
                    .is_some_and(|v| !v.is_finite() || v <= 0.0)
                    || m.finished_id_max
                        .is_some_and(|v| !v.is_finite() || v <= 0.0)
                    || matches!((m.finished_id_min,m.finished_id_max),(Some(lo),Some(hi)) if lo>hi)))
            || !m.seating_min.is_finite()
            || m.seating_min < 0.0
            || !m.seating_max.is_finite()
            || m.seating_max < m.seating_min
            || !nonnegative(m.od_tolerance)
            || !m.reference_normal_deg.is_finite()
            || m.reference_normal_deg.abs() >= 90.0
            || (input.bushing_enabled && m.wall_override.is_some_and(|v| !nonnegative(v)))
        {
            return Err(error(
                Some(m.id),
                "dimensions / normal / wall",
                "Check positive thickness, ordered diameter limits and finite reference normal",
            ));
        }
        if let Some(w) = &m.imported_wall {
            if input.bushing_enabled && (!nonnegative(w.value) || w.units != "in") {
                return Err(error(
                    Some(m.id),
                    "imported wall",
                    "Expected finite nonnegative inches; convert source units explicitly",
                ));
            }
        }
        let oa = m.original.angle(input.mode).ok_or_else(|| {
            error(
                Some(m.id),
                "original angularity",
                "Invalid angle or cylindrical zone/reference span",
            )
        })?;
        let fa = m.finished.angle(input.mode).ok_or_else(|| {
            error(
                Some(m.id),
                "finished angularity",
                "Invalid angle or cylindrical zone/reference span",
            )
        })?;
        for (interval, tol) in [(&mut a, oa), (&mut b, fa)] {
            let lo = m.reference_normal_deg - tol;
            let hi = m.reference_normal_deg + tol;
            if lo <= -90.0 || hi >= 90.0 {
                return Err(error(
                    Some(m.id),
                    "angularity",
                    "Axis interval must stay strictly between -90 and +90 degrees",
                ));
            }
            interval[0] = interval[0].max(lo);
            interval[1] = interval[1].min(hi);
        }
        let end = length + m.thickness;
        if !end.is_finite() || end <= length {
            let mut e = error(
                Some(m.id),
                "axial coordinates",
                "Thickness cannot be represented at this cumulative stack length",
            );
            e.kind = ErrorKind::NumericalFailure;
            return Err(e);
        }
        length = end;
    }
    if !length.is_finite() || input.pilot_z > length {
        return Err(error(
            None,
            "pilot z",
            "Pilot must lie within finite stack span",
        ));
    }
    if a[0] > a[1] || b[0] > b[1] {
        let mut e = error(
            None,
            "common axes",
            "Member angularity constraints have no common straight axis",
        );
        e.kind = ErrorKind::Infeasible;
        return Err(e);
    }
    let mut z = 0.0;
    let mut layers = Vec::with_capacity(input.members.len());
    for m in &input.members {
        let faces = [z, z + m.thickness];
        z = faces[1];
        let cleanup = cleanup_diameter(faces, input.pilot_z, a, b, m.hole.max, input.pilot_offset);
        let policy = wall_policy(input, m);
        let wall = if input.bushing_enabled && policy.mode == WallMode::Required {
            policy.effective.unwrap_or(0.0)
        } else {
            0.0
        };
        let baseline = cleanup.max(m.seating_min);
        let diameter = if input.bushing_enabled {
            baseline.max(input.finished_id.max + 2.0 * wall)
        } else {
            baseline
        };
        let nominal_b = b[0] / 2.0 + b[1] / 2.0;
        let original_angle_max = (a[0] - m.reference_normal_deg)
            .abs()
            .max((a[1] - m.reference_normal_deg).abs());
        let finished_angle_max = (b[0] - m.reference_normal_deg)
            .abs()
            .max((b[1] - m.reference_normal_deg).abs());
        layers.push(LayerResult {
            id: m.id,
            z: faces,
            cleanup_diameter: cleanup,
            final_hole: Band {
                nominal: diameter,
                min: diameter,
                max: diameter + m.od_tolerance,
            },
            enlargement_reason: None,
            od: Band {
                nominal: diameter,
                min: diameter,
                max: diameter + m.od_tolerance,
            },
            center_x: ((faces[0] / 2.0 + faces[1] / 2.0) - input.pilot_z)
                * nominal_b.to_radians().tan(),
            original_envelope: envelope(faces, input.pilot_z, a, m.hole.max, 0.0),
            finished_envelope: envelope(faces, input.pilot_z, b, diameter, input.pilot_offset),
            eccentricity_max: 0.0,
            nominal_eccentricity: [0.0, 0.0],
            original_angle_max,
            finished_angle_max,
            original_zone_used: m.original.reference_span * original_angle_max.to_radians().tan(),
            finished_zone_used: m.finished.reference_span * finished_angle_max.to_radians().tan(),
            original_displacement: m.thickness
                * a.iter()
                    .map(|v| {
                        (v.to_radians().tan() - m.reference_normal_deg.to_radians().tan()).abs()
                    })
                    .fold(0.0, f64::max),
            finished_displacement: m.thickness
                * b.iter()
                    .map(|v| {
                        (v.to_radians().tan() - m.reference_normal_deg.to_radians().tan()).abs()
                    })
                    .fold(0.0, f64::max),
            bore_capacity: 0.0,
            wall_min: 0.0,
            wall_max: 0.0,
            policy,
            status: Status::Feasible,
            governing: "Angularity cleanup on common finished axis".into(),
            comparison: String::new(),
            imported_comparison: None,
        });
    }
    let uniform = layers.iter().map(|r| r.final_hole.min).fold(0.0, f64::max);
    for (r, m) in layers.iter_mut().zip(&input.members) {
        if input.bushing_enabled && input.uniform_od {
            r.final_hole = Band {
                nominal: uniform,
                min: uniform,
                max: uniform + m.od_tolerance,
            };
        }
        r.od = r.final_hole;
        let mut reasons = Vec::new();
        if m.seating_min > r.cleanup_diameter {
            reasons.push("configured machining minimum");
        }
        if input.bushing_enabled {
            let required = if r.policy.mode == WallMode::Required {
                r.policy.effective.unwrap_or(0.0)
            } else {
                0.0
            };
            if input.finished_id.max + 2.0 * required > r.cleanup_diameter.max(m.seating_min) {
                reasons.push("bushing ID and minimum wall");
            }
            if input.uniform_od
                && uniform
                    > r.cleanup_diameter
                        .max(m.seating_min)
                        .max(input.finished_id.max + 2.0 * required)
            {
                reasons.push("uniform bushing OD");
            }
            r.wall_min = (r.od.min - input.finished_id.max) / 2.0;
            r.wall_max = (r.od.max - input.finished_id.min) / 2.0;
            r.bore_capacity = r.od.min - 2.0 * required;
            r.status = if r.policy.mode == WallMode::Required && r.policy.effective.is_none() {
                Status::Incomplete
            } else if r.wall_min <= 0.0
                || m.finished_id_min.is_some_and(|v| input.finished_id.min < v)
                || m.finished_id_max.is_some_and(|v| input.finished_id.max > v)
            {
                r.governing = "Bushing ID, wall or member bore limits".into();
                Status::Infeasible
            } else if r.policy.mode == WallMode::Unclassified
                || r.policy.imported.as_ref().is_some_and(|w| {
                    w.semantics != WallMode::Informational
                        && !matches!(
                            m.imported_use,
                            Some(WallMode::Required | WallMode::Informational)
                        )
                })
            {
                Status::ReviewRequired
            } else if r.policy.mode == WallMode::Informational {
                Status::Informational
            } else {
                Status::Feasible
            };
            r.comparison = match (r.policy.mode, r.policy.effective) {
                (WallMode::Required, Some(_)) => "Required wall met",
                (WallMode::Informational, Some(v)) => {
                    if r.wall_min < v {
                        "Below informational reference"
                    } else if r.wall_min > v {
                        "Above informational reference"
                    } else {
                        "Equal to informational reference"
                    }
                }
                _ => "No mandatory minimum configured",
            }
            .into();
            r.imported_comparison =
                r.policy
                    .imported
                    .as_ref()
                    .map(|w| match m.imported_use.unwrap_or(w.semantics) {
                        WallMode::Unclassified => "Review required".into(),
                        WallMode::Required => {
                            "Imported requirement retained; active sizing policy shown separately"
                                .into()
                        }
                        WallMode::Informational => if r.wall_min < w.value {
                            "Below imported reference"
                        } else if r.wall_min > w.value {
                            "Above imported reference"
                        } else {
                            "Equal to imported reference"
                        }
                        .into(),
                    });
        } else {
            r.comparison = "Hole only; no bushing constraints".into();
        }
        if !reasons.is_empty() {
            r.enlargement_reason = Some(reasons.join(" + "));
            r.governing = format!(
                "Cleanup hole enlarged for {}",
                r.enlargement_reason.as_deref().unwrap()
            );
        }
        let footprint = envelope(r.z, input.pilot_z, b, r.final_hole.max, input.pilot_offset);
        if r.final_hole.max > m.seating_max
            || footprint[0] < -m.member_width / 2.0
            || footprint[1] > m.member_width / 2.0
            || r.final_hole.max > m.member_depth
        {
            r.status = Status::Infeasible;
            r.governing = "Final hole exceeds machining limit or member body".into();
        }
        if ![
            r.cleanup_diameter,
            r.final_hole.max,
            r.center_x,
            r.bore_capacity,
            r.wall_min,
            r.wall_max,
            r.original_zone_used,
            r.finished_zone_used,
            r.original_displacement,
            r.finished_displacement,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            let mut e = error(
                Some(m.id),
                "numerical range",
                "Nonfinite geometry; reduce extreme dimensions or angles",
            );
            e.kind = ErrorKind::NumericalFailure;
            return Err(e);
        }
    }
    let section = section(input, &layers, length, a[0], b[1]);
    if !section.bounds.iter().flatten().all(|v| v.is_finite())
        || !(section.bounds[1][0] - section.bounds[0][0]).is_finite()
        || section.layers.iter().any(|layer| {
            !layer
                .od
                .iter()
                .chain(layer.original.iter().flatten())
                .chain(layer.finished.iter().flatten())
                .flatten()
                .all(|v| v.is_finite())
        })
    {
        let mut e = error(
            None,
            "section geometry",
            "Section coordinates exceed finite numerical range",
        );
        e.kind = ErrorKind::NumericalFailure;
        return Err(e);
    }
    let solids = solid_geometry(input, &layers, a[0], b[1]);
    Ok(ResultSet {
        solids,
        section,
        original_angles: a,
        finished_angles: b,
        layers,
        length,
    })
}
fn solid_geometry(
    input: &Inputs,
    layers: &[LayerResult],
    original_angle: f64,
    angle: f64,
) -> Vec<SolidLayer> {
    let a = angle.to_radians();
    layers
        .iter()
        .zip(&input.members)
        .map(|(r, m)| {
            let ring = |diameter: f64| {
                r.z.map(|z| {
                    (0..48)
                        .map(|k| {
                            let phi = k as f64 * std::f64::consts::TAU / 48.0;
                            [
                                (z - input.pilot_z) * a.tan()
                                    + input.pilot_offset
                                    + diameter / 2.0 / a.cos() * phi.cos(),
                                diameter / 2.0 * phi.sin(),
                                z,
                            ]
                        })
                        .collect()
                })
            };
            let w = m.member_width / 2.0;
            let d = m.member_depth / 2.0;
            let [start, end] = r.z;
            let oa = original_angle.to_radians();
            let original_hole = r.z.map(|z| {
                (0..48)
                    .map(|k| {
                        let phi = k as f64 * std::f64::consts::TAU / 48.0;
                        [
                            (z - input.pilot_z) * oa.tan()
                                + m.hole.nominal / 2.0 / oa.cos() * phi.cos(),
                            m.hole.nominal / 2.0 * phi.sin(),
                            z,
                        ]
                    })
                    .collect()
            });
            SolidLayer {
                original_hole,
                id: m.id,
                body: [
                    [-w, -d, start],
                    [w, -d, start],
                    [w, d, start],
                    [-w, d, start],
                    [-w, -d, end],
                    [w, -d, end],
                    [w, d, end],
                    [-w, d, end],
                ],
                hole: ring(r.final_hole.nominal),
                bushing_id: input
                    .bushing_enabled
                    .then(|| ring(input.finished_id.nominal)),
                axis: [a.sin(), 0.0, a.cos()],
                z: r.z,
            }
        })
        .collect()
}

fn section(
    input: &Inputs,
    layers: &[LayerResult],
    length: f64,
    original: f64,
    finished: f64,
) -> Section {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let sections = layers
        .iter()
        .zip(&input.members)
        .map(|(r, m)| {
            let [start, end] = r.z;
            let left = (start - input.pilot_z) * finished.to_radians().tan() + input.pilot_offset
                - r.od.nominal / 2.0 / finished.to_radians().cos();
            let right = (start - input.pilot_z) * finished.to_radians().tan()
                + input.pilot_offset
                + r.od.nominal / 2.0 / finished.to_radians().cos();
            let walls = |angle: f64, diameter: f64, shift: f64| {
                [-1.0, 1.0].map(|side| {
                    r.z.map(|z| {
                        [
                            (z - input.pilot_z) * angle.to_radians().tan()
                                + shift
                                + side * diameter / 2.0 / angle.to_radians().cos(),
                            z,
                        ]
                    })
                })
            };
            let shift = (end - start) * finished.to_radians().tan();
            let od = [
                [left, start],
                [right, start],
                [right + shift, end],
                [left + shift, end],
                [left, start],
            ];
            let original = walls(original, m.hole.nominal, 0.0);
            let finished = walls(finished, input.finished_id.nominal, input.pilot_offset);
            for pt in od
                .iter()
                .chain(original.iter().flatten())
                .chain(finished.iter().flatten())
            {
                xmin = xmin.min(pt[0]);
                xmax = xmax.max(pt[0]);
            }
            LayerSection {
                od,
                original,
                finished,
            }
        })
        .collect();
    let axis = |angle: f64, shift: f64| {
        [0.0, length].map(|z| [(z - input.pilot_z) * angle.to_radians().tan() + shift, z])
    };
    Section {
        bounds: [[xmin, 0.0], [xmax, length]],
        layers: sections,
        original_axis: axis(original, 0.0),
        finished_axis: axis(finished, input.pilot_offset),
    }
}

// A cylinder cuts a z plane in an ellipse. Its farthest radial point lies
// on x. The support function has no interior maximum in (-90°,90°).
fn envelope(z: [f64; 2], pilot: f64, angles: [f64; 2], diameter: f64, offset: f64) -> [f64; 2] {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for face in z {
        for deg in angles {
            let angle = deg.to_radians();
            let center = (face - pilot) * angle.tan();
            let radius = diameter / 2.0 / angle.cos();
            lo = lo.min(center - radius - offset);
            hi = hi.max(center + radius + offset);
        }
    }
    [lo, hi]
}
/// Diameter of the smallest cylinder on the common finished axis containing the
/// complete old hole. End faces govern; angle b also has interior stationary cases.
fn cleanup_diameter(
    z: [f64; 2],
    pilot: f64,
    original: [f64; 2],
    finished: [f64; 2],
    hole: f64,
    offset: f64,
) -> f64 {
    let mut radius: f64 = 0.0;
    for face in z {
        for a in original.map(f64::to_radians) {
            for p in [-offset, offset] {
                let span = face - pilot;
                let mut candidates = finished.map(f64::to_radians).to_vec();
                for sign in [-1.0, 1.0] {
                    let x = span * a.tan() - p + sign * hole / 2.0 / a.cos();
                    let mut stationary = (-span).atan2(x);
                    if stationary > std::f64::consts::FRAC_PI_2 {
                        stationary -= std::f64::consts::PI;
                    }
                    if stationary < -std::f64::consts::FRAC_PI_2 {
                        stationary += std::f64::consts::PI;
                    }
                    if stationary >= candidates[0] && stationary <= candidates[1] {
                        candidates.push(stationary);
                    }
                }
                for b in candidates {
                    radius = radius.max(projected_radius(span, a, b, p, hole / 2.0));
                }
            }
        }
    }
    radius * 2.0
}
fn projected_radius(span: f64, a: f64, b: f64, p: f64, r: f64) -> f64 {
    let center = (span * (a.tan() - b.tan()) - p) * b.cos();
    let xradius = r * b.cos() / a.cos();
    let mut squared = (center.abs() + xradius).powi(2);
    if xradius < r {
        let u = center * xradius / (r * r - xradius * xradius);
        if (-1.0..=1.0).contains(&u) {
            squared = squared.max(
                center * center
                    + r * r
                    + 2.0 * center * xradius * u
                    + (xradius * xradius - r * r) * u * u,
            );
        }
    }
    squared.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-11, "{a} != {b}");
    }
    fn parallel() -> Inputs {
        let mut i = Inputs::default();
        i.members.truncate(1);
        i.members[0].original.degrees = 0.0;
        i.members[0].finished.degrees = 0.0;
        i.members[0].od_tolerance = 0.0;
        i
    }
    #[test]
    fn hole_only_ignores_bushing_wall_and_id() {
        let mut i = parallel();
        i.common_wall = Some(-1.0);
        i.members[0].wall_override = Some(-2.0);
        i.members[0].finished_id_min = Some(-3.0);
        i.finished_id = Band::exact(10.0);
        let r = calculate(&i).unwrap();
        near(r.layers[0].cleanup_diameter, 0.3);
        near(r.layers[0].final_hole.min, 0.3);
        assert_eq!(r.layers[0].status, Status::Feasible);
        assert!(r.layers[0].enlargement_reason.is_none());
    }
    #[test]
    fn angularity_cleanup_hand_case() {
        near(
            cleanup_diameter([0.0, 1.0], 0.0, [30.0, 30.0], [0.0, 0.0], 0.4, 0.0),
            2.0 / 3f64.sqrt() + 0.8 / 3f64.sqrt(),
        );
    }
    #[test]
    fn finished_angle_interior_can_govern() {
        near(
            cleanup_diameter([0.0, 0.001], 0.0, [30.0, 30.0], [-10.0, 10.0], 0.4, 0.0),
            0.4630392351986313,
        );
    }
    #[test]
    fn pilot_and_every_member_boundary_participate() {
        let mut i = Inputs::default();
        i.pilot_offset = 0.01;
        let r = calculate(&i).unwrap();
        assert!(r.layers[1].cleanup_diameter > r.layers[0].cleanup_diameter);
        i.pilot_z = 1.0;
        let r = calculate(&i).unwrap();
        assert!(r.layers[0].cleanup_diameter > r.layers[1].cleanup_diameter);
    }
    #[test]
    fn optional_bushing_enlargement_is_explicit() {
        let mut i = parallel();
        i.bushing_enabled = true;
        i.common_wall = Some(0.05);
        let r = calculate(&i).unwrap();
        near(r.layers[0].cleanup_diameter, 0.3);
        near(r.layers[0].final_hole.min, 0.35);
        near(r.layers[0].wall_min, 0.05);
        assert!(r.layers[0]
            .enlargement_reason
            .as_ref()
            .unwrap()
            .contains("bushing"));
    }
    #[test]
    fn uniform_od_rechecks_all_limits() {
        let mut i = Inputs::default();
        i.bushing_enabled = true;
        i.uniform_od = true;
        i.members[0].seating_max = 0.31;
        let r = calculate(&i).unwrap();
        near(r.layers[0].final_hole.min, r.layers[1].final_hole.min);
        assert_eq!(r.layers[0].status, Status::Infeasible);
    }
    #[test]
    fn wall_policy_modes_and_missing_are_distinct() {
        let mut i = parallel();
        i.bushing_enabled = true;
        i.common_wall = None;
        assert_eq!(calculate(&i).unwrap().layers[0].status, Status::Incomplete);
        i.common_wall = Some(0.04);
        i.members[0].wall_override = Some(0.02);
        near(
            calculate(&i).unwrap().layers[0].policy.effective.unwrap(),
            0.02,
        );
        i.members[0].wall_mode = WallMode::Informational;
        i.members[0].wall_override = Some(0.2);
        let r = calculate(&i).unwrap();
        near(r.layers[0].final_hole.min, 0.3);
        assert_eq!(r.layers[0].comparison, "Below informational reference");
    }
    #[test]
    fn unknown_import_requires_explicit_classification() {
        let mut i = parallel();
        i.bushing_enabled = true;
        i.members[0].imported_wall = Some(ImportedWall {
            value: 0.1,
            units: "in".into(),
            workbench: "old".into(),
            source_id: "a".into(),
            semantics: WallMode::Unclassified,
            context: String::new(),
        });
        for classification in [None, Some(WallMode::Unclassified)] {
            i.members[0].imported_use = classification;
            assert_eq!(
                calculate(&i).unwrap().layers[0].status,
                Status::ReviewRequired
            );
        }
        i.members[0].imported_use = Some(WallMode::Informational);
        assert_eq!(calculate(&i).unwrap().layers[0].status, Status::Feasible);
    }
    #[test]
    fn gdandt_zones_and_normals_constrain_common_axes() {
        let mut i = Inputs::default();
        i.mode = AngularityMode::CylindricalZone;
        i.members[1].reference_normal_deg = 10.0;
        assert_eq!(calculate(&i).unwrap_err().kind, ErrorKind::Infeasible);
    }
    #[test]
    fn invalid_inputs_and_lost_thickness_are_rejected() {
        let mut i = Inputs::default();
        i.members[1].id = 1;
        assert!(calculate(&i).is_err());
        i = Inputs::default();
        i.members[0].thickness = 1e20;
        i.members[1].thickness = 0.001;
        assert_eq!(calculate(&i).unwrap_err().kind, ErrorKind::NumericalFailure);
    }
    #[test]
    fn cleanup_bounds_dense_independent_point_projection() {
        for a in [-30.0_f64, 0.0, 25.0] {
            let expected =
                cleanup_diameter([0.0, 0.7], 0.2, [a, a], [-35.0, 40.0], 0.3, 0.01) / 2.0;
            for b_step in 0..=150 {
                let b = (-35.0 + b_step as f64 * 0.5).to_radians();
                for face in [0.0, 0.7] {
                    for p in [-0.01, 0.01] {
                        for k in 0..=180 {
                            let phi = (k as f64 * 2.0).to_radians();
                            let x = (face - 0.2) * a.to_radians().tan()
                                + 0.15 / a.to_radians().cos() * phi.cos()
                                - p;
                            let y = 0.15 * phi.sin();
                            let transverse = x * b.cos() - (face - 0.2) * b.sin();
                            assert!(transverse.hypot(y) <= expected + 1e-12);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn repeatable_results_and_order() {
        let mut i = Inputs::default();
        assert_eq!(calculate(&i), calculate(&i));
        i.members.swap(0, 1);
        let r = calculate(&i).unwrap();
        assert_eq!(r.layers[0].id, 2);
        assert_eq!(r.layers[1].z, [0.5, 1.0]);
    }
}
