//! New, separate cases for the nominal hole. Source and existing target files are never replaced.

use super::super::{bushing::file::BushingFile, handoff, preload::file::PreloadFile};
use crate::gpu_viewer::EditorKind;
use crate::toolboxes::{bushing::model::{BushingModel, NumberTarget as BushingNumber}, fastener_hole::{domain::TolerancedValue, model::{FastenerHoleModel, HoleType}}, preload_analysis::model::PreloadModel};
use std::{io::{ErrorKind, Write}, path::{Path, PathBuf}};

fn hole(model: &FastenerHoleModel) -> Result<TolerancedValue, String> {
    let value = match model.hole_type {
        HoleType::Regular => model.regular.hole1,
        HoleType::Countersunk => model.countersink_primary.as_ref().map_err(|e| format!("primary countersink is invalid: {e}"))?.hole_diameter,
    };
    if value.min <= 0.0 { return Err("hole diameter must be positive at every tolerance extreme".into()); }
    Ok(value)
}

fn write_new(source: &Path, suffix: &str, json: &str) -> Result<PathBuf, String> {
    let dir = source.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for n in 1..=100 {
        let extension = if n == 1 { format!("{suffix}.json") } else { format!("{suffix}-{n}.json") };
        let path = source.with_extension(extension);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(json.as_bytes()).and_then(|_| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("cannot write {}: {e}", path.display()));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        }
    }
    Err(format!("no unused {suffix} case name beside {}", source.display()))
}

pub fn send(model: &FastenerHoleModel, source: &Path, name: &str, kind: EditorKind) -> Result<String, String> {
    let bore = hole(model)?;
    let (suffix, json, note) = match kind {
        EditorKind::Bushing => {
            let mut target = BushingModel::default();
            target.set_number(BushingNumber::BoreDia, bore.nominal);
            target.set_number(BushingNumber::BoreTolMinus, bore.tol_minus());
            target.set_number(BushingNumber::BoreTolPlus, bore.tol_plus());
            if let Some(edge) = model.edge_distance { target.set_number(BushingNumber::EdgeDist, edge.nominal); }
            if let Some(plate) = model.countersink.plate_thickness.filter(|_| model.hole_type == HoleType::Countersunk) { target.set_number(BushingNumber::HousingLen, plate.nominal); }
            target.recompute();
            ("bushing", BushingFile::capture(&target, name).to_json(), "Review bushing ID and fit. Countersink and edge tolerances remain in the source case.")
        }
        EditorKind::Preload => {
            let mut target = PreloadModel::default();
            target.members[0].hole_diameter = bore.nominal;
            if let Some(plate) = model.countersink.plate_thickness.filter(|_| model.hole_type == HoleType::Countersunk) { target.members[0].thickness = plate.nominal; }
            target.recompute();
            ("preload", PreloadFile::capture(&target, name).to_json(), "Review bolt size and joint stack. Only the primary nominal hole transfers; tolerances and companion geometry remain in the source case.")
        }
        _ => return Err("unsupported transfer target".into()),
    };
    let path = write_new(source, suffix, &json)?;
    handoff::launch(kind, &path).map_err(|e| format!("case saved at {}, but the window did not open: {e}", path.display()))?;
    Ok(format!("Opened {}. {note}", path.display()))
}
