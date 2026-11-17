//! Segment mode: check only the stretch of a line between two supports instead of the whole run.
//!
//! A segment cut from a continuous run is not truly clamped or pinned at its ends: the neighbouring
//! spans give it a finite rotational restraint. Two ways to handle that:
//! - model the end honestly with a rotational spring ([`EndFixity::Spring`]) when the stiffness is known;
//! - bracket it ([`analyze_segment_bounds`], two held ends): pinned-pinned (no end restraint, largest midspan moment)
//!   and clamped-clamped (full restraint, largest end moment) bound the rotational restraint a real
//!   end can supply, so the lower margin per criterion is the conservative result. This is an
//!   engineering bound on the restraint, not a theorem about every stress component: see
//!   `docs/pressure-vessel-line-run.md`.
//!
//! End actions cut from a full-run solve (moment, shear, axial force at the cut) are applied as a
//! [`PointLoad`] at the end station (`moment` is the bending-moment vector, `force` the shear and
//! axial force); on an end that carries a support the transverse force and (clamped) moment go
//! straight into that support's reaction, so such loads matter mostly at a Free end.

use super::*;

/// Slope condition (and transverse hold) at a segment end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndFixity {
    /// No support: the end is free (valve at the tip as a point load, or an overhang).
    Free,
    /// Transversely pinned, slope free (simple support, pin or clip with no moment capacity).
    Pinned,
    /// Pinned with a rotational spring (moment per radian, both planes): a clip between pinned and clamped.
    Spring(f64),
    /// Transversely pinned and slope held (rigid fixture; the infinite-stiffness case).
    Clamped,
}

/// One end of a segment: its fixity and whether it also holds the line axially.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentEnd {
    pub fixity: EndFixity,
    pub axial_anchor: bool,
}

/// Which end-restraint bound a result belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// Every held end pinned.
    Pinned,
    /// Every held end clamped.
    Clamped,
}

/// The lowest margin of one criterion over all cases and both bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct WorstMargin {
    pub bound: Bound,
    pub margin: LineMargin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentBounds {
    pub pinned: LineResult,
    pub clamped: LineResult,
    /// One entry per criterion of the material class, in the class's criterion order.
    pub worst: Vec<WorstMargin>,
}

fn end_support(x: f64, end: &SegmentEnd) -> Result<Option<Support>, LineRunError> {
    let (held, fixed, k) = match end.fixity {
        EndFixity::Free => (false, false, None),
        EndFixity::Pinned => (true, false, None),
        EndFixity::Spring(k) => (true, false, Some(k)),
        EndFixity::Clamped => (true, true, None),
    };
    if !held && end.axial_anchor {
        return Err(LineRunError::InvalidInput(format!("segment end at x = {x}: a free end cannot be an axial anchor (an anchor is a support)")));
    }
    Ok(held.then_some(Support { x, axial_anchor: end.axial_anchor, rotation_fixed: fixed, rotation_stiffness: k }))
}

/// `run` (its `x_start` / `x_end` are the segment ends; `supports` may hold interior supports only) with the given end
/// conditions added as supports: the run `analyze_segment` solves, for callers that solve it themselves (a wall search).
pub fn segment_run(run: &LineRun, ends: [SegmentEnd; 2]) -> Result<LineRun, LineRunError> {
    let tol = 1e-9 * (run.x_end - run.x_start).abs();
    for s in &run.supports {
        if (s.x - run.x_start).abs() <= tol || (s.x - run.x_end).abs() <= tol {
            return Err(LineRunError::InvalidInput(format!("support at x = {} lies on a segment end: give the end condition through `ends`", s.x)));
        }
    }
    let mut seg = run.clone();
    seg.supports.extend(end_support(run.x_start, &ends[0])?);
    seg.supports.extend(end_support(run.x_end, &ends[1])?);
    Ok(seg)
}

/// Runs [`segment_run`].
pub fn analyze_segment(run: &LineRun, ends: [SegmentEnd; 2], class: &MaterialClass, cases: &[LoadCase]) -> Result<LineResult, LineRunError> {
    analyze(&segment_run(run, ends)?, class, cases)
}

/// Brackets the restraint a segment end cut from a continuous run can supply: both ends are run once
/// pinned and once clamped (their stated `fixity` is replaced by the two bounds; a Free end is
/// refused), and the lowest margin per criterion over cases and bounds is reported with the
/// governing bound.
pub fn analyze_segment_bounds(run: &LineRun, ends: [SegmentEnd; 2], class: &MaterialClass, cases: &[LoadCase]) -> Result<SegmentBounds, LineRunError> {
    if ends.iter().any(|e| e.fixity == EndFixity::Free) {
        return Err(LineRunError::InvalidInput("bounds need both ends held: a free end has no restraint to bracket (pinned-free is a mechanism); use analyze_segment, with a Spring end if the clip stiffness is known".into()));
    }
    let with = |f: EndFixity| ends.map(|e| SegmentEnd { fixity: f, ..e });
    let pinned = analyze_segment(run, with(EndFixity::Pinned), class, cases)?;
    let clamped = analyze_segment(run, with(EndFixity::Clamped), class, cases)?;
    let n = pinned.cases[0].margins.len();
    let worst = (0..n)
        .map(|i| {
            let pick = |r: &LineResult| r.cases.iter().map(|c| &c.margins[i]).min_by(|a, b| a.margin.partial_cmp(&b.margin).expect("margins are not NaN")).expect("cases").clone();
            let (p, c) = (pick(&pinned), pick(&clamped));
            if c.margin < p.margin {
                WorstMargin { bound: Bound::Clamped, margin: c }
            } else {
                WorstMargin { bound: Bound::Pinned, margin: p }
            }
        })
        .collect();
    Ok(SegmentBounds { pinned, clamped, worst })
}
