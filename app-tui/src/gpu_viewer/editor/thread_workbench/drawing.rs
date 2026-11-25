//! Illustrative assembled thread geometry. Strength calculations stay in thread_capability.
use crate::gpu_viewer::{editor::drawing::Mesh, Scene};
use fastened_joint_solver::thread_capability::{Case, Family};
use std::f64::consts::{PI, TAU};

const PARENT: f32 = 0.5;
const INSERT: f32 = 1.0;
const BOLT: f32 = 2.0;
const CUT: f32 = 3.0;
const FACETS: usize = 72;
const HELIX_STEPS: usize = 18;

fn cylinder(mesh: &mut Mesh, radius: f64, bottom: f64, top: f64, code: f32) {
    let rows: Vec<_> = (0..=FACETS)
        .map(|i| {
            let a = TAU * i as f64 / FACETS as f64;
            let (x, z) = (radius * a.cos(), radius * a.sin());
            vec![[x, bottom, z], [x, top, z]]
        })
        .collect();
    mesh.grid(&rows, |_| code);
    for y in [bottom, top] {
        let center = mesh.positions.len() as u32;
        mesh.positions.push([0.0, y, 0.0]);
        mesh.values.push(code);
        let rim = mesh.positions.len() as u32;
        for i in 0..=FACETS {
            let a = TAU * i as f64 / FACETS as f64;
            mesh.positions.push([radius * a.cos(), y, radius * a.sin()]);
            mesh.values.push(code);
        }
        for i in 0..FACETS as u32 {
            mesh.triangles.extend([center, rim + i, rim + i + 1]);
        }
    }
}

/// A diamond-section wire swept on a helix. The two halves are appended separately so the near side can be translucent.
fn wire_half(
    mesh: &mut Mesh,
    pitch: f64,
    top: f64,
    turn: usize,
    half: usize,
    inner: f64,
    outer: f64,
    code: f32,
) {
    let middle = (inner + outer) / 2.0;
    let axial_half = (pitch * 0.28).min((outer - inner) * 0.62);
    let rows: Vec<_> = (0..=HELIX_STEPS)
        .map(|step| {
            let a = TAU * turn as f64 + PI * (half as f64 + step as f64 / HELIX_STEPS as f64);
            let y = top - pitch * a / TAU;
            let at = |r: f64, y: f64| [r * a.cos(), y, r * a.sin()];
            vec![
                at(inner, y),
                at(middle, y + axial_half),
                at(outer, y),
                at(middle, y - axial_half),
                at(inner, y),
            ]
        })
        .collect();
    mesh.grid(&rows, |_| code);
}

fn threaded_body(
    mesh: &mut Mesh,
    minor: f64,
    major: f64,
    pitch: f64,
    height: f64,
    outer: f64,
    top: f64,
    body_code: f32,
    section_code: f32,
) {
    // The bore and the insert share one helical phase. A revolved zigzag makes circular grooves
    // that miss the wire by half a pitch on the far wall.
    let steps = ((height / pitch) * 24.0).ceil().max(24.0) as usize;
    let angles: Vec<_> = (0..=FACETS)
        .map(|i| 1.75 * PI + 1.5 * PI * i as f64 / FACETS as f64)
        .collect();
    let bore = |a: f64, y: f64| {
        let phase = ((top - y) / pitch - a / TAU).rem_euclid(1.0);
        let valley = 1.0 - 2.0 * phase.min(1.0 - phase);
        minor + (major - minor) * valley
    };
    let point = |r: f64, y: f64, a: f64| [r * a.cos(), y, r * a.sin()];
    let profile: Vec<_> = angles
        .iter()
        .map(|&a| {
            let mut row = vec![
                point(outer, 0.0, a),
                point(outer, height, a),
                point(bore(a, height), height, a),
            ];
            row.extend((0..steps).rev().map(|i| {
                let y = height * i as f64 / steps as f64;
                point(bore(a, y), y, a)
            }));
            row.push(point(outer, 0.0, a));
            row
        })
        .collect();
    mesh.grid(&profile, |_| body_code);
    for &a in [angles[0], angles[FACETS]].iter() {
        let section: Vec<_> = (0..=steps)
            .map(|i| {
                let y = height * i as f64 / steps as f64;
                vec![point(bore(a, y), y, a), point(outer, y, a)]
            })
            .collect();
        mesh.grid(&section, |_| section_code);
    }
}

fn key(mesh: &mut Mesh, radius: f64, height: f64, angle: f64) {
    let width = height * 0.09;
    let y0 = height * 0.48;
    let y1 = height * 1.035;
    let z0 = -width;
    let z1 = width;
    let x0 = radius - width / 2.0;
    let x1 = radius + width / 2.0;
    let mut p = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    for v in &mut p {
        let (x, z) = (v[0], v[2]);
        v[0] = x * angle.cos() - z * angle.sin();
        v[2] = x * angle.sin() + z * angle.cos();
    }
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(p);
    mesh.values.extend([INSERT; 8]);
    for [a, b, c, d] in [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [3, 2, 6, 7],
        [0, 3, 7, 4],
        [1, 5, 6, 2],
    ] {
        mesh.triangles
            .extend([base + a, base + b, base + c, base + a, base + c, base + d]);
    }
}

/// Builds the view and index where the near-side helical wire starts.
pub fn scene(baseline: &Case, shown: &Case, label: &str) -> Result<(Scene, Option<u32>), String> {
    let pitch = 1.0 / baseline.tpi;
    let height = shown.inner.engagement.min(12.0 * pitch).max(pitch);
    let free = 4.0 * pitch;
    let top = height + free;
    let turns = ((top / pitch).ceil() as usize).min(16);
    let mut mesh = Mesh::default();
    let mut translucent = None;
    match shown.family {
        Family::Tapped => {
            let root = shown.inner.minor / 2.0;
            let major = shown.inner.major / 2.0;
            let minor = shown.inner.female_minor / 2.0;
            cylinder(&mut mesh, root, -pitch * 0.25, top + pitch * 0.4, BOLT);
            cylinder(
                &mut mesh,
                major * 1.55,
                top + pitch * 0.35,
                top + pitch * 0.9,
                BOLT,
            );
            threaded_body(
                &mut mesh,
                minor,
                major + 0.003 * pitch,
                pitch,
                height,
                major * 1.9,
                top,
                PARENT,
                CUT,
            );
            for turn in 0..turns {
                for half in 0..2 {
                    wire_half(&mut mesh, pitch, top, turn, half, root, major, BOLT);
                }
            }
        }
        Family::HelicalCoil => {
            let sti = shown
                .sti
                .or(baseline.sti)
                .ok_or("STI dimensions required")?;
            let inner = baseline.nominal_major / 2.0;
            let outer = sti.minor / 2.0 + 0.52 * pitch;
            threaded_body(
                &mut mesh,
                sti.minor / 2.0,
                outer + 0.003 * pitch,
                pitch,
                height,
                outer + baseline.nominal_major * 0.35,
                top,
                PARENT,
                CUT,
            );
            for turn in 0..turns {
                wire_half(&mut mesh, pitch, top, turn, 0, inner, outer, INSERT);
            }
            translucent = Some(mesh.triangles.len() as u32);
            for turn in 0..turns {
                wire_half(&mut mesh, pitch, top, turn, 1, inner, outer, INSERT);
            }
        }
        Family::Keensert => {
            let (outer_tpi, outer_thread) = shown
                .outer
                .as_ref()
                .ok_or("external insert dimensions required")?;
            let outer_root = outer_thread.minor / 2.0;
            let outer_major = outer_thread.major / 2.0;
            let outer_pitch = 1.0 / outer_tpi;
            let parent_height = height.min(outer_thread.engagement);
            threaded_body(
                &mut mesh,
                outer_thread.female_minor / 2.0,
                outer_major + 0.003 * outer_pitch,
                outer_pitch,
                parent_height,
                outer_major * 1.45,
                parent_height,
                PARENT,
                CUT,
            );
            threaded_body(
                &mut mesh,
                shown.inner.female_minor / 2.0,
                shown.inner.female_minor / 2.0 + 0.15 * pitch,
                pitch,
                parent_height,
                outer_root,
                parent_height,
                INSERT,
                INSERT,
            );
            for i in 0..4 {
                key(
                    &mut mesh,
                    outer_root + 0.35 * (outer_major - outer_root),
                    parent_height,
                    i as f64 * PI / 2.0,
                );
            }
            let turns = ((parent_height / outer_pitch).ceil() as usize).min(16);
            for turn in 0..turns {
                wire_half(
                    &mut mesh,
                    outer_pitch,
                    parent_height,
                    turn,
                    0,
                    outer_root,
                    outer_major,
                    INSERT,
                );
            }
            translucent = Some(mesh.triangles.len() as u32);
            for turn in 0..turns {
                wire_half(
                    &mut mesh,
                    outer_pitch,
                    parent_height,
                    turn,
                    1,
                    outer_root,
                    outer_major,
                    INSERT,
                );
            }
        }
    }
    let scene = mesh.into_scene(
        format!("{label} | assembled Unified thread"),
        "part (see color key)",
        [0.0, 3.0],
        Vec::new(),
    )?;
    Ok((scene, translucent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastened_joint_solver::thread_capability::{Interface, StiMeasurement};

    #[test]
    fn all_thread_families_have_valid_assembled_geometry() {
        let mut case = super::super::File::new().case;
        for family in [Family::Tapped, Family::HelicalCoil, Family::Keensert] {
            case.family = family;
            case.sti = (family == Family::HelicalCoil).then_some(StiMeasurement {
                pitch: 0.2765,
                minor: 0.2646,
            });
            case.outer =
                (family == Family::Keensert).then_some((16.0, Interface::basic(0.375, 16.0, 0.37)));
            let (picture, translucent) = scene(&case, &case, "thread").unwrap();
            assert!(picture.positions.len() > 1000);
            assert!(picture.positions.iter().any(|p| p[2].abs() > 0.1));
            assert_eq!(picture.positions.len(), picture.samples.len());
            assert_eq!(translucent.is_some(), family != Family::Tapped);
            if let Some(first) = translucent {
                assert!(first > 0 && (first as usize) < picture.triangles.len());
            }
        }
    }

    #[test]
    fn bolt_damage_does_not_resize_an_installed_coil() {
        let mut baseline = super::super::File::new().case;
        baseline.family = Family::HelicalCoil;
        baseline.sti = Some(StiMeasurement {
            pitch: 0.2765,
            minor: 0.2646,
        });
        let mut damaged = baseline.clone();
        damaged.inner.major -= 0.005;
        let (original, _) = scene(&baseline, &baseline, "thread").unwrap();
        let (changed, _) = scene(&baseline, &damaged, "thread").unwrap();
        assert_eq!(original.positions, changed.positions);
    }
}
