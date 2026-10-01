//! Common bolted-joint stack-ups for the Preload Analysis toolbox: a set of
//! preset templates (bolt + washers + plates + nut variations) and a seeded
//! random generator, both producing a [`JointTemplate`] that
//! [`PreloadModel::apply_template`] turns into ordinary editable model
//! fields (members, bearing radii, shank length, nut data, preload/torque).
//!
//! Pure data and arithmetic - no ratatui. Every dimension a template fills
//! in (washer sizes, hole clearance, typical torque) is a *typical value*,
//! not a drawing: the user can hand-edit all of it afterwards, exactly like
//! values filled in by the bolt picker.

use fastened_joint_solver::thread_catalog::{BoltCatalogEntry, AN_BOLT_CATALOG};
use fastened_joint_solver::solve::TighteningMember;

use super::model::{MemberUi, PreloadModel, MAX_MEMBERS};

/// A structural material for stack members: name and elastic modulus (psi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub name: &'static str,
    pub e_psi: f64,
}

pub const AL_7075: Material = Material { name: "Al 7075-T6", e_psi: 10_300_000.0 };
pub const AL_2024: Material = Material { name: "Al 2024-T3", e_psi: 10_500_000.0 };
pub const AL_6061: Material = Material { name: "Al 6061-T6", e_psi: 10_000_000.0 };
pub const TI_6AL_4V: Material = Material { name: "Ti-6Al-4V", e_psi: 16_000_000.0 };
pub const STEEL_4130: Material = Material { name: "Steel 4130", e_psi: 29_000_000.0 };
pub const STAINLESS: Material = Material { name: "Stainless 17-4PH", e_psi: 28_500_000.0 };
pub const CFRP: Material = Material { name: "CFRP laminate", e_psi: 7_700_000.0 };

pub const MATERIALS: [Material; 7] = [AL_7075, AL_2024, AL_6061, TI_6AL_4V, STEEL_4130, STAINLESS, CFRP];

/// Name of the catalogued material whose modulus matches `e` (1%), for labels.
pub fn material_name(e: f64) -> String {
    MATERIALS.iter().find(|m| (m.e_psi - e).abs() / m.e_psi < 0.01).map(|m| m.name.to_string()).unwrap_or_else(|| format!("E {:.1} Msi", e / 1.0e6))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerRole {
    Washer,
    Plate,
    Shim,
    Doubler,
    Fitting,
}

impl LayerRole {
    pub fn name(self) -> &'static str {
        match self {
            LayerRole::Washer => "Washer",
            LayerRole::Plate => "Plate",
            LayerRole::Shim => "Shim",
            LayerRole::Doubler => "Doubler",
            LayerRole::Fitting => "Fitting",
        }
    }
}

/// One clamped layer, listed head side first. A washer's `thickness` of
/// `0.0` means "the typical washer thickness for the bolt size".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub role: LayerRole,
    pub thickness: f64,
    pub material: Material,
}

const fn plate(t: f64, m: Material) -> Layer {
    Layer { role: LayerRole::Plate, thickness: t, material: m }
}
const fn washer(m: Material) -> Layer {
    Layer { role: LayerRole::Washer, thickness: 0.0, material: m }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JointTemplate {
    pub name: String,
    pub description: String,
    /// Index into `AN_BOLT_CATALOG`; `None` keeps the model's current bolt.
    pub bolt_index: Option<usize>,
    pub layers: Vec<Layer>,
    pub tightening_from: TighteningMember,
    /// Thread and bearing friction coefficient.
    pub mu: f64,
}

/// Typical flat-washer dimensions for a bolt of major diameter `d` (in):
/// `(inner diameter, outer diameter, thickness)` - AN960-style proportions.
pub fn washer_dims(d: f64) -> (f64, f64, f64) {
    let id = ((d + 0.016) * 1000.0).round() / 1000.0;
    let od = ((d * 2.17) * 1000.0).round() / 1000.0;
    let t = if d >= 0.4375 { 0.090 } else { 0.063 };
    (id, od, t)
}

/// Tensile stress area of a UN thread (in^2).
fn tensile_stress_area(d: f64, pitch: f64) -> f64 {
    let dm = d - 0.9743 * pitch;
    std::f64::consts::FRAC_PI_4 * dm * dm
}

/// The preset templates. They keep the model's current bolt.
pub fn presets() -> Vec<JointTemplate> {
    let two = |name: &str, desc: &str, layers: Vec<Layer>, from: TighteningMember| JointTemplate { name: name.to_string(), description: desc.to_string(), bolt_index: None, layers, tightening_from: from, mu: 0.15 };
    vec![
        two(
            "2 plates, washers both sides",
            "Bolt, washer under the head, two clamped plates, washer under the nut, nut. The standard aircraft lap-joint stack-up.",
            vec![washer(STEEL_4130), plate(0.125, AL_7075), plate(0.125, AL_7075), washer(STEEL_4130)],
            TighteningMember::Nut,
        ),
        two(
            "2 plates, washer under nut only",
            "Bolt head bears directly on the plate (e.g. a flush or protruding head on a thick skin); a washer protects the nut-side surface.",
            vec![plate(0.125, AL_7075), plate(0.190, AL_7075), washer(STEEL_4130)],
            TighteningMember::Nut,
        ),
        two(
            "2 plates, washer under head only",
            "Washer under the bolt head, nut bearing directly on the far plate (self-locking nut on a machined face).",
            vec![washer(STEEL_4130), plate(0.125, AL_2024), plate(0.125, AL_2024)],
            TighteningMember::Nut,
        ),
        two(
            "2 plates, no washers",
            "Head and nut both bear directly on the plates. Highest bearing pressure under the head and nut - check the Bearing contact pressure in Results.",
            vec![plate(0.160, AL_7075), plate(0.160, AL_7075)],
            TighteningMember::Nut,
        ),
        two(
            "2 plates + shim, washers both sides",
            "Two plates with a thin gap-filling shim between them (a common rigging/fit-up detail), washers both sides.",
            vec![washer(STEEL_4130), plate(0.125, AL_7075), Layer { role: LayerRole::Shim, thickness: 0.032, material: AL_6061 }, plate(0.125, AL_7075), washer(STEEL_4130)],
            TighteningMember::Nut,
        ),
        two(
            "Skin + doubler + fitting, washers both sides",
            "Thin skin riding on a doubler over a thick machined fitting - three dissimilar-thickness layers under one bolt.",
            vec![washer(STEEL_4130), plate(0.063, AL_2024), Layer { role: LayerRole::Doubler, thickness: 0.063, material: AL_2024 }, Layer { role: LayerRole::Fitting, thickness: 0.250, material: AL_7075 }, washer(STEEL_4130)],
            TighteningMember::Nut,
        ),
        two(
            "Aluminum to titanium fitting, Ti washers",
            "Aluminum plate bolted to a titanium fitting. Titanium washers keep the bearing faces compatible; the much stiffer titanium raises the joint stiffness.",
            vec![washer(TI_6AL_4V), plate(0.125, AL_7075), Layer { role: LayerRole::Fitting, thickness: 0.250, material: TI_6AL_4V }, washer(TI_6AL_4V)],
            TighteningMember::Nut,
        ),
        two(
            "Thin skin on thick fitting, head turned",
            "Installed by turning the bolt head (blind nut-plate side): washer under the head only, nut held.",
            vec![washer(STEEL_4130), plate(0.050, AL_2024), Layer { role: LayerRole::Fitting, thickness: 0.375, material: AL_7075 }],
            TighteningMember::BoltHead,
        ),
    ]
}

/// Small deterministic generator (SplitMix64) - no dependency, and the same
/// seed always rebuilds the same candidate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

const PLATE_THICKNESSES: [f64; 13] = [0.040, 0.050, 0.063, 0.071, 0.080, 0.090, 0.100, 0.125, 0.160, 0.190, 0.250, 0.312, 0.375];
const FRICTIONS: [f64; 5] = [0.10, 0.12, 0.15, 0.18, 0.20];

/// A random but physically sensible joint: 2-3 plates (maybe a shim and a
/// doubler/fitting), washers on 0-2 sides, an AN6-AN12-ish bolt, grip length
/// 1-4.5 bolt diameters, one of a handful of typical materials and friction
/// values. Same `seed` = same joint.
pub fn random(seed: u64) -> JointTemplate {
    let mut rng = Rng(seed ^ 0xA5A5_5A5A_1234_5678);
    // AN4..AN10: sizes where the typical washers/torques below make sense.
    let bolt_index = 1 + rng.below(6);
    let d = AN_BOLT_CATALOG[bolt_index].major_diameter;

    let plates = if rng.chance(60) { 2 } else { 3 };
    let head_washer = rng.chance(75);
    let nut_washer = rng.chance(85);
    let washer_mat = if rng.chance(15) { TI_6AL_4V } else { STEEL_4130 };
    let structural = [AL_7075, AL_2024, AL_6061, TI_6AL_4V, STEEL_4130, CFRP];

    // Re-roll thicknesses until the grip is a sensible multiple of the diameter.
    let (mut layers, mut grip) = (Vec::new(), 0.0);
    for _ in 0..400 {
        layers = Vec::new();
        if head_washer {
            layers.push(washer(washer_mat));
        }
        for i in 0..plates {
            let t = rng.pick(&PLATE_THICKNESSES);
            let role = if i == plates - 1 && t >= 0.25 { LayerRole::Fitting } else if i > 0 && t <= 0.071 { LayerRole::Doubler } else { LayerRole::Plate };
            layers.push(Layer { role, thickness: t, material: rng.pick(&structural) });
            if i + 1 < plates && rng.chance(20) {
                layers.push(Layer { role: LayerRole::Shim, thickness: rng.pick(&[0.016, 0.020, 0.032]), material: AL_6061 });
            }
        }
        if nut_washer {
            layers.push(washer(washer_mat));
        }
        grip = layers.iter().filter(|l| l.role != LayerRole::Washer).map(|l| l.thickness).sum::<f64>();
        if (1.0 * d..=4.5 * d).contains(&grip) && layers.len() <= MAX_MEMBERS {
            break;
        }
    }
    if !(1.0 * d..=4.5 * d).contains(&grip) {
        // Practically unreachable; keep the joint valid rather than odd.
        let others: f64 = layers.iter().filter(|l| l.role != LayerRole::Washer).rev().skip(1).map(|l| l.thickness).sum();
        if let Some(last) = layers.iter_mut().rev().find(|l| l.role != LayerRole::Washer) {
            last.thickness = (((2.0 * d - others).max(0.04)) * 1000.0).round() / 1000.0;
        }
        grip = layers.iter().filter(|l| l.role != LayerRole::Washer).map(|l| l.thickness).sum::<f64>();
    }
    let tightening_from = if rng.chance(80) { TighteningMember::Nut } else { TighteningMember::BoltHead };
    let mu = rng.pick(&FRICTIONS);

    let desc_layers: Vec<String> = layers.iter().filter(|l| l.role != LayerRole::Washer).map(|l| format!("{} {:.3} {}", l.role.name().to_lowercase(), l.thickness, l.material.name)).collect();
    let washers = match (head_washer, nut_washer) {
        (true, true) => "washers under head and nut",
        (true, false) => "washer under head only",
        (false, true) => "washer under nut only",
        (false, false) => "no washers",
    };
    JointTemplate {
        name: format!("Random: {} plates, {}", plates, washers),
        description: format!(
            "{}; {washers}; grip {grip:.3} in ({:.1} x diameter); {} installation; friction {mu:.2}.",
            desc_layers.join(" + "),
            grip / d,
            if tightening_from == TighteningMember::Nut { "nut-turned" } else { "head-turned" }
        ),
        bolt_index: Some(bolt_index),
        layers,
        tightening_from,
        mu,
    }
}

impl PreloadModel {
    /// Replaces the member stack, bearing geometry, shank length, nut data,
    /// friction and tightening member from `template`, sized to the bolt
    /// (the template's own, or the current one), and re-solves. Typical
    /// torque/preload (K = 0.2, 50% of a 125 ksi proof stress on the tensile
    /// area) are filled in too so the result is immediately meaningful;
    /// everything stays an ordinary editable field.
    pub fn apply_template(&mut self, template: &JointTemplate) {
        if let Some(i) = template.bolt_index {
            if let Some(entry) = AN_BOLT_CATALOG.get(i) {
                self.select_bolt(entry);
            }
        }
        let d = self.thread_major_dia;
        let (w_id, w_od, w_t) = washer_dims(d);
        let hole = ((d + 0.003) * 10_000.0).round() / 10_000.0;

        self.members = template
            .layers
            .iter()
            .take(MAX_MEMBERS)
            .map(|l| match l.role {
                LayerRole::Washer => MemberUi { name: l.role.name(), thickness: if l.thickness > 0.0 { l.thickness } else { w_t }, e: l.material.e_psi, hole_diameter: w_id, outer_diameter: w_od },
                _ => MemberUi { name: l.role.name(), thickness: l.thickness, e: l.material.e_psi, hole_diameter: hole, outer_diameter: 0.0 },
            })
            .collect();

        // Bearing faces: a washer's own annulus, else the head/nut bearing face (~1.3 d across).
        let face = |m: &MemberUi| if m.name == LayerRole::Washer.name() { (m.hole_diameter / 2.0, m.outer_diameter / 2.0) } else { (m.hole_diameter / 2.0, 0.65 * d) };
        if let (Some(head), Some(nut)) = (self.members.first(), self.members.last()) {
            let (hi, ho) = face(head);
            let (ni, no) = face(nut);
            self.head_bearing_inner_radius = hi;
            self.head_bearing_outer_radius = ho;
            self.bearing_inner_radius = ni;
            self.bearing_outer_radius = no;
        }

        let grip: f64 = self.members.iter().map(|m| m.thickness).sum();
        self.shank_length = grip;
        self.tightening_from = template.tightening_from;
        self.mu_thread = template.mu;
        self.mu_bearing = template.mu;
        self.engaged_threads = (0.8 * d / self.thread_pitch).round().max(3.0);
        self.nut_outer_diameter = (1.73 * d * 1000.0).round() / 1000.0;

        let force = 0.5 * 125_000.0 * tensile_stress_area(d, self.thread_pitch);
        self.target_preload = force.round();
        self.applied_torque = (0.2 * d * force).round();
        self.recompute();
    }
}

/// One row of the stack picture: a bar glyph run and its caption.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagramRow {
    pub bar: String,
    pub caption: String,
    pub is_washer: bool,
}

/// Head-to-nut picture of the clamped stack for any model (templates and
/// hand-built stacks alike): `┌──┐ bolt head`, one bar per member (washers
/// narrow and light, plates wide), `└──┘ nut`.
pub fn stack_diagram(model: &PreloadModel) -> Vec<DiagramRow> {
    let mut rows = Vec::new();
    let head = if model.tightening_from == TighteningMember::BoltHead { "bolt head (turned)" } else { "bolt head" };
    rows.push(DiagramRow { bar: "     \u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}     ".to_string(), caption: head.to_string(), is_washer: false });
    for m in &model.members {
        let is_washer = m.name == LayerRole::Washer.name();
        let (glyph, width) = match m.name {
            "Washer" => ('\u{2580}', 8),
            "Shim" => ('\u{2591}', 14),
            "Doubler" => ('\u{2592}', 16),
            "Fitting" => ('\u{2588}', 16),
            _ => ('\u{2593}', 16),
        };
        let bar: String = std::iter::repeat(glyph).take(width).collect();
        let pad = " ".repeat((18 - width) / 2);
        rows.push(DiagramRow { bar: format!("{pad}{bar}{pad}"), caption: format!("{} {:.3} in, {}", m.name, m.thickness, material_name(m.e)), is_washer });
    }
    let nut = if model.tightening_from == TighteningMember::Nut { "nut (turned)" } else { "nut" };
    rows.push(DiagramRow { bar: "     \u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}     ".to_string(), caption: nut.to_string(), is_washer: false });
    rows
}

pub fn bolt_label(entry: &BoltCatalogEntry) -> &'static str {
    entry.designation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_applies_and_solves_with_sane_geometry() {
        for t in presets() {
            let mut m = PreloadModel::default();
            m.apply_template(&t);
            assert!(m.output.is_ok(), "{}: {:?}", t.name, m.output);
            assert!(m.members.len() <= MAX_MEMBERS && m.members.len() == t.layers.len(), "{}", t.name);
            assert!(m.applied_torque > 0.0 && m.target_preload > 0.0);
            let d = m.thread_major_dia;
            for mem in &m.members {
                assert!(mem.hole_diameter > d, "{}: hole {} must clear the bolt {d}", t.name, mem.hole_diameter);
                assert!(mem.thickness > 0.0);
            }
            assert!(m.bearing_outer_radius > m.bearing_inner_radius && m.head_bearing_outer_radius > m.head_bearing_inner_radius, "{}", t.name);
        }
    }

    #[test]
    fn the_headline_template_is_washer_plate_plate_washer() {
        let mut m = PreloadModel::default();
        m.apply_template(&presets()[0]);
        let names: Vec<_> = m.members.iter().map(|x| x.name).collect();
        assert_eq!(names, ["Washer", "Plate", "Plate", "Washer"]);
        let (id, od, _) = washer_dims(m.thread_major_dia);
        assert_eq!(m.members[0].hole_diameter, id);
        assert_eq!(m.members[0].outer_diameter, od);
        assert_eq!(m.bearing_outer_radius, od / 2.0, "nut bears on its washer");
    }

    #[test]
    fn presets_keep_the_current_bolt_and_random_sets_its_own() {
        let mut m = PreloadModel::default();
        m.select_bolt(&AN_BOLT_CATALOG[5]); // AN8
        m.apply_template(&presets()[0]);
        assert!((m.thread_major_dia - 0.5).abs() < 1e-9, "preset must not change the bolt");
        let r = random(7);
        m.apply_template(&r);
        let want = AN_BOLT_CATALOG[r.bolt_index.unwrap()].major_diameter;
        assert!((m.thread_major_dia - want).abs() < 1e-9);
    }

    #[test]
    fn random_is_deterministic_per_seed_and_varies_across_seeds() {
        assert_eq!(random(42), random(42));
        let distinct: std::collections::HashSet<String> = (0..40u64).map(|s| random(s).description).collect();
        assert!(distinct.len() > 30, "only {} distinct joints in 40 seeds", distinct.len());
    }

    #[test]
    fn random_joints_are_always_valid_and_analyzable() {
        for seed in 0..300u64 {
            let t = random(seed);
            let mut m = PreloadModel::default();
            m.apply_template(&t);
            assert!(m.output.is_ok(), "seed {seed}: {} -> {:?}", t.description, m.output);
            assert!(m.members.len() >= 2 && m.members.len() <= MAX_MEMBERS, "seed {seed}");
            let d = m.thread_major_dia;
            let grip: f64 = m.members.iter().filter(|x| x.name != "Washer").map(|x| x.thickness).sum();
            assert!(grip >= 0.9 * d && grip <= 4.7 * d, "seed {seed}: grip {grip} vs d {d}");
        }
    }

    #[test]
    fn diagram_has_head_every_member_and_nut() {
        let mut m = PreloadModel::default();
        m.apply_template(&presets()[0]);
        let rows = stack_diagram(&m);
        assert_eq!(rows.len(), m.members.len() + 2);
        assert!(rows.first().unwrap().caption.starts_with("bolt head"));
        assert!(rows.last().unwrap().caption.starts_with("nut"));
        assert!(rows[1].is_washer && !rows[2].is_washer);
        assert!(rows.iter().all(|r| r.bar.chars().count() == 18));
    }

    #[test]
    fn washer_dims_scale_with_the_bolt() {
        let (id, od, t) = washer_dims(0.375);
        assert!(id > 0.375 && od > 2.0 * 0.375 && t == 0.063);
        assert_eq!(washer_dims(0.5).2, 0.090);
    }
}
