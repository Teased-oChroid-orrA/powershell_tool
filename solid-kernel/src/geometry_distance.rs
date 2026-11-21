//! Shared bounded branch-and-bound for curve/surface distance objectives.
use crate::{
    geometry_bounds::Bounds3,
    native_interval::Interval,
    nurbs::fail,
    step::{Error, ErrorKind},
    V3,
};
use std::{cmp::Ordering, collections::BinaryHeap};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub distance_tolerance: f64,
    pub max_enclosures: usize,
    pub max_pending: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            distance_tolerance: 0.001,
            max_enclosures: 100_000,
            max_pending: 100_000,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub enclosures: usize,
    pub pending_intervals: usize,
    pub distance_bounds: [f64; 2],
}

pub(crate) trait Geometry<const D: usize> {
    fn domain(&self) -> [[f64; 2]; D];
    fn cells(&self, maximum: usize) -> Result<Vec<[[f64; 2]; D]>, Error>;
    fn enclose(&self, domain: [[f64; 2]; D]) -> Result<Bounds3, Error>;
    /// Diagnostic binary64 representative at the parameter. Bounds depend on
    /// `enclose`, never this point. Wrappers document whether it is an evaluated
    /// point or an enclosure midpoint; it does not prove projection uniqueness.
    fn representative(&self, parameter: [f64; D], enclosure: Bounds3) -> Result<V3, Error>;
}
pub(crate) struct Bracket<const D: usize> {
    pub bounds: [f64; 2],
    pub parameter: [f64; D],
    pub point: V3,
    pub enclosure: Bounds3,
    pub calls: usize,
}
#[derive(Clone, Copy)]
struct Node<const D: usize> {
    domain: [[f64; 2]; D],
    lower: f64,
}
impl<const D: usize> PartialEq for Node<D> {
    fn eq(&self, b: &Self) -> bool {
        self.lower == b.lower && self.domain == b.domain
    }
}
impl<const D: usize> Eq for Node<D> {}
impl<const D: usize> PartialOrd for Node<D> {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl<const D: usize> Ord for Node<D> {
    fn cmp(&self, b: &Self) -> Ordering {
        let mut order = b.lower.total_cmp(&self.lower);
        for axis in 0..D {
            for end in 0..2 {
                order = order.then_with(|| b.domain[axis][end].total_cmp(&self.domain[axis][end]));
            }
        }
        order
    }
}
pub(crate) fn distance_bounds(b: Bounds3, point: V3) -> Result<[f64; 2], Error> {
    let mut sum = Interval::point(0.);
    for i in 0..3 {
        let d = Interval::new(b.min[i], b.max[i]) - Interval::point(point[i]);
        let lo = if d.lo > 0. {
            d.lo
        } else if d.hi < 0. {
            -d.hi
        } else {
            0.
        };
        let range = Interval::new(lo, d.lo.abs().max(d.hi.abs()));
        sum = sum + range * range;
    }
    let bounds = [
        sum.lo.max(0.).sqrt().next_down().max(0.),
        sum.hi.sqrt().next_up(),
    ];
    if bounds.iter().any(|x| !x.is_finite()) {
        return Err(fail(
            ErrorKind::Geometry,
            "nonfinite geometry distance enclosure",
        ));
    }
    Ok(bounds)
}

struct Search<'a, G, const D: usize> {
    geometry: &'a G,
    query: V3,
    opt: Options,
    calls: usize,
    best: Option<(f64, [f64; D], Bounds3)>,
}
impl<G: Geometry<D>, const D: usize> Search<'_, G, D> {
    fn enclose(&mut self, domain: [[f64; 2]; D]) -> Result<Bounds3, Error> {
        if self.calls >= self.opt.max_enclosures {
            return Err(fail(
                ErrorKind::Limit,
                "geometry distance enclosure budget exceeded",
            ));
        }
        self.calls += 1;
        self.geometry.enclose(domain)
    }
    fn candidate(&mut self, t: [f64; D]) -> Result<(), Error> {
        let enclosure = self.enclose(t.map(|x| [x, x]))?;
        let upper = distance_bounds(enclosure, self.query)?[1];
        if self.best.as_ref().is_none_or(|b| upper < b.0) {
            self.best = Some((upper, t, enclosure));
        }
        Ok(())
    }
    fn node(&mut self, domain: [[f64; 2]; D]) -> Result<Node<D>, Error> {
        let bounds = self.enclose(domain)?;
        Ok(Node {
            domain,
            lower: distance_bounds(bounds, self.query)?[0],
        })
    }
    fn candidates(&mut self, domain: [[f64; 2]; D]) -> Result<(), Error> {
        for corner in 0..(1usize << D) {
            self.candidate(std::array::from_fn(|axis| {
                domain[axis][(corner >> axis) & 1]
            }))?;
        }
        self.candidate(domain.map(|d| d[0] + (d[1] - d[0]) * 0.5))
    }
}
/// The best upper distance bound found so far; `candidates` has always run on a cell before this is asked.
fn best_upper<G, const D: usize>(search: &Search<'_, G, D>) -> f64 {
    search.best.as_ref().expect("a candidate is recorded for every cell").0
}
pub(crate) fn distance<G: Geometry<D>, const D: usize>(
    geometry: &G,
    query: V3,
    opt: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Bracket<D>, Error> {
    if !opt.distance_tolerance.is_finite()
        || opt.distance_tolerance <= 0.
        || query.iter().any(|x| !x.is_finite() || x.abs() > 1e30)
    {
        return Err(fail(
            ErrorKind::Geometry,
            "invalid geometry distance query/tolerance",
        ));
    }
    let cancelled = || fail(ErrorKind::Cancelled, "geometry distance query cancelled");
    if !progress(Progress {
        enclosures: 0,
        pending_intervals: 0,
        distance_bounds: [0., f64::INFINITY],
    }) {
        return Err(cancelled());
    }
    let cells = geometry.cells(opt.max_pending)?;
    if cells.is_empty() {
        return Err(fail(
            ErrorKind::Geometry,
            "geometry has no non-degenerate parameter cell to search",
        ));
    }
    let mut search = Search {
        geometry,
        query,
        opt,
        calls: 0,
        best: None,
    };
    let mut queue = BinaryHeap::new();
    for domain in cells {
        queue.push(search.node(domain)?);
        search.candidates(domain)?;
        if !progress(Progress {
            enclosures: search.calls,
            pending_intervals: queue.len(),
            distance_bounds: [0., best_upper(&search)],
        }) {
            return Err(cancelled());
        }
    }
    let full = geometry.domain();
    loop {
        let (upper, parameter, enclosure) = search.best.expect("every searched cell records a candidate, and there is at least one");
        let lower = queue.peek().map_or(upper, |node| node.lower.min(upper));
        let state = Progress {
            enclosures: search.calls,
            pending_intervals: queue.len(),
            distance_bounds: [lower, upper],
        };
        if !progress(state) {
            return Err(cancelled());
        }
        if (Interval::point(upper) - Interval::point(lower)).hi <= opt.distance_tolerance {
            let result = Bracket {
                bounds: [lower, upper],
                parameter,
                point: geometry.representative(parameter, enclosure)?,
                enclosure,
                calls: search.calls,
            };
            if !progress(Progress {
                pending_intervals: 0,
                ..state
            }) {
                return Err(cancelled());
            }
            return Ok(result);
        }
        let node = queue.pop().expect("an empty queue has lower = upper, which ends the search above");
        if node.lower >= upper {
            continue;
        }
        // Normalize by the complete parameter domain; units/reparameterization
        // of either axis must not arbitrarily bias the subdivision direction.
        let mut split = None;
        for axis in 0..D {
            let d = node.domain[axis];
            let middle = d[0] + (d[1] - d[0]) * 0.5;
            if middle <= d[0] || middle >= d[1] {
                continue;
            }
            let width = (d[1] - d[0]) / (full[axis][1] - full[axis][0]);
            if split.is_none_or(|(_, _, best)| width > best) {
                split = Some((axis, middle, width));
            }
        }
        let (axis, middle, _) = split.ok_or_else(|| {
            fail(
                ErrorKind::Geometry,
                "geometry distance parameter precision exhausted",
            )
        })?;
        for half in 0..2 {
            let mut domain = node.domain;
            domain[axis][1 - half] = middle;
            let child = search.node(domain)?;
            if child.lower < best_upper(&search) {
                if queue.len() >= opt.max_pending {
                    return Err(fail(
                        ErrorKind::Limit,
                        "geometry distance pending-interval budget exceeded",
                    ));
                }
                queue.push(child);
            }
            search.candidates(domain)?;
        }
    }
}
pub(crate) fn push_cell<const D: usize>(
    cells: &mut Vec<[[f64; 2]; D]>,
    domain: [[f64; 2]; D],
    maximum: usize,
) -> Result<(), Error> {
    if cells.len() >= maximum {
        return Err(fail(
            ErrorKind::Limit,
            "geometry distance pending-interval budget exceeded",
        ));
    }
    cells.push(domain);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A geometry whose parameter domain yields no cell to search (a trait implementation is free to).
    struct NoCells;
    impl Geometry<1> for NoCells {
        fn domain(&self) -> [[f64; 2]; 1] {
            [[0., 1.]]
        }
        fn cells(&self, _maximum: usize) -> Result<Vec<[[f64; 2]; 1]>, Error> {
            Ok(Vec::new())
        }
        fn enclose(&self, _domain: [[f64; 2]; 1]) -> Result<Bounds3, Error> {
            unreachable!("no cell, so nothing is enclosed")
        }
        fn representative(&self, _parameter: [f64; 1], _enclosure: Bounds3) -> Result<V3, Error> {
            unreachable!("no cell, so no candidate")
        }
    }
    /// Without a cell there is no candidate and no upper bound: the search used to unwrap its missing best candidate.
    #[test]
    fn a_geometry_without_cells_is_refused_not_unwrapped() {
        let Err(error) = distance(&NoCells, [0.; 3], Options::default(), |_| true) else {
            panic!("a search without cells cannot bound a distance");
        };
        assert_eq!(error.kind, ErrorKind::Geometry);
        assert!(error.detail.contains("no non-degenerate parameter cell"), "{}", error.detail);
    }
}
