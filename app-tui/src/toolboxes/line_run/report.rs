//! The Line Run report (plain text) and the station table (CSV), in the units the window shows.
use super::model::{LineRunModel, Qty};
use pressure_vessel_solver::line_run::{Location, Surface};

/// `x = 12 in, outer surface, angle 90 deg` for a governing location.
pub fn location_text(m: &LineRunModel, loc: &Location) -> String {
    let u = m.units();
    format!(
        "x = {}, {} surface, angle {:.0}\u{b0}",
        u.text(Qty::Length, loc.x),
        match loc.surface {
            Surface::Inner => "inner",
            Surface::Outer => "outer",
        },
        loc.theta.to_degrees().rem_euclid(360.0)
    )
}

pub fn build_report_text(m: &LineRunModel) -> String {
    let (i, u) = (&m.inputs, m.units());
    let mut s = String::from("LINE RUN REPORT\n");
    s += &format!("Units: {}\n\n", u.name());
    s += &format!("Tube: OD {}, wall {}, line {} to {}\n", u.text(Qty::Length, i.od), u.text(Qty::Length, i.wall), u.text(Qty::Length, i.x_start), u.text(Qty::Length, i.x_end));
    s += &format!("Material class: {}\n", i.class.label());
    s += "Supports:\n";
    for (k, sp) in i.supports.iter().enumerate() {
        s += &format!("  {}  x {}{}{}\n", k + 1, u.text(Qty::Length, sp.x), if sp.anchor { ", axial anchor" } else { "" }, if sp.clamp { ", clamped" } else { "" });
    }
    if !i.loads.is_empty() {
        s += "Point loads:\n";
        for (k, l) in i.loads.iter().enumerate() {
            s += &format!(
                "  {}  x {}, weight {}, force ({}, {}, {}), offset ({}, {}, {}){}; {}\n",
                k + 1,
                u.text(Qty::Length, l.x),
                u.text(Qty::Force, l.weight),
                u.text(Qty::Force, l.force[0]),
                u.text(Qty::Force, l.force[1]),
                u.text(Qty::Force, l.force[2]),
                u.text(Qty::Length, l.offset[0]),
                u.text(Qty::Length, l.offset[1]),
                u.text(Qty::Length, l.offset[2]),
                if l.thrust { ", includes pressure thrust" } else { "" },
                m.scope_label(l)
            );
        }
    }
    s.push('\n');
    match &m.outcome {
        Err(why) => s += &format!("No result: {why}\n"),
        Ok(result) => {
            let g = result.governing();
            s += &format!("Governing: {} in case \"{}\", MS {:+.3} ({})\n\n", g.criterion.label(), g.location.case, g.margin, location_text(m, &g.location));
            for c in &result.cases {
                s += &format!("Case \"{}\" (equilibrium residual {:.1e})\n", c.name, c.residual);
                for mg in &c.margins {
                    s += &format!(
                        "  {:<32} MS {:+.3}  applied {} of {}, FS {}, {}\n",
                        mg.criterion.label(),
                        mg.margin,
                        u.text(Qty::Stress, mg.applied),
                        u.text(Qty::Stress, mg.allowable),
                        mg.factor_of_safety,
                        location_text(m, &mg.location)
                    );
                }
                s += "  Reactions:\n";
                for r in &c.reactions {
                    s += &format!(
                        "    x {}  F ({}, {}, {})  M ({}, {}, {})\n",
                        u.text(Qty::Length, r.x),
                        u.text(Qty::Force, r.force[0]),
                        u.text(Qty::Force, r.force[1]),
                        u.text(Qty::Force, r.force[2]),
                        u.text(Qty::Moment, r.moment[0]),
                        u.text(Qty::Moment, r.moment[1]),
                        u.text(Qty::Moment, r.moment[2])
                    );
                }
                s.push('\n');
            }
        }
    }
    s += "Allowables and factors of safety are the entered values; none is stored or assumed. See docs/pressure-vessel-line-run.md for the method and its limits.\n";
    s
}

/// One row per station of every case: `case,x,bend_y,bend_z,resultant,shear_y,shear_z,axial` in the shown units.
pub fn stations_csv(m: &LineRunModel) -> Result<String, String> {
    let result = m.outcome.as_ref().map_err(|e| e.clone())?;
    let u = m.units();
    let (l, mo, f) = (u.factor(Qty::Length), u.factor(Qty::Moment), u.factor(Qty::Force));
    let mut s = format!("case,x ({}),bend_y ({}),bend_z ({}),moment ({}),shear_y ({}),shear_z ({}),axial ({})\n", u.label(Qty::Length), u.label(Qty::Moment), u.label(Qty::Moment), u.label(Qty::Moment), u.label(Qty::Force), u.label(Qty::Force), u.label(Qty::Force));
    for c in &result.cases {
        let name = c.name.replace('"', "\"\"");
        for st in &c.stations {
            s += &format!("\"{name}\",{},{},{},{},{},{},{}\n", st.x * l, st.bend_y * mo, st.bend_z * mo, st.resultant_moment() * mo, st.shear_y * f, st.shear_z * f, st.axial_force * f);
        }
    }
    Ok(s)
}
