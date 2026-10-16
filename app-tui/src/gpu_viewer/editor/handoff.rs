//! The hand-off between the FEA workbench and the two bushing workbenches. A bushing case (or an eccentric one) opens in the
//! FEA workbench as its derived `Problem` (`<case>.fea.json`), whose geometry, bushing and load can be edited there; the FEA
//! workbench sends the edited model back, and the case file takes exactly what changed, so the inputs the FEA model cannot
//! carry (tolerances, countersinks, pin data) stay as they were. Files are the interface (ADR-014): a sidecar
//! (`<case>.fea.link.json`) says which case files belong to an FEA file, and a heartbeat file (`<file>.open`, touched while a
//! window has the file) tells a sender whether a window already shows it.
use super::{
    bushing::{fe as bushing_fe, file::BushingFile},
    eccentric::{fe as eccentric_fe, file::EccentricFile},
};
use crate::gpu_viewer::EditorKind;
use crate::toolboxes::bushing::model::{BushingModel, NumberTarget};
use crate::toolboxes::eccentric_bushing::model::EccentricUi;
use fea_problem::{Geometry, Load, Problem, Shape};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------- files

/// The FEA file of a case: `wing.json` -> `wing.fea.json`.
pub fn fea_path(case: &Path) -> PathBuf {
    case.with_extension("fea.json")
}

/// The case file names an FEA file gets when it has no link yet: `wing.fea.json` -> `wing.bushing.json` / `wing.eccentric.json`.
pub fn default_case(fea: &Path, kind: EditorKind) -> PathBuf {
    let stem = fea.file_name().and_then(|n| n.to_str()).unwrap_or("case");
    let stem = stem.strip_suffix(".json").unwrap_or(stem);
    let stem = stem.strip_suffix(".fea").unwrap_or(stem);
    let tag = if kind == EditorKind::Eccentric { "eccentric" } else { "bushing" };
    fea.with_file_name(format!("{stem}.{tag}.json"))
}

/// Which case files an FEA file belongs to (paths relative to its folder when they sit beside it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    #[serde(default)]
    pub bushing: Option<String>,
    #[serde(default)]
    pub eccentric: Option<String>,
}

fn link_path(fea: &Path) -> PathBuf {
    fea.with_extension("link.json")
}

impl Link {
    pub fn load(fea: &Path) -> Self {
        std::fs::read_to_string(link_path(fea)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn exists(fea: &Path) -> bool {
        link_path(fea).exists()
    }
    pub fn save(&self, fea: &Path) -> Result<(), String> {
        crate::persistence::write(link_path(fea), serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| format!("cannot write {}: {e}", link_path(fea).display()))
    }
    pub(crate) fn rel(fea: &Path, case: &Path) -> String {
        case.strip_prefix(fea.parent().unwrap_or(Path::new(""))).unwrap_or(case).to_string_lossy().into_owned()
    }
    fn resolve(fea: &Path, name: &str) -> PathBuf {
        let p = Path::new(name);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            fea.parent().unwrap_or(Path::new("")).join(p)
        }
    }
    pub fn case(&self, fea: &Path, kind: EditorKind) -> PathBuf {
        let name = if kind == EditorKind::Eccentric { &self.eccentric } else { &self.bushing };
        name.as_deref().map_or_else(|| default_case(fea, kind), |n| Self::resolve(fea, n))
    }
    pub fn set(&mut self, fea: &Path, kind: EditorKind, case: &Path) {
        let name = Some(Self::rel(fea, case));
        if kind == EditorKind::Eccentric {
            self.eccentric = name;
        } else {
            self.bushing = name;
        }
    }
}

/// How long a heartbeat stays fresh.
const BEAT_SECS: u64 = 6;

fn beat_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".open");
    file.with_file_name(name)
}
/// A window has `file` open: touch its heartbeat.
pub fn beat(file: &Path) {
    let _ = crate::persistence::write(beat_path(file), std::process::id().to_string());
}
/// The window closed.
pub fn clear_beat(file: &Path) {
    let _ = std::fs::remove_file(beat_path(file));
}
/// Whether a window touched `file`'s heartbeat recently (so a sender need not open another).
pub fn is_open(file: &Path) -> bool {
    std::fs::metadata(beat_path(file)).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() < BEAT_SECS)
}
/// The file's modification time, for noticing a change made by another window.
pub fn stamp(file: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
thread_local! {
    /// What `launch` was asked to start on this thread (tests start no process).
    pub static LAUNCHED: std::cell::RefCell<Vec<(EditorKind, PathBuf)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Start the window of `kind` on `file` as its own process.
pub fn launch(kind: EditorKind, file: &Path) -> Result<(), String> {
    #[cfg(test)]
    {
        LAUNCHED.with(|l| l.borrow_mut().push((kind, file.to_path_buf())));
        Ok(())
    }
    #[cfg(not(test))]
    launch_process(kind, file)
}

#[cfg(not(test))]
fn launch_process(kind: EditorKind, file: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe).arg(kind.flag()).arg(file).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------- reading a bushing model

/// The part of a bushing-in-housing model both workbenches share.
#[derive(Clone, Debug, PartialEq)]
struct Shared {
    bore_dia: f64,
    bushing_id: f64,
    interference: f64,
    friction: f64,
    housing_material: String,
    bushing_material: String,
    thickness: f64,
    load: f64,
    /// Direction of the pin load, degrees from +x.
    load_dir: f64,
    offset: [f64; 2],
}

fn shared(p: &Problem) -> Result<Shared, String> {
    let Geometry::Sketch { holes, .. } = &p.geometry else { return Err("the FEA model is not a sketch with a bored hole".into()) };
    let Some(Shape::Circle { r, .. }) = holes.first() else { return Err("the FEA model has no circular hole for the bushing".into()) };
    let Some(b) = p.bushings.first() else { return Err("the FEA model has no bushing: add one to the hole first".into()) };
    let (fx, fy) = p.loads.iter().find_map(|l| if let Load::Bearing { hole: 1, fx, fy } = l { Some((*fx, *fy)) } else { None }).unwrap_or((0.0, 0.0));
    Ok(Shared {
        bore_dia: 2.0 * r,
        bushing_id: b.inner_diameter,
        interference: b.interference,
        friction: b.friction,
        housing_material: p.material.name.clone(),
        bushing_material: b.material.name.clone(),
        thickness: p.thickness,
        load: fx.hypot(fy),
        load_dir: fy.atan2(fx).to_degrees(),
        offset: b.offset,
    })
}

/// The housing outline: `(centre of the hole, outer shape)`.
fn housing(p: &Problem) -> Result<([f64; 2], Shape), String> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Err("the FEA model is not a sketch".into()) };
    let Some(Shape::Circle { cx, cy, .. }) = holes.first() else { return Err("the FEA model has no circular hole".into()) };
    Ok(([*cx, *cy], outer.clone()))
}

fn differs(a: f64, b: f64) -> bool {
    (a - b).abs() > 1e-9 * (1.0 + a.abs().max(b.abs()))
}

/// Take what changed between `base` (the model the case derives) and `new` into the bushing model; returns what it did.
fn apply_shared(m: &mut BushingModel, base: &Shared, new: &Shared) -> Vec<String> {
    let mut did = Vec::new();
    let number = |m: &mut BushingModel, t: NumberTarget, was: f64, now: f64, unit: &str, did: &mut Vec<String>| {
        if differs(was, now) {
            m.commit_number(t, now);
            did.push(format!("{} {:.4}{unit}", t.label(), now));
        }
    };
    number(m, NumberTarget::BoreDia, base.bore_dia, new.bore_dia, " in", &mut did);
    number(m, NumberTarget::IdBushing, base.bushing_id, new.bushing_id, " in", &mut did);
    number(m, NumberTarget::Interference, base.interference, new.interference, " in", &mut did);
    number(m, NumberTarget::Friction, base.friction, new.friction, "", &mut did);
    number(m, NumberTarget::HousingLen, base.thickness, new.thickness, " in", &mut did);
    number(m, NumberTarget::Load, base.load, new.load, " lbf", &mut did);
    for (bushing, was, now) in [(false, &base.housing_material, &new.housing_material), (true, &base.bushing_material, &new.bushing_material)] {
        if was == now {
            continue;
        }
        let which = if bushing { "bushing" } else { "housing" };
        match m.material_catalog().iter().position(|c| c.name == now.as_str()) {
            Some(i) => {
                if bushing {
                    m.select_bushing_material(i);
                } else {
                    m.select_housing_material(i);
                }
                did.push(format!("{which} material {now}"));
            }
            None => did.push(format!("{which} material {now} is not in the catalog: it stays {}", if bushing { m.bushing_material().name } else { m.housing_material().name })),
        }
    }
    did
}

/// Take an edited FEA model into a bushing case: the bore, bushing ID, interference, friction, materials, thickness, pin load,
/// and the housing's edge distance and width. `m` is the case's model; only what differs from what that model derives changes.
pub fn bushing_from_problem(m: &mut BushingModel, new: &Problem) -> Result<Vec<String>, String> {
    let mut inch = new.clone();
    inch.convert_units(fea_problem::UnitSystem::InchPsi)?;
    let new = &inch;
    let base = bushing_fe::housing_problem(m)?;
    let (b, n) = (shared(&base)?, shared(new)?);
    let ((bc, bo), (nc, no)) = (housing(&base)?, housing(new)?);
    let rect = |s: &Shape| if let Shape::Rect { x0, y0, y1, .. } = s { Some((*x0, y1 - y0)) } else { None };
    let (Some((bx0, bw)), Some((nx0, nw))) = (rect(&bo), rect(&no)) else { return Err("the housing must be a rectangle for the Bushing workbench (a round boss belongs to the Eccentric Bushing workbench)".into()) };
    let mut did = apply_shared(m, &b, &n);
    let (be, ne) = (bc[0] - bx0, nc[0] - nx0);
    if differs(be, ne) {
        m.commit_number(NumberTarget::EdgeDist, ne);
        did.push(format!("Edge Distance {ne:.4} in"));
    }
    if differs(bw, nw) {
        m.commit_number(NumberTarget::HousingWidth, nw);
        did.push(format!("Housing Width {nw:.4} in"));
    }
    Ok(did)
}

/// The same for the eccentric workbench: the shared inputs go to the bushing model, the offset, load angle and housing to `ui`.
pub fn eccentric_from_problem(m: &mut BushingModel, ui: &mut EccentricUi, new: &Problem) -> Result<Vec<String>, String> {
    let mut inch = new.clone();
    inch.convert_units(fea_problem::UnitSystem::InchPsi)?;
    let new = &inch;
    let base = eccentric_fe::preview_problem(m, ui)?;
    let (b, n) = (shared(&base)?, shared(new)?);
    let ((bc, bo), (nc, no)) = (housing(&base)?, housing(new)?);
    let mut did = apply_shared(m, &b, &n);
    let angle = |s: &Shared| s.offset[1].atan2(s.offset[0]).to_degrees();
    let (bo_len, no_len) = (b.offset[0].hypot(b.offset[1]), n.offset[0].hypot(n.offset[1]));
    if differs(bo_len, no_len) {
        ui.commit_number(crate::toolboxes::eccentric_bushing::model::NumberTarget::Offset, no_len);
        did.push(format!("Bore Offset e {no_len:.4} in"));
    }
    // The load angle is measured from the offset line.
    let (bl, nl) = ((b.load_dir - angle(&b)).rem_euclid(360.0), (n.load_dir - angle(&n)).rem_euclid(360.0));
    if differs(bl, nl) && n.load > 0.0 {
        ui.commit_number(crate::toolboxes::eccentric_bushing::model::NumberTarget::LoadAngle, nl);
        did.push(format!("Load Angle {nl:.1} deg"));
    }
    match (&bo, &no) {
        (Shape::Circle { r: br, .. }, Shape::Circle { r: nr, .. }) => {
            let (was, now) = (2.0 * br / b.bore_dia, 2.0 * nr / n.bore_dia);
            if differs(was, now) {
                ui.commit_number(crate::toolboxes::eccentric_bushing::model::NumberTarget::BossFactor, now);
                did.push(format!("Boss OD / Bore {now:.2}"));
            }
        }
        (Shape::Rect { x0: bx0, .. }, Shape::Rect { x0: nx0, .. }) => {
            let (be, ne) = (bc[0] - bx0, nc[0] - nx0);
            if differs(be, ne) {
                m.commit_number(NumberTarget::EdgeDist, ne);
                did.push(format!("Edge Distance {ne:.4} in"));
            }
        }
        (Shape::Circle { .. }, Shape::Rect { x0, .. }) => {
            let edge = nc[0] - x0;
            ui.edge_limited = true;
            m.commit_number(NumberTarget::EdgeDist, edge);
            did.push(format!("Housing plate, edge distance {edge:.4} in"));
        }
        (Shape::Rect { .. }, Shape::Circle { r, .. }) => {
            ui.edge_limited = false;
            ui.commit_number(crate::toolboxes::eccentric_bushing::model::NumberTarget::BossFactor, 2.0 * r / n.bore_dia);
            did.push("Housing round boss".into());
        }
        _ => return Err("the housing outline is neither a round boss nor a rectangle".into()),
    }
    Ok(did)
}

// ---------------------------------------------------------------------- the two directions

fn read_bushing(case: &Path) -> Result<(BushingModel, String), String> {
    let mut m = BushingModel::default();
    if !case.exists() {
        return Ok((m, String::new()));
    }
    let f = BushingFile::from_json(&std::fs::read_to_string(case).map_err(|e| format!("cannot read {}: {e}", case.display()))?)?;
    f.apply(&mut m);
    Ok((m, f.name))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    crate::persistence::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Open a case in the FEA workbench: its derived `problem` is written beside it as `<case>.fea.json` (an FEA file already
/// there is kept as `.previous.json`), the link is recorded and the FEA window started. A window that already has the FEA file
/// open is left alone: its model is its own until it sends it back.
pub fn open_in_fea(case: &Path, kind: EditorKind, problem: &Problem) -> Result<String, String> {
    let fea = fea_path(case);
    let mut link = Link::load(&fea);
    link.set(&fea, kind, case);
    link.save(&fea)?;
    if is_open(&fea) {
        return Ok(format!("{} is already open in the FEA workbench; its model is not replaced. Use Send to the workbench there to bring its changes back.", fea.display()));
    }
    let json = problem.to_json()?;
    let mut note = String::new();
    if fea.exists() && std::fs::read_to_string(&fea).ok().as_deref() != Some(json.as_str()) {
        let kept = fea.with_extension("previous.json");
        std::fs::rename(&fea, &kept).map_err(|e| format!("cannot keep the earlier {}: {e}", fea.display()))?;
        note = format!(" The earlier FEA file is kept as {}.", kept.display());
    }
    write(&fea, &json)?;
    launch(EditorKind::Solid, &fea)?;
    Ok(format!("Opened {} in the FEA workbench.{note} Edit the bushing there, then Send to the workbench.", fea.display()))
}

/// Whether the FEA model `p` can go to the workbench `kind` (what it would refuse, without sending).
pub fn sendable(p: &Problem, kind: EditorKind) -> Result<(), String> {
    let mut m = BushingModel::default();
    match kind {
        EditorKind::Eccentric => eccentric_from_problem(&mut m, &mut EccentricUi::default(), p).map(|_| ()),
        _ => bushing_from_problem(&mut m, p).map(|_| ()),
    }
}

/// Send the FEA workbench's model to the workbench `kind`: the case file it is linked to (or the default name beside the FEA
/// file) takes what changed, and its window is started, or, when one has the case open, updates itself from the file.
pub fn send_from_fea(fea: &Path, problem: &Problem, kind: EditorKind) -> Result<String, String> {
    let mut link = Link::load(fea);
    let case = link.case(fea, kind);
    let did = match kind {
        EditorKind::Eccentric => {
            let existing = if case.exists() { Some(EccentricFile::from_json(&std::fs::read_to_string(&case).map_err(|e| format!("cannot read {}: {e}", case.display()))?)?) } else { None };
            let bushing_case = existing.as_ref().map_or_else(|| Link::rel(fea, &link.case(fea, EditorKind::Bushing)), |f| f.bushing_case.clone());
            let name = existing.as_ref().map(|f| f.name.clone()).unwrap_or_default();
            let mut ui = existing.as_ref().map_or_else(EccentricUi::default, |f| f.ui());
            let bushing_path = EccentricFile::new("", &bushing_case, &ui).case_path(&case);
            let (mut m, bname) = read_bushing(&bushing_path)?;
            let did = eccentric_from_problem(&mut m, &mut ui, problem)?;
            write(&bushing_path, &BushingFile::capture(&m, &bname).to_json())?;
            write(&case, &EccentricFile::new(&name, &bushing_case, &ui).to_json())?;
            did
        }
        _ => {
            let (mut m, name) = read_bushing(&case)?;
            let did = bushing_from_problem(&mut m, problem)?;
            write(&case, &BushingFile::capture(&m, &name).to_json())?;
            did
        }
    };
    link.set(fea, kind, &case);
    link.save(fea)?;
    let what = if did.is_empty() { "no change".to_string() } else { did.join(", ") };
    if is_open(&case) {
        Ok(format!("Sent to {} ({what}); the open window updates itself.", case.display()))
    } else {
        launch(kind, &case)?;
        Ok(format!("Sent to {} ({what}); opened its workbench.", case.display()))
    }
}

#[cfg(test)]
mod tests;
