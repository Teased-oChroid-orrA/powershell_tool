//! Physics-aware region selection: which kind of region an element sits in, which estimators judge it, and how strongly its
//! physics error counts in the refinement ranking.
//!
//! Zones come from the geometry (2D: re-entrant corners are singular, sharp convex corners and the boundaries of holes are
//! geometric discontinuities) and from what the caller knows and the mesh cannot tell (contact patches, fastener bearing
//! surfaces, buckling-sensitive members, given as node or element lists). The default assignment follows the epic:
//!
//! | zone | estimators |
//! |---|---|
//! | smooth continuum | DWR + SPR+ |
//! | contact | DWR + equilibrated stress |
//! | contact perimeter | DWR + contact residual checks |
//! | fastener bearing | DWR + localised physics estimate |
//! | geometric discontinuity | DWR + equilibrated stress |
//! | buckling sensitive | eigenvalue estimator |
//! | singular | equilibrated stress, refinement capped |
//!
//! Only the estimators that exist in this crate run; an assignment to one that does not (contact residuals, eigenvalue
//! estimators) is reported as unmet by [`ZoneMap::unmet`], never silently replaced.

use crate::meshing::adjacency::Adjacency;
use crate::meshing::MeshError;
use crate::mesh::Mesh;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Zone {
    Smooth,
    Contact,
    ContactPerimeter,
    FastenerBearing,
    GeometricDiscontinuity,
    BucklingSensitive,
    Singular,
}

/// Physics-side estimator a zone calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicsEstimator {
    SprPlus,
    Equilibrated,
    ContactResidual,
    LocalisedPhysics,
    Eigenvalue,
}

impl Zone {
    /// Whether the goal-oriented (adjoint) estimate applies in the zone.
    pub fn uses_adjoint(self) -> bool {
        !matches!(self, Zone::BucklingSensitive | Zone::Singular)
    }

    pub fn physics(self) -> PhysicsEstimator {
        match self {
            Zone::Smooth => PhysicsEstimator::SprPlus,
            Zone::Contact | Zone::GeometricDiscontinuity | Zone::Singular => PhysicsEstimator::Equilibrated,
            Zone::ContactPerimeter => PhysicsEstimator::ContactResidual,
            Zone::FastenerBearing => PhysicsEstimator::LocalisedPhysics,
            Zone::BucklingSensitive => PhysicsEstimator::Eigenvalue,
        }
    }
}

/// What the caller knows that the mesh does not say, and the detection thresholds.
#[derive(Debug, Clone)]
pub struct ZoneSpec {
    /// Nodes on contact surfaces: elements touching them are `Contact`; elements with some but not all of their boundary
    /// nodes in the set are `ContactPerimeter`.
    pub contact_nodes: Vec<usize>,
    pub bearing_nodes: Vec<usize>,
    pub buckling_elements: Vec<usize>,
    /// Interior angle (radians) beyond which a boundary corner is re-entrant (singular).
    pub reentrant_angle: f64,
    /// Interior angle below which a convex boundary corner is a geometric discontinuity.
    pub sharp_angle: f64,
    /// Element rings around a detected feature that belong to its zone.
    pub rings: usize,
    /// Weight `beta_e` of the physics indicator per zone (the spatial physics weighting of the fusion).
    pub beta: HashMap<Zone, f64>,
    /// Detect geometry features (2D only).
    pub detect_geometry: bool,
}

impl Default for ZoneSpec {
    fn default() -> Self {
        let beta = [(Zone::Smooth, 1.0), (Zone::Contact, 2.0), (Zone::ContactPerimeter, 3.0), (Zone::FastenerBearing, 2.0), (Zone::GeometricDiscontinuity, 1.5), (Zone::BucklingSensitive, 1.0), (Zone::Singular, 0.5)].into_iter().collect();
        Self { contact_nodes: Vec::new(), bearing_nodes: Vec::new(), buckling_elements: Vec::new(), reentrant_angle: 200f64.to_radians(), sharp_angle: 160f64.to_radians(), rings: 1, beta, detect_geometry: true }
    }
}

/// Zone and physics weight of every element (global element order).
#[derive(Debug, Clone)]
pub struct ZoneMap {
    pub zones: Vec<Zone>,
    pub beta: Vec<f64>,
    /// Corner nodes that triggered a singular / discontinuity zone (for diagnostics and the singular-region policy).
    pub singular_nodes: Vec<usize>,
}

impl ZoneMap {
    pub fn count(&self, z: Zone) -> usize {
        self.zones.iter().filter(|&&q| q == z).count()
    }

    /// Estimators the zones call for that this crate cannot run, with the number of elements waiting for them.
    pub fn unmet(&self) -> Vec<(PhysicsEstimator, usize)> {
        let mut m: HashMap<PhysicsEstimator, usize> = HashMap::new();
        for z in &self.zones {
            let p = z.physics();
            if matches!(p, PhysicsEstimator::ContactResidual | PhysicsEstimator::LocalisedPhysics | PhysicsEstimator::Eigenvalue) {
                *m.entry(p).or_default() += 1;
            }
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by_key(|x| x.0 as u8);
        v
    }
}

/// Resolve the zones of a mesh.
pub trait PhysicsZoneResolver {
    fn resolve(&self, mesh: &Mesh, adj: &Adjacency) -> Result<ZoneMap, MeshError>;
}

/// Geometry- and list-based resolver (see the module docs).
pub struct DefaultZoneResolver {
    pub spec: ZoneSpec,
}

fn corner_angle(mesh: &Mesh, elem_nodes: &[usize], node: usize) -> f64 {
    let c = elem_nodes.iter().position(|&n| n == node).expect("the element contains the node");
    let nc = elem_nodes.len();
    let (p, a, b) = (mesh.nodes[node], mesh.nodes[elem_nodes[(c + 1) % nc]], mesh.nodes[elem_nodes[(c + nc - 1) % nc]]);
    let (u, v) = ([a[0] - p[0], a[1] - p[1]], [b[0] - p[0], b[1] - p[1]]);
    let cs = (u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1]));
    cs.clamp(-1.0, 1.0).acos()
}

impl PhysicsZoneResolver for DefaultZoneResolver {
    fn resolve(&self, mesh: &Mesh, adj: &Adjacency) -> Result<ZoneMap, MeshError> {
        let spec = &self.spec;
        let n_el = adj.n_elems();
        let mut zones = vec![Zone::Smooth; n_el];
        let mut singular_nodes = Vec::new();
        let n_nodes = mesh.nodes.len();
        let set = |zones: &mut Vec<Zone>, elems: &[usize], z: Zone| {
            for &e in elems {
                if z > zones[e] {
                    zones[e] = z; // later enum variants win: Singular over discontinuity over smooth
                }
            }
        };
        // elements within `rings` rings of a node set
        let ring_elems = |nodes: &[usize]| -> Vec<usize> {
            let mut mark = vec![false; n_el];
            let mut frontier: Vec<usize> = Vec::new();
            for &n in nodes {
                for &e in adj.node_elements(n) {
                    if !mark[e as usize] {
                        mark[e as usize] = true;
                        frontier.push(e as usize);
                    }
                }
            }
            for _ in 0..spec.rings {
                let mut next = Vec::new();
                for &e in &frontier {
                    let (b, le) = adj.local(e);
                    let blk = &mesh.blocks[b];
                    for &n in &blk.elem(le)[..blk.kind.n_corners()] {
                        for &m in adj.node_elements(n) {
                            if !mark[m as usize] {
                                mark[m as usize] = true;
                                next.push(m as usize);
                            }
                        }
                    }
                }
                frontier = next;
            }
            (0..n_el).filter(|&e| mark[e]).collect()
        };

        if spec.detect_geometry && mesh.dim() == 2 {
            // boundary nodes and their boundary-edge neighbours
            let mut bnodes: HashMap<usize, Vec<usize>> = HashMap::new();
            let mut bedges: Vec<(usize, usize)> = Vec::new();
            for f in adj.boundary_faces() {
                if f.key[1] != usize::MAX {
                    bnodes.entry(f.key[0]).or_default().push(f.key[1]);
                    bnodes.entry(f.key[1]).or_default().push(f.key[0]);
                    bedges.push((f.key[0], f.key[1]));
                }
            }
            // interior angle at every boundary corner node
            let mut reentrant = Vec::new();
            let mut sharp = Vec::new();
            for &n in bnodes.keys() {
                let mut ang = 0.0;
                for &g in adj.node_elements(n) {
                    let (b, le) = adj.local(g as usize);
                    let blk = &mesh.blocks[b];
                    let nc = blk.kind.n_corners();
                    let conn = &blk.elem(le)[..nc];
                    if conn.contains(&n) {
                        ang += corner_angle(mesh, conn, n);
                    }
                }
                if ang > spec.reentrant_angle {
                    reentrant.push(n);
                } else if ang < spec.sharp_angle && bnodes[&n].len() == 2 {
                    sharp.push(n);
                }
            }
            reentrant.sort_unstable();
            sharp.sort_unstable();
            // loops of the boundary graph: the one with the largest bounding box is the outer boundary, the others are holes
            let mut comp = vec![usize::MAX; n_nodes];
            let mut n_comp = 0;
            let mut starts: Vec<usize> = bnodes.keys().copied().collect();
            starts.sort_unstable();
            for &s in &starts {
                if comp[s] != usize::MAX {
                    continue;
                }
                let mut stack = vec![s];
                comp[s] = n_comp;
                while let Some(v) = stack.pop() {
                    for &w in &bnodes[&v] {
                        if comp[w] == usize::MAX {
                            comp[w] = n_comp;
                            stack.push(w);
                        }
                    }
                }
                n_comp += 1;
            }
            let mut extent = vec![0.0f64; n_comp];
            let (mut lo, mut hi) = (vec![[f64::INFINITY; 2]; n_comp], vec![[f64::NEG_INFINITY; 2]; n_comp]);
            for &n in &starts {
                for k in 0..2 {
                    lo[comp[n]][k] = lo[comp[n]][k].min(mesh.nodes[n][k]);
                    hi[comp[n]][k] = hi[comp[n]][k].max(mesh.nodes[n][k]);
                }
            }
            for c in 0..n_comp {
                extent[c] = (hi[c][0] - lo[c][0]) * (hi[c][1] - lo[c][1]);
            }
            let outer = (0..n_comp).max_by(|&a, &b| extent[a].total_cmp(&extent[b]));
            let hole_nodes: Vec<usize> = starts.iter().copied().filter(|&n| Some(comp[n]) != outer).collect();
            set(&mut zones, &ring_elems(&hole_nodes), Zone::GeometricDiscontinuity);
            set(&mut zones, &ring_elems(&sharp), Zone::GeometricDiscontinuity);
            set(&mut zones, &ring_elems(&reentrant), Zone::Singular);
            singular_nodes = reentrant;
            let _ = bedges;
        }
        // caller knowledge
        if !spec.contact_nodes.is_empty() {
            let mut in_set = vec![false; n_nodes];
            for &n in &spec.contact_nodes {
                in_set[n] = true;
            }
            let (mut full, mut part) = (Vec::new(), Vec::new());
            for e in 0..n_el {
                let (b, le) = adj.local(e);
                let blk = &mesh.blocks[b];
                let nodes = &blk.elem(le)[..blk.kind.n_corners()];
                let hits = nodes.iter().filter(|&&n| in_set[n]).count();
                if hits == nodes.len() || (hits >= 2 && hits * 2 >= nodes.len()) {
                    full.push(e);
                } else if hits > 0 {
                    part.push(e);
                }
            }
            set(&mut zones, &part, Zone::ContactPerimeter);
            set(&mut zones, &full, Zone::Contact);
            // the perimeter is the ring of elements around the contact elements that is not itself in contact
            let around = ring_elems(&spec.contact_nodes);
            let perimeter: Vec<usize> = around.into_iter().filter(|&e| zones[e] == Zone::Smooth).collect();
            set(&mut zones, &perimeter, Zone::ContactPerimeter);
        }
        if !spec.bearing_nodes.is_empty() {
            set(&mut zones, &ring_elems(&spec.bearing_nodes), Zone::FastenerBearing);
        }
        set(&mut zones, &spec.buckling_elements, Zone::BucklingSensitive);
        let beta = zones.iter().map(|z| spec.beta.get(z).copied().unwrap_or(1.0)).collect();
        Ok(ZoneMap { zones, beta, singular_nodes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn square(n: usize) -> Mesh {
        grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [n, n, 1], &|p| p).unwrap()
    }

    #[test]
    fn a_plain_square_is_smooth_except_at_its_sharp_corners() {
        let m = square(8);
        let adj = m.adjacency();
        let z = DefaultZoneResolver { spec: ZoneSpec { rings: 0, ..Default::default() } }.resolve(&m, &adj).unwrap();
        // the four 90 degree corners are sharp convex corners: one element each
        assert_eq!(z.count(Zone::GeometricDiscontinuity), 4);
        assert_eq!(z.count(Zone::Singular), 0);
        assert!(z.unmet().is_empty());
        assert_eq!(z.beta.len(), 64);
    }

    #[test]
    fn a_reentrant_corner_is_singular_and_a_hole_boundary_is_a_discontinuity() {
        // L-shape from a 6 x 6 grid with the upper-right quadrant removed
        let m = square(6);
        let mut l = Mesh::new(m.physics);
        l.nodes = m.nodes.clone();
        let blk = &m.blocks[0];
        let mut conn = Vec::new();
        for e in 0..blk.n_elems() {
            let c = blk.elem(e);
            let ctr: f64 = c.iter().map(|&n| m.nodes[n][0]).sum::<f64>() / 4.0;
            let cty: f64 = c.iter().map(|&n| m.nodes[n][1]).sum::<f64>() / 4.0;
            if !(ctr > 0.5 && cty > 0.5) {
                conn.extend_from_slice(c);
            }
        }
        l.add_block(ElementKind::Quad4, conn, Elastic::new(1.0, 0.3), "l").unwrap();
        // drop unused nodes so the boundary graph is clean
        let adj = l.adjacency();
        let z = DefaultZoneResolver { spec: ZoneSpec { rings: 0, ..Default::default() } }.resolve(&l, &adj).unwrap();
        assert!(z.count(Zone::Singular) >= 1, "the re-entrant corner must be singular");
        assert_eq!(z.singular_nodes.len(), 1);
        let c = z.singular_nodes[0];
        assert!((l.nodes[c][0] - 0.5).abs() < 1e-9 && (l.nodes[c][1] - 0.5).abs() < 1e-9);
        // a square with a rectangular hole: the ring of elements around the hole is a discontinuity zone
        let h = square(8);
        let mut holed = Mesh::new(h.physics);
        holed.nodes = h.nodes.clone();
        let mut conn = Vec::new();
        for e in 0..h.blocks[0].n_elems() {
            let c = h.blocks[0].elem(e);
            let (x, y) = (c.iter().map(|&n| h.nodes[n][0]).sum::<f64>() / 4.0, c.iter().map(|&n| h.nodes[n][1]).sum::<f64>() / 4.0);
            if !((x - 0.5).abs() < 0.13 && (y - 0.5).abs() < 0.13) {
                conn.extend_from_slice(c);
            }
        }
        holed.add_block(ElementKind::Quad4, conn, Elastic::new(1.0, 0.3), "h").unwrap();
        let adj = holed.adjacency();
        let z = DefaultZoneResolver { spec: ZoneSpec { rings: 0, ..Default::default() } }.resolve(&holed, &adj).unwrap();
        // 2 x 2 hole: 12 elements touch its boundary nodes; the 90 degree concave hole corners are re-entrant (270 degrees)
        assert!(z.count(Zone::GeometricDiscontinuity) + z.count(Zone::Singular) >= 12, "{} {}", z.count(Zone::GeometricDiscontinuity), z.count(Zone::Singular));
    }

    #[test]
    fn contact_bearing_and_buckling_lists_set_their_zones_and_unmet_estimators_are_reported() {
        let m = square(6);
        let adj = m.adjacency();
        let contact: Vec<usize> = (0..m.nodes.len()).filter(|&n| (m.nodes[n][1] - 0.5).abs() < 1e-9 && m.nodes[n][0] <= 0.5 + 1e-9).collect();
        let spec = ZoneSpec { contact_nodes: contact, bearing_nodes: vec![0], buckling_elements: vec![35], detect_geometry: false, rings: 1, ..Default::default() };
        let z = DefaultZoneResolver { spec }.resolve(&m, &adj).unwrap();
        assert!(z.count(Zone::ContactPerimeter) > 0);
        assert_eq!(z.zones[35], Zone::BucklingSensitive);
        assert!(z.count(Zone::FastenerBearing) > 0);
        assert!(z.beta.iter().all(|&b| b > 0.0));
        let unmet = z.unmet();
        assert!(unmet.iter().any(|u| u.0 == PhysicsEstimator::Eigenvalue && u.1 == 1));
        assert!(unmet.iter().any(|u| u.0 == PhysicsEstimator::ContactResidual));
        assert!(!Zone::Singular.uses_adjoint() && Zone::Smooth.uses_adjoint());
    }
}
