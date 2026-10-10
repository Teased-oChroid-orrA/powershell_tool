//! Bounded, verified free vibration after instantaneous removal of a static mechanical load.
use super::{progress::GenerationProgress, scene::SAMPLE_BUDGET, Scene};
use fea_core::report::{AcceptanceReport, TransientEnergy};
use fea_core::{Loads, ModalOptions, Model, TransientOptions};
use fea_problem::Problem;
pub fn run(p: &Problem, import: Option<&str>) -> Result<Scene, String> {
    run_with_progress(p, import, &|_| {})
}

pub fn run_with_progress(
    p: &Problem,
    import: Option<&str>,
    progress: &dyn Fn(GenerationProgress),
) -> Result<Scene, String> {
    progress(GenerationProgress::stage("Validating animation inputs"));
    p.validate()?;
    if !p.bushings.is_empty()
        || p.delta_t != 0.0
        || !p.material.density.is_finite()
        || p.material.density <= 0.0
    {
        return Err("free vibration needs positive mass density, mechanical loads and no contact bushings or thermal load".into());
    }
    progress(GenerationProgress::stage("Meshing animation model"));
    let mut mesh = fea_problem::solve::preview(p, import)?;
    let stride = display_stride(mesh.nodes.len(), 240)?;
    let dim = mesh.dim();
    let span = (0..3)
        .map(|a| {
            let lo = mesh
                .nodes
                .iter()
                .map(|p| p[a])
                .fold(f64::INFINITY, f64::min);
            let hi = mesh
                .nodes
                .iter()
                .map(|p| p[a])
                .fold(f64::NEG_INFINITY, f64::max);
            hi - lo
        })
        .fold(0.0, f64::max);
    let bc = fea_problem::solve::apply_supports(&mesh, &p.supports)?;
    if bc.fixed.iter().zip(&bc.value).any(|(f, v)| *f && *v != 0.0) {
        return Err("free vibration requires homogeneous supports".into());
    }
    let loads = fea_problem::solve::build_loads(&mut mesh, p)?;
    mesh.set_density_all(p.material.density)?;
    let model = Model::new(mesh)?;
    progress(GenerationProgress::stage("Computing fundamental frequency"));
    let modal = model.modal(
        &bc,
        &ModalOptions {
            n_modes: 1,
            ..Default::default()
        },
    )?;
    let omega = modal
        .modes
        .first()
        .ok_or("no fundamental mode")?
        .omega2
        .sqrt();
    if !omega.is_finite() || omega <= 0.0 {
        return Err("invalid fundamental frequency".into());
    }
    progress(GenerationProgress::stage("Solving initial loaded state"));
    let initial =
        model.solve_adaptive(&loads, &bc, &fea_core::strategy::Requirements::default())?;
    let zero = vec![0.0; initial.solution.u.len()];
    let values: Vec<f64> = initial
        .solution
        .u
        .chunks_exact(dim)
        .map(|u| u.iter().fold(0.0_f64, |a, v| a.hypot(*v)))
        .collect();
    let mut scene = Scene::from_frames(
        &model.mesh, &[initial.solution.u.clone()], vec![0.0], &values,
        format!("{} | verified unloaded free vibration | seconds | initial displacement magnitude | display stride {} (240 solver steps)", p.name, stride),
    )?;
    let result = model.transient_observed(
        &Loads::default(),
        &|_| 0.0,
        &bc,
        Some((&initial.solution.u, &zero)),
        &TransientOptions {
            dt: 2.0 * std::f64::consts::TAU / omega / 240.0,
            steps: 240,
            // Snapshot streaming preserves every integration/acceptance step without f64 history duplication.
            record: Vec::new(),
            ..Default::default()
        },
        &mut |step, time, u| {
            progress(GenerationProgress {
                stage: "Integrating free vibration",
                completed: step,
                total: 240,
            });
            if step > 0 && (step % stride == 0 || step == 240) {
                scene.append_displacements(u, dim, span, time as f32)?;
            }
            Ok(())
        },
    )?;
    progress(GenerationProgress::stage("Checking residuals and energy"));
    AcceptanceReport::transient(
        &result,
        1e-8,
        Some(TransientEnergy::Conserved { tol: 1e-6 }),
    )
    .require()?;
    let peak = scene
        .samples
        .iter()
        .flat_map(|s| s[..3].iter())
        .map(|x| x.abs())
        .fold(0.0, f32::max);
    scene.gain = if peak > 0.0 {
        (0.08 / peak).min(1e12)
    } else {
        1.0
    };
    scene.validate()?;
    progress(GenerationProgress::stage("Opening native viewport"));
    Ok(scene)
}

/// Only display sampling adapts. Time integration and acceptance diagnostics retain every step.
fn display_stride(nodes: usize, steps: usize) -> Result<usize, String> {
    if nodes == 0 || nodes > 500_000 {
        return Err("animation geometry exceeds the 500,000-node viewport limit; refine a smaller region or coarsen the mesh".into());
    }
    let frames = (SAMPLE_BUDGET / nodes).min(steps + 1);
    if frames < 2 {
        return Err("animation cannot fit two display frames; coarsen the mesh".into());
    }
    Ok(steps.div_ceil(frames - 1).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn load_release_produces_verified_physical_frames() {
        let (_, mut p) = fea_problem::templates::templates()
            .into_iter()
            .nth(1)
            .unwrap();
        p.material.density = 1.0;
        let scene = run(&p, None).unwrap();
        assert_eq!(scene.times.len(), 241);
        assert!(scene.times[240] > 0.0);
        assert_ne!(
            &scene.samples[..scene.positions.len()],
            &scene.samples[scene.positions.len()..2 * scene.positions.len()]
        );
        assert!(scene.label.contains("verified unloaded free vibration"));
    }
    #[test]
    fn missing_mass_and_thermal_release_are_refused() {
        let (_, mut p) = fea_problem::templates::templates()
            .into_iter()
            .next()
            .unwrap();
        p.material.density = 0.0;
        assert!(run(&p, None).is_err());
        p.material.density = 1.0;
        p.delta_t = 1.0;
        assert!(run(&p, None).is_err());
    }
    #[test]
    fn previously_refused_full_history_gets_bounded_display_sampling() {
        let (_, mut p) = fea_problem::templates::templates()
            .into_iter()
            .next()
            .unwrap();
        p.material.density = 1.0;
        let mesh = fea_problem::solve::preview(&p, None).unwrap();
        assert!(mesh.n_dofs() * 241 > SAMPLE_BUDGET);
        assert!(display_stride(mesh.nodes.len(), 240).is_ok());
        let mut progress = std::cell::RefCell::new(Vec::new());
        let scene =
            run_with_progress(&p, None, &|stage| progress.borrow_mut().push(stage)).unwrap();
        assert_eq!(scene.positions.len(), mesh.nodes.len());
        assert!(scene.samples.len() <= SAMPLE_BUDGET);
        let progress = progress.get_mut();
        let steps: Vec<_> = progress
            .iter()
            .filter(|p| p.total == 240)
            .map(|p| p.completed)
            .collect();
        assert_eq!(steps, (0..=240).collect::<Vec<_>>());
        assert!(progress.last().unwrap().stage.contains("Opening"));
        for n in [1, 16_000, 40_000, 100_000, 500_000] {
            let stride = display_stride(n, 240).unwrap();
            let frames = 1 + (1..=240).filter(|i| i % stride == 0 || *i == 240).count();
            assert!(n * frames <= SAMPLE_BUDGET);
        }
        assert!(display_stride(500_001, 240).is_err());
    }
}
