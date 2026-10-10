//! Bounded, verified free vibration after instantaneous removal of a static mechanical load.
use super::Scene;
use fea_core::report::{AcceptanceReport, TransientEnergy};
use fea_core::{Loads, ModalOptions, Model, TransientOptions};
use fea_problem::Problem;
pub fn run(p: &Problem, import: Option<&str>) -> Result<Scene, String> {
    p.validate()?;
    if !p.bushings.is_empty()
        || p.delta_t != 0.0
        || !p.material.density.is_finite()
        || p.material.density <= 0.0
    {
        return Err("free vibration needs positive mass density, mechanical loads and no contact bushings or thermal load".into());
    }
    let mut mesh = fea_problem::solve::preview(p, import)?;
    if mesh.n_dofs().checked_mul(241).is_none_or(|n| n > 4_000_000) {
        return Err("full displacement history exceeds the animation memory budget".into());
    }
    let bc = fea_problem::solve::apply_supports(&mesh, &p.supports)?;
    if bc.fixed.iter().zip(&bc.value).any(|(f, v)| *f && *v != 0.0) {
        return Err("free vibration requires homogeneous supports".into());
    }
    let loads = fea_problem::solve::build_loads(&mut mesh, p)?;
    mesh.set_density_all(p.material.density)?;
    let model = Model::new(mesh)?;
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
    let initial =
        model.solve_adaptive(&loads, &bc, &fea_core::strategy::Requirements::default())?;
    let zero = vec![0.0; initial.solution.u.len()];
    let result = model.transient(
        &Loads::default(),
        &|_| 0.0,
        &bc,
        Some((&initial.solution.u, &zero)),
        &TransientOptions {
            dt: 2.0 * std::f64::consts::TAU / omega / 240.0,
            steps: 240,
            record: (0..zero.len()).collect(),
            ..Default::default()
        },
    )?;
    AcceptanceReport::transient(
        &result,
        1e-8,
        Some(TransientEnergy::Conserved { tol: 1e-6 }),
    )
    .require()?;
    let frames: Vec<Vec<f64>> = (0..result.times.len())
        .map(|s| result.history.iter().map(|h| h[s]).collect())
        .collect();
    let d = model.mesh.dim();
    let values: Vec<f64> = (0..model.mesh.nodes.len())
        .map(|i| {
            initial.solution.u[i * d..(i + 1) * d]
                .iter()
                .fold(0.0_f64, |a, v| a.hypot(*v))
        })
        .collect();
    Scene::from_frames(
        &model.mesh,
        &frames,
        result.times.iter().map(|t| *t as f32).collect(),
        &values,
        format!(
            "{} | verified unloaded free vibration | seconds | initial displacement magnitude",
            p.name
        ),
    )
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
    fn oversized_full_history_is_refused_before_solving() {
        let (_, mut p) = fea_problem::templates::templates()
            .into_iter()
            .next()
            .unwrap();
        p.material.density = 1.0;
        assert!(run(&p, None).unwrap_err().contains("memory budget"));
    }
}
