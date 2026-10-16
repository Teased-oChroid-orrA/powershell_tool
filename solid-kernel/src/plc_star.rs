//! Boundary-conforming fan seed for strictly star-shaped triangulated PLCs.
//! Not constrained Delaunay, general PLC recovery or a production strategy.
use crate::{
    plc::ValidatedPlc,
    step::{Error, ErrorKind},
    V3,
};
use fea_core::meshing::tet_topology::{orientation, TetTopology};
use std::{cmp::Ordering, collections::BTreeSet};
fn fail(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
pub struct StarSeed {
    plc: ValidatedPlc,
    topology: TetTopology,
}
impl StarSeed {
    /// The caller supplies a kernel point. Every outward boundary facet must
    /// see it strictly on its interior side, certified by exact orientation.
    /// No epsilon, centroid classification, feature deletion or fallback.
    pub fn new(
        plc: ValidatedPlc,
        center: V3,
        max_cells: usize,
        mut progress: impl FnMut(usize, usize) -> bool,
    ) -> Result<Self, Error> {
        let n = plc.facets().len();
        if n > max_cells {
            return Err(fail(
                ErrorKind::Limit,
                "star seed tetrahedron budget exceeded",
            ));
        }
        if !progress(0, n) {
            return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
        }
        let mut cells = Vec::with_capacity(n);
        let center_id = plc.vertices().len();
        for (i, tri) in plc.facets().iter().enumerate() {
            if i % 256 == 0 && !progress(i, n) {
                return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
            }
            let [a, b, c] = tri.map(|id| plc.vertices()[id]);
            let side = orientation(a, b, c, center)
                .map_err(|e| fail(ErrorKind::Geometry, e.to_string()))?;
            if side != Ordering::Less {
                return Err(fail(ErrorKind::Geometry,format!("kernel point is not strictly inside boundary facet {i}; star seed unsupported")));
            }
            cells.push([center_id, tri[0], tri[1], tri[2]]);
        }
        let mut points = plc.vertices().to_vec();
        points.push(center);
        let topology = TetTopology::new(points, cells, max_cells)
            .map_err(|e| fail(ErrorKind::Topology, e.to_string()))?;
        let keys = |faces: &[[usize; 3]]| -> BTreeSet<[usize; 3]> {
            faces
                .iter()
                .map(|f| {
                    let mut f = *f;
                    f.sort_unstable();
                    f
                })
                .collect()
        };
        if keys(topology.boundary()) != keys(plc.facets()) {
            return Err(fail(
                ErrorKind::Topology,
                "star seed did not preserve every PLC facet",
            ));
        }
        if !progress(n, n) {
            return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
        }
        Ok(Self { plc, topology })
    }
    pub fn topology(&self) -> &TetTopology {
        &self.topology
    }
    pub fn plc(&self) -> &ValidatedPlc {
        &self.plc
    }
}
