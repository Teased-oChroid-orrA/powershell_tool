//! The Plate Edge case: everything the person types or picks, and the two calculations' inputs derived from it
//! (`edge_check::modes::ModesInput` for the allowables, `lug_solver::bracket::Case` for the FE). The case file is these
//! inputs as JSON; results are never stored, they are recomputed.
//!
//! Units are the window's: inches and lbf, strengths in ksi (the material library's), pressures in psi.
use edge_check::modes::{ModesInput, PinSpec};
use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use lug_solver::bracket::{Case, Housing, Idealisation};
use mechanics_core::materials::{builtin_catalog, get_material, Material};
use serde::{Deserialize, Serialize};

const KSI: f64 = 1000.0;
const FORMAT_VERSION: u32 = 1;
/// The classical shear-out planes' inclination (MMPDS lug practice), degrees.
const PLANE_ANGLE: f64 = 40.0;

/// Where a strength set comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Library,
    Custom,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Library => "Library",
            Source::Custom => "Custom",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Source::Library => Source::Custom,
            Source::Custom => Source::Library,
        }
    }
}

/// The fit between pin and hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitKind {
    /// Hole larger than the pin; `fit_value` is the diametral clearance.
    Clearance,
    /// Light interference; `fit_value` is the diametral interference.
    #[default]
    PressFit,
    /// Heavy interference; `fit_value` is the diametral interference.
    Interference,
    /// Only the contact pressure is known; `fit_value` is that pressure, psi.
    Custom,
}

impl FitKind {
    pub const ALL: [FitKind; 4] = [FitKind::Clearance, FitKind::PressFit, FitKind::Interference, FitKind::Custom];
    pub fn label(self) -> &'static str {
        match self {
            FitKind::Clearance => "Clearance",
            FitKind::PressFit => "Press-fit",
            FitKind::Interference => "Interference",
            FitKind::Custom => "Custom pressure",
        }
    }
    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|k| *k == self).unwrap_or(0) + 1) % 4]
    }
    /// What the value row means and its typical starting value.
    pub fn value_label(self) -> &'static str {
        match self {
            FitKind::Clearance => "Diametral clearance",
            FitKind::PressFit | FitKind::Interference => "Diametral interference",
            FitKind::Custom => "Contact pressure",
        }
    }
    pub fn unit(self) -> &'static str {
        if self == FitKind::Custom {
            " psi"
        } else {
            " in"
        }
    }
    pub fn default_value(self) -> f64 {
        match self {
            FitKind::Clearance => 0.002,
            FitKind::PressFit => 0.0008,
            FitKind::Interference => 0.003,
            FitKind::Custom => 5000.0,
        }
    }
}

/// How the FE meshes are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshMode {
    /// One mesh graded about the bore (the default).
    #[default]
    Graded,
    /// Finer meshes until the collapse load moves less than the tolerance.
    Converge,
    /// One fixed fine mesh (a reference run).
    Fine,
}

impl MeshMode {
    pub fn label(self) -> &'static str {
        match self {
            MeshMode::Graded => "Graded (one mesh)",
            MeshMode::Converge => "Graded + converge",
            MeshMode::Fine => "Fixed fine",
        }
    }
    pub fn next(self) -> Self {
        match self {
            MeshMode::Graded => MeshMode::Converge,
            MeshMode::Converge => MeshMode::Fine,
            MeshMode::Fine => MeshMode::Graded,
        }
    }
}

/// Strengths and elastic constants typed by the person (ksi, as the library tables are).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomMaterial {
    pub e_ksi: f64,
    pub nu: f64,
    pub fty_ksi: f64,
    pub ftu_ksi: f64,
    pub fsu_ksi: f64,
    /// Bearing ultimate / yield at e/D 2.0 and 1.5 (`0` = not known).
    pub fbru_ksi: f64,
    pub fbru15_ksi: f64,
    pub fbry_ksi: f64,
    pub fbry15_ksi: f64,
}

impl Default for CustomMaterial {
    fn default() -> Self {
        // 7075-T651 plate, the library default's neighbour: a starting point to edit, not a recommendation.
        Self { e_ksi: 10300.0, nu: 0.33, fty_ksi: 67.0, ftu_ksi: 78.0, fsu_ksi: 44.0, fbru_ksi: 142.0, fbru15_ksi: 0.0, fbry_ksi: 110.0, fbry15_ksi: 0.0 }
    }
}

/// A fastener class from a specification: the numbers the specification rates (ultimate tension and double-shear), no more.
/// Yield and bearing are not specified for a bolt, so a class never supplies them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinClass {
    pub label: &'static str,
    pub e_ksi: f64,
    pub nu: f64,
    pub ftu_ksi: f64,
    pub fsu_ksi: f64,
}

/// Specification ratings (verify against the allowables of the program).
pub const PIN_CLASSES: [PinClass; 3] = [
    PinClass { label: "Alloy steel bolt, 160 ksi (NAS6200 series)", e_ksi: 29000.0, nu: 0.30, ftu_ksi: 160.0, fsu_ksi: 95.0 },
    PinClass { label: "Alloy steel bolt, 180 ksi", e_ksi: 29000.0, nu: 0.30, ftu_ksi: 180.0, fsu_ksi: 108.0 },
    PinClass { label: "A286 CRES bolt, 160 ksi (NAS6300 series)", e_ksi: 29100.0, nu: 0.31, ftu_ksi: 160.0, fsu_ksi: 95.0 },
];

/// Where the pin's strengths come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinSource {
    /// A specification class (`PIN_CLASSES[pin_class]`): ratings only.
    #[default]
    Class,
    /// A library material (`pin_material`): gives yield and bearing as well.
    Material,
    Custom,
}

impl PinSource {
    pub fn label(self) -> &'static str {
        match self {
            PinSource::Class => "Library: fastener class",
            PinSource::Material => "Library: material",
            PinSource::Custom => "Custom",
        }
    }
    pub fn next(self) -> Self {
        match self {
            PinSource::Class => PinSource::Material,
            PinSource::Material => PinSource::Custom,
            PinSource::Custom => PinSource::Class,
        }
    }
}

/// Nominal fastener sizes: the major diameters of the NAS6200-series catalogue.
pub fn sizes() -> impl Iterator<Item = (&'static str, f64)> {
    fastened_joint_solver::thread_catalog::NAS_UNJF_BOLT_CATALOG.iter().map(|e| (e.designation, e.major_diameter))
}

/// Everything that defines a case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlateEdgeInputs {
    pub plate_source: Source,
    /// `Material::id` of the plate (library).
    pub plate_material: String,
    pub plate_custom: CustomMaterial,
    pub pin_source: PinSource,
    pub pin_class: usize,
    /// `Material::id` of a pin taken from the material library.
    pub pin_material: String,
    pub pin_custom: CustomMaterial,
    /// Index into [`sizes`]; ignored for a custom pin, which has its own diameter.
    pub size: usize,
    /// A custom pin's diameter, in.
    pub pin_dia: f64,
    /// Shear planes carrying the load, 1 or 2.
    pub planes: u8,
    /// Distance between the bearing resultants (bending moment arm), in.
    pub span: f64,
    pub fit: FitKind,
    /// Clearance or interference (in) or the custom pressure (psi), by `fit`.
    pub fit_value: f64,
    pub friction: f64,
    /// Hole centre to the free edge, in.
    pub edge: f64,
    pub thickness: f64,
    pub mesh: MeshMode,
    /// Elements around the bore of the first (or only) mesh.
    pub around: usize,
    /// Hex27 layers over half the thickness of the first (or only) 3D mesh.
    pub layers: usize,
    /// Convergence tolerance on the FE loads, percent.
    pub tol_percent: f64,
}

impl Default for PlateEdgeInputs {
    fn default() -> Self {
        Self {
            plate_source: Source::Library,
            plate_material: "al7075".into(),
            plate_custom: CustomMaterial::default(),
            pin_source: PinSource::Class,
            pin_class: 0,
            pin_material: "steel4340".into(),
            pin_custom: CustomMaterial { e_ksi: 29000.0, nu: 0.30, fty_ksi: 0.0, ftu_ksi: 160.0, fsu_ksi: 95.0, fbru_ksi: 0.0, fbru15_ksi: 0.0, fbry_ksi: 0.0, fbry15_ksi: 0.0 },
            size: 1,
            pin_dia: 0.25,
            planes: 2,
            span: 0.5,
            fit: FitKind::PressFit,
            fit_value: FitKind::PressFit.default_value(),
            friction: 0.12,
            edge: 0.5,
            thickness: 0.125,
            mesh: MeshMode::Graded,
            around: 36,
            layers: 2,
            tol_percent: 0.5,
        }
    }
}

/// A numeric input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Edge,
    Thickness,
    FitValue,
    Friction,
    PinDia,
    Planes,
    Span,
    Around,
    Layers,
    Tolerance,
    Plate(Prop),
    Pin(Prop),
}

/// A property of a custom material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    E,
    Nu,
    Fty,
    Ftu,
    Fsu,
    Fbru,
    Fbru15,
    Fbry,
    Fbry15,
}

impl Prop {
    pub const ALL: [Prop; 9] = [Prop::E, Prop::Nu, Prop::Fty, Prop::Ftu, Prop::Fsu, Prop::Fbru, Prop::Fbru15, Prop::Fbry, Prop::Fbry15];
    pub fn label(self) -> &'static str {
        match self {
            Prop::E => "Modulus E",
            Prop::Nu => "Poisson ratio",
            Prop::Fty => "Fty (tension yield)",
            Prop::Ftu => "Ftu (tension ultimate)",
            Prop::Fsu => "Fsu (shear ultimate)",
            Prop::Fbru => "Fbru at e/D 2.0",
            Prop::Fbru15 => "Fbru at e/D 1.5",
            Prop::Fbry => "Fbry at e/D 2.0",
            Prop::Fbry15 => "Fbry at e/D 1.5",
        }
    }
    pub fn unit(self) -> &'static str {
        if self == Prop::Nu {
            ""
        } else {
            " ksi"
        }
    }
    fn get(self, m: &CustomMaterial) -> f64 {
        match self {
            Prop::E => m.e_ksi,
            Prop::Nu => m.nu,
            Prop::Fty => m.fty_ksi,
            Prop::Ftu => m.ftu_ksi,
            Prop::Fsu => m.fsu_ksi,
            Prop::Fbru => m.fbru_ksi,
            Prop::Fbru15 => m.fbru15_ksi,
            Prop::Fbry => m.fbry_ksi,
            Prop::Fbry15 => m.fbry15_ksi,
        }
    }
    fn slot(self, m: &mut CustomMaterial) -> &mut f64 {
        match self {
            Prop::E => &mut m.e_ksi,
            Prop::Nu => &mut m.nu,
            Prop::Fty => &mut m.fty_ksi,
            Prop::Ftu => &mut m.ftu_ksi,
            Prop::Fsu => &mut m.fsu_ksi,
            Prop::Fbru => &mut m.fbru_ksi,
            Prop::Fbru15 => &mut m.fbru15_ksi,
            Prop::Fbry => &mut m.fbry_ksi,
            Prop::Fbry15 => &mut m.fbry15_ksi,
        }
    }
    /// A property that may be left at zero (not known).
    fn optional(self) -> bool {
        matches!(self, Prop::Fbru15 | Prop::Fbry15 | Prop::Fbru | Prop::Fbry | Prop::Fty)
    }
}

impl Field {
    pub fn label(self) -> String {
        match self {
            Field::Edge => "Edge distance e".into(),
            Field::Thickness => "Plate thickness t".into(),
            Field::FitValue => "Fit value".into(),
            Field::Friction => "Pin-hole friction".into(),
            Field::PinDia => "Pin diameter".into(),
            Field::Planes => "Shear planes".into(),
            Field::Span => "Bending span".into(),
            Field::Around => "Elements around bore".into(),
            Field::Layers => "Layers, half thickness".into(),
            Field::Tolerance => "Convergence tolerance".into(),
            Field::Plate(p) | Field::Pin(p) => p.label().into(),
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            Field::Edge | Field::Thickness | Field::PinDia | Field::Span => " in",
            Field::Tolerance => " %",
            Field::Plate(p) | Field::Pin(p) => p.unit(),
            _ => "",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Field::Edge => "Hole centre to the free edge. Must exceed the hole radius.",
            Field::Thickness => "Plate thickness at the hole. Must be positive.",
            Field::FitValue => "Clearance or interference is diametral, in inches; a custom fit is the contact pressure in psi.",
            Field::Friction => "Coulomb coefficient between pin and hole, 0 to 1. The validation tests agree best with 0.",
            Field::PinDia => "Shank diameter of a custom pin.",
            Field::Planes => "1 for single shear, 2 for double shear.",
            Field::Span => "Distance between the bearing resultants: the clevis span (double shear) or the mean of the two plate thicknesses (single).",
            Field::Around => "Elements around the full bore circle of the first mesh (18 to 144).",
            Field::Layers => "Hex27 layers over half the thickness of the first 3D mesh (1 to 6). A layer costs far more than a ring element.",
            Field::Tolerance => "A convergence run stops when the collapse load changes by less than this between two meshes.",
            Field::Plate(_) | Field::Pin(_) => "A strength in ksi; zero where the property is not known (rows that need it say so).",
        }
    }
}

fn catalog_find(id: &str) -> Option<&'static Material> {
    builtin_catalog().find(|m| m.id == id)
}

/// The library materials whose id, name or alloy contains `text`, in catalogue order.
pub fn search(text: &str) -> Vec<&'static Material> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Vec::new();
    }
    builtin_catalog()
        .filter(|m| m.id.to_lowercase().contains(&t) || m.name.to_lowercase().contains(&t) || m.extra.is_some_and(|x| x.alloy.to_lowercase().contains(&t) || x.temper.to_lowercase().contains(&t)))
        .collect()
}

/// A short description of a library material for the picker.
pub fn describe(m: &Material) -> String {
    match m.extra {
        Some(x) => format!("{} {} {} ({})", x.alloy, x.form, x.temper, m.id),
        None => format!("{} ({})", m.name, m.id),
    }
}

impl PlateEdgeInputs {
    /// The plate's library material (the default if the file names one this build does not have).
    pub fn plate_library(&self) -> &'static Material {
        catalog_find(&self.plate_material).unwrap_or_else(|| get_material("al7075"))
    }
    pub fn pin_library(&self) -> &'static Material {
        catalog_find(&self.pin_material).unwrap_or_else(|| get_material("steel4340"))
    }
    pub fn pin_class_spec(&self) -> &'static PinClass {
        &PIN_CLASSES[self.pin_class.min(PIN_CLASSES.len() - 1)]
    }

    /// The fastener size's designation and diameter (library pins).
    pub fn size_entry(&self) -> (&'static str, f64) {
        let all: Vec<_> = sizes().collect();
        all[self.size.min(all.len() - 1)]
    }
    /// Pin diameter, in.
    pub fn pin_diameter(&self) -> f64 {
        if self.pin_source == PinSource::Custom {
            self.pin_dia
        } else {
            self.size_entry().1
        }
    }

    /// The fit as a signed diametral interference (positive presses the pin in) and the custom pressure.
    pub fn delta_and_pressure(&self) -> (f64, Option<f64>) {
        match self.fit {
            FitKind::Clearance => (-self.fit_value.abs(), None),
            FitKind::PressFit | FitKind::Interference => (self.fit_value.abs(), None),
            FitKind::Custom => (0.0, Some(self.fit_value.abs())),
        }
    }

    /// Hole diameter: the pin's less the interference (plus the clearance).
    pub fn hole_diameter(&self) -> f64 {
        self.pin_diameter() - self.delta_and_pressure().0
    }

    /// Plate strengths in psi, and its bearing-yield table values (`Fbry` at e/D 2.0 and 1.5).
    pub fn plate_strengths(&self) -> (Strengths, f64, f64) {
        match self.plate_source {
            Source::Library => {
                let m = self.plate_library();
                let (fbry, fbry15) = m.extra.map_or((0.0, 0.0), |x| (x.fbry_e20_ksi * KSI, x.fbry_e15_ksi * KSI));
                (Strengths::from_material(m), fbry, fbry15)
            }
            Source::Custom => {
                let c = &self.plate_custom;
                let s = Strengths { e: c.e_ksi * KSI, nu: c.nu, sy: c.fty_ksi * KSI, fsu: c.fsu_ksi * KSI, ftu: c.ftu_ksi * KSI, fbru: if c.fbru_ksi > 0.0 { c.fbru_ksi } else { c.fty_ksi } * KSI, fbru_e15: c.fbru15_ksi * KSI };
                (s, c.fbry_ksi * KSI, c.fbry15_ksi * KSI)
            }
        }
    }

    pub fn pin_spec(&self) -> PinSpec {
        let some = |v: f64| (v > 0.0).then_some(v * KSI);
        let (e, nu, ftu, fsu, fty, fbru) = match self.pin_source {
            PinSource::Class => {
                let c = self.pin_class_spec();
                (c.e_ksi * KSI, c.nu, c.ftu_ksi * KSI, c.fsu_ksi * KSI, None, None)
            }
            PinSource::Material => {
                let m = self.pin_library();
                let s = Strengths::from_material(m);
                (s.e, s.nu, s.ftu, s.fsu, some(m.sy_ksi), some(m.fbru_ksi))
            }
            PinSource::Custom => {
                let c = &self.pin_custom;
                (c.e_ksi * KSI, c.nu, c.ftu_ksi * KSI, c.fsu_ksi * KSI, some(c.fty_ksi), some(c.fbru_ksi))
            }
        };
        PinSpec { dia: self.pin_diameter(), e, nu, ftu, fsu, fty, fbru, planes: self.planes, span: self.span }
    }

    /// The allowables' input, or why the inputs are not one.
    pub fn modes_input(&self, deep: bool) -> Result<ModesInput, String> {
        let (plate, fbry, fbry_e15) = self.plate_strengths();
        let (delta, custom_pressure) = self.delta_and_pressure();
        let pin = self.pin_spec();
        Ok(ModesInput { bore_dia: self.hole_diameter(), edge: self.edge, thickness: self.thickness, plate, fbry, fbry_e15, pin, delta, custom_pressure, friction: self.friction, plane_angle_deg: PLANE_ANGLE, deep })
    }

    /// Why the FE bracket cannot model this fit, if it cannot: its pin is rigid and only a bushing's interference is modelled.
    pub fn bracket_refusal(&self) -> Option<String> {
        let (delta, pressure) = self.delta_and_pressure();
        (delta > 0.0 || pressure.is_some_and(|p| p > 0.0)).then(|| "the 2D/3D bracket FE models a clearance or line-to-line pin (a rigid pin in the hole); an interference fit is in the contact FE of the 2D allowables".to_string())
    }

    /// The FE bracket's case, or why there is none.
    pub fn bracket_case(&self) -> Result<Case, String> {
        if let Some(why) = self.bracket_refusal() {
            return Err(why);
        }
        self.picture_case()
    }

    /// The plate as the FE meshes it (the picture of the inputs uses it whatever the fit).
    pub fn picture_case(&self) -> Result<Case, String> {
        let d = self.hole_diameter();
        if !(self.edge > 0.5 * d) {
            return Err("the edge distance must exceed the hole radius".into());
        }
        let (plate, _, _) = self.plate_strengths();
        let reach = (3.0 * self.edge).max(5.0 * d);
        Ok(Case {
            dia: d,
            pin_clearance: (d - self.pin_diameter()).max(0.0),
            ed: self.edge / d,
            td: self.thickness / d,
            width: 2.0 * reach,
            length: reach,
            around: self.around,
            housing: Housing { e: plate.e, nu: plate.nu, flow: flow_stress(&plate), law: None },
            bush: None,
        })
    }

    pub fn solid(&self) -> Idealisation {
        Idealisation::Solid { layers: self.layers, lock_z: false }
    }

    /// Value of a numeric field.
    pub fn get(&self, f: Field) -> f64 {
        match f {
            Field::Edge => self.edge,
            Field::Thickness => self.thickness,
            Field::FitValue => self.fit_value,
            Field::Friction => self.friction,
            Field::PinDia => self.pin_dia,
            Field::Planes => f64::from(self.planes),
            Field::Span => self.span,
            Field::Around => self.around as f64,
            Field::Layers => self.layers as f64,
            Field::Tolerance => self.tol_percent,
            Field::Plate(p) => p.get(&self.plate_custom),
            Field::Pin(p) => p.get(&self.pin_custom),
        }
    }

    /// Sets a numeric field, or says why the value is not one.
    pub fn set(&mut self, f: Field, v: f64) -> Result<(), String> {
        if !v.is_finite() {
            return Err(format!("{} must be a number", f.label()));
        }
        let positive = |what: &str| if v > 0.0 { Ok(()) } else { Err(format!("{what} must be positive")) };
        match f {
            Field::Edge => {
                positive("The edge distance")?;
                self.edge = v;
            }
            Field::Thickness => {
                positive("The plate thickness")?;
                self.thickness = v;
            }
            Field::FitValue => {
                if v < 0.0 {
                    return Err("The fit value must not be negative (the fit type says which way it acts)".into());
                }
                self.fit_value = v;
            }
            Field::Friction => {
                if !(0.0..=1.0).contains(&v) {
                    return Err("Friction must be between 0 and 1".into());
                }
                self.friction = v;
            }
            Field::PinDia => {
                positive("The pin diameter")?;
                self.pin_dia = v;
            }
            Field::Planes => {
                if v != 1.0 && v != 2.0 {
                    return Err("Shear planes must be 1 or 2".into());
                }
                self.planes = v as u8;
            }
            Field::Span => {
                positive("The bending span")?;
                self.span = v;
            }
            Field::Around => {
                if !(18.0..=144.0).contains(&v) || v.fract() != 0.0 {
                    return Err("Elements around the bore must be a whole number from 18 to 144".into());
                }
                self.around = v as usize;
            }
            Field::Layers => {
                if !(1.0..=6.0).contains(&v) || v.fract() != 0.0 {
                    return Err("Layers must be a whole number from 1 to 6".into());
                }
                self.layers = v as usize;
            }
            Field::Tolerance => {
                if !(0.01..=10.0).contains(&v) {
                    return Err("The tolerance must be between 0.01 and 10 percent".into());
                }
                self.tol_percent = v;
            }
            Field::Plate(p) | Field::Pin(p) => {
                let custom = if matches!(f, Field::Plate(_)) { &mut self.plate_custom } else { &mut self.pin_custom };
                if v < 0.0 || (v == 0.0 && !p.optional()) {
                    return Err(format!("{} must be positive", p.label()));
                }
                if p == Prop::Nu && v >= 0.5 {
                    return Err("Poisson ratio must be below 0.5".into());
                }
                *p.slot(custom) = v;
            }
        }
        Ok(())
    }

    /// The next fit type, with its typical value.
    pub fn cycle_fit(&mut self) {
        self.fit = self.fit.next();
        self.fit_value = self.fit.default_value();
    }

    pub fn cycle_size(&mut self) {
        self.size = (self.size + 1) % sizes().count();
    }

    // ------------------------------------------------------------------ file

    pub fn to_json(&self, name: &str) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            version: u32,
            name: &'a str,
            #[serde(flatten)]
            inputs: &'a PlateEdgeInputs,
        }
        serde_json::to_string_pretty(&File { version: FORMAT_VERSION, name, inputs: self }).expect("plain data serializes")
    }

    /// The inputs and the case's name from file text.
    pub fn from_json(text: &str) -> Result<(Self, String), String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            version: u32,
            #[serde(default)]
            name: String,
            #[serde(flatten)]
            inputs: PlateEdgeInputs,
        }
        let f: File = serde_json::from_str(text).map_err(|e| format!("not a plate edge case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        let mut i = f.inputs;
        i.pin_class = i.pin_class.min(PIN_CLASSES.len() - 1);
        i.size = i.size.min(sizes().count() - 1);
        i.planes = if i.planes == 1 { 1 } else { 2 };
        Ok((i, f.name))
    }
}
