//! Native Bushing case construction from the calculated member machining result.
use super::bushing::file::BushingFile;
use crate::toolboxes::bushing::model::BushingModel;
use bushing_solver::stackup::{self, Band, Inputs, Status, WallMode};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StackupTransfer {
    pub source_file: String,
    pub member_id: u64,
    pub source_revision: u64,
    pub inputs: Inputs,
    pub angularity_cleanup: f64,
    pub adopted_hole: Band,
    pub enlargement_reason: Option<String>,
    pub representative_pilot_offset: f64,
}
pub fn prepare(
    inputs: &Inputs,
    index: usize,
    source: &str,
    revision: u64,
) -> Result<BushingFile, String> {
    if !inputs.bushing_enabled {
        return Err("Enable optional bushing sizing before transfer".into());
    }
    let results =
        stackup::calculate(inputs).map_err(|e| format!("{}: {}", e.parameter, e.message))?;
    let r = results.layers.get(index).ok_or("Member no longer exists")?;
    let m = &inputs.members[index];
    if !matches!(r.status, Status::Feasible | Status::Informational) {
        return Err("Only current feasible geometry can transfer".into());
    }
    if results.finished_angles != [0.0, 0.0] {
        return Err("The structural target is a plane model; oblique axes or angular uncertainty cannot transfer losslessly".into());
    }
    if inputs.finished_id.min != inputs.finished_id.max {
        return Err("The structural target does not represent an ID tolerance band".into());
    }
    let mut model = BushingModel::default();
    model.bore_dia = r.final_hole.nominal;
    model.bore_tol_plus = r.final_hole.max - r.final_hole.nominal;
    model.bore_tol_minus = r.final_hole.nominal - r.final_hole.min;
    model.id_bushing = inputs.finished_id.nominal;
    model.interference = 0.0015;
    model.interference_tol_plus = m.od_tolerance;
    model.interference_tol_minus = 0.0;
    model.housing_len = m.thickness;
    model.housing_width = m.member_depth;
    model.edge_dist = m.member_width / 2.0 - inputs.pilot_offset;
    model.min_wall_straight = if m.wall_mode == WallMode::Required {
        r.policy.effective.ok_or("Mandatory wall missing")?
    } else {
        0.0
    };
    let catalog = model.material_catalog();
    model.housing_material_index = catalog
        .iter()
        .position(|v| v.id == m.housing_material)
        .ok_or("Unknown housing material")?;
    model.bushing_material_index = catalog
        .iter()
        .position(|v| v.id == m.bushing_material)
        .ok_or("Unknown bushing material")?;
    model.recompute();
    if model.output.bore_tol.nominal != r.final_hole.nominal
        || model.id_bushing != inputs.finished_id.nominal
    {
        return Err("Target altered calculated dimensions".into());
    }
    let layout = super::bushing::fe::layout(&model)?;
    if layout.height != m.member_depth {
        return Err("Target structural model would change member depth; use the FEA workbench for this geometry".into());
    }
    let mut file = BushingFile::capture(&model, &format!("Stackup member #{}", m.id));
    file.stackup = Some(StackupTransfer {
        source_file: source.into(),
        member_id: m.id,
        source_revision: revision,
        inputs: inputs.clone(),
        angularity_cleanup: r.cleanup_diameter,
        adopted_hole: r.final_hole,
        enlargement_reason: r.enlargement_reason.clone(),
        representative_pilot_offset: -inputs.pilot_offset,
    });
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_bushing_transfer_preserves_hole_and_policy() {
        let mut i = Inputs::default();
        i.bushing_enabled = true;
        i.members.truncate(1);
        i.members[0].member_width = 1.4;
        i.members[0].member_depth = 1.1;
        i.pilot_offset = 0.02;
        i.members[0].original.degrees = 0.0;
        i.members[0].finished.degrees = 0.0;
        for mode in [WallMode::Required, WallMode::Informational] {
            i.members[0].wall_mode = mode;
            let file = prepare(&i, 0, "source", 4).unwrap();
            let back = BushingFile::from_json(&file.to_json()).unwrap();
            assert_eq!(back.stackup.as_ref().unwrap().inputs, i);
            let mut model = BushingModel::default();
            assert!(back.apply(&mut model).is_empty());
            assert_eq!(
                model.bore_dia,
                back.stackup.as_ref().unwrap().adopted_hole.nominal
            );
            assert_eq!(
                model.min_wall_straight,
                if mode == WallMode::Required {
                    0.025
                } else {
                    0.0
                }
            );
        }
        i.bushing_enabled = false;
        assert!(prepare(&i, 0, "source", 4).is_err());
    }
    #[test]
    fn fea_roundtrip_keeps_machining_provenance() {
        use crate::gpu_viewer::{
            editor::{bushing::fe, handoff},
            EditorKind,
        };
        let dir = tempfile::tempdir().unwrap();
        let bp = dir.path().join("bushing.json");
        let fp = dir.path().join("linked.json");
        let mut i = Inputs::default();
        i.bushing_enabled = true;
        i.members.truncate(1);
        i.members[0].original.degrees = 0.0;
        i.members[0].finished.degrees = 0.0;
        let file = prepare(&i, 0, "source", 7).unwrap();
        std::fs::write(&bp, file.to_json()).unwrap();
        let mut model = BushingModel::default();
        file.apply(&mut model);
        let problem = fe::housing_problem(&model).unwrap();
        let mut link = handoff::Link::default();
        link.set(&fp, EditorKind::Bushing, &bp);
        link.save(&fp).unwrap();
        handoff::send_from_fea(&fp, &problem, EditorKind::Bushing).unwrap();
        assert_eq!(
            BushingFile::from_json(&std::fs::read_to_string(bp).unwrap())
                .unwrap()
                .stackup,
            file.stackup
        );
    }
}
