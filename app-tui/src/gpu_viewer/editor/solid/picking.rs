//! Screen-space boundary-face BVH. Rebuilt only when geometry or the camera changes.
use super::*;

struct Node {
    bounds: [f32; 4],
    children: Option<(Box<Node>, Box<Node>)>,
    faces: Vec<usize>,
}
impl Node {
    fn build(mut ids: Vec<usize>, bounds: &[[f32; 4]]) -> Self {
        let mut b = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for &i in &ids {
            b[0] = b[0].min(bounds[i][0]);
            b[1] = b[1].min(bounds[i][1]);
            b[2] = b[2].max(bounds[i][2]);
            b[3] = b[3].max(bounds[i][3]);
        }
        if ids.len() <= 8 {
            return Self {
                bounds: b,
                children: None,
                faces: ids,
            };
        }
        let axis = usize::from(b[3] - b[1] > b[2] - b[0]);
        ids.sort_unstable_by(|&i, &j| {
            (bounds[i][axis] + bounds[i][axis + 2])
                .total_cmp(&(bounds[j][axis] + bounds[j][axis + 2]))
        });
        let right = ids.split_off(ids.len() / 2);
        Self {
            bounds: b,
            children: Some((
                Box::new(Self::build(ids, bounds)),
                Box::new(Self::build(right, bounds)),
            )),
            faces: Vec::new(),
        }
    }
    fn query(&self, p: [f32; 2], radius: f32, out: &mut Vec<usize>) {
        if p[0] + radius < self.bounds[0]
            || p[1] + radius < self.bounds[1]
            || p[0] - radius > self.bounds[2]
            || p[1] - radius > self.bounds[3]
        {
            return;
        }
        if let Some((a, b)) = &self.children {
            a.query(p, radius, out);
            b.query(p, radius, out);
        } else {
            out.extend(&self.faces);
        }
    }
}

pub(super) struct Index {
    pub camera: Camera,
    pub size: [f32; 2],
    pub mesh: std::sync::Weak<Mesh>,
    faces: Vec<(String, Vec<usize>)>,
    tree: Node,
}
impl Index {
    pub fn new(
        mesh: &Arc<Mesh>,
        positions: &[[f32; 3]],
        camera: Camera,
        size: [f32; 2],
    ) -> Self {
        let faces: Vec<_> = mesh
            .surfaces
            .iter()
            .flat_map(|(name, faces)| faces.iter().map(|f| (name.clone(), f.clone())))
            .collect();
        let bounds: Vec<_> = faces
            .iter()
            .map(|(_, f)| {
                let mut b = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                for &n in corners(f) {
                    if let Some(&v) = positions.get(n) {
                        let p = Camera::pixel(camera.project(v).0, size[0], size[1]);
                        b[0] = b[0].min(p[0]);
                        b[1] = b[1].min(p[1]);
                        b[2] = b[2].max(p[0]);
                        b[3] = b[3].max(p[1]);
                    }
                }
                b
            })
            .collect();
        let tree = Node::build((0..faces.len()).collect(), &bounds);
        Self {
            camera,
            size,
            mesh: Arc::downgrade(mesh),
            faces,
            tree,
        }
    }
    #[cfg(test)]
    pub fn allocation_id(&self) -> usize { self.faces.as_ptr() as usize }
    pub fn pick(
        &self,
        solid: bool,
        positions: &[[f32; 3]],
        px: [f32; 2],
        radius: f32,
    ) -> Option<String> {
        let mut ids = Vec::new();
        self.tree.query(px, radius, &mut ids);
        ids.sort_unstable();
        pick_faces(
            solid,
            positions,
            &self.camera,
            px,
            self.size,
            radius,
            ids.into_iter()
                .map(|i| (&self.faces[i].0, &self.faces[i].1)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_matches_brute_force_for_depth_edges_and_misses() {
        let p = fea_problem::templates::templates()
            .into_iter()
            .map(|(_, p)| p)
            .find(|p| p.analysis == fea_problem::Analysis::Solid)
            .unwrap();
        let mesh = Arc::new(fea_problem::solve::body_preview(&p).unwrap());
        let scene = Scene::from_frames(
            &mesh,
            &[vec![0.0; mesh.n_dofs()]],
            vec![0.0],
            &vec![0.0; mesh.nodes.len()],
            "test".into(),
        )
        .unwrap();
        let cam = Camera {
            yaw: 0.6,
            pitch: 0.3,
            zoom: 1.0,
            aspect: 1.5,
            pan: [0.0; 2],
        };
        let size = [600.0, 400.0];
        let index = Index::new(&mesh, &scene.positions, cam, size);
        for x in (0..600).step_by(23) {
            for y in (0..400).step_by(19) {
                let px = [x as f32, y as f32];
                assert_eq!(
                    index.pick(true, &scene.positions, px, 8.0),
                    pick_surface(&mesh, &scene.positions, &cam, px, size, 8.0)
                );
            }
        }
    }
}

#[cfg(test)]
mod measurements {
    use super::*;
    #[test]
    #[ignore = "explicit selection benchmark; no hardware timing assertion"]
    fn cached_picking_measurement() {
        let mut mesh = Mesh::new(fea_core::Physics::Solid);
        let n=80usize;
        let mut positions=Vec::new();
        for y in 0..=n { for x in 0..=n { positions.push([x as f32/n as f32-0.5,y as f32/n as f32-0.5,0.0]); } }
        mesh.nodes=positions.iter().map(|p|p.map(f64::from)).collect();
        for y in 0..n { for x in 0..n { let a=y*(n+1)+x; mesh.surfaces.entry("front".into()).or_default().push(vec![a,a+1,a+n+2,a+n+1]); } }
        let cam=Camera{yaw:0.0,pitch:0.0,zoom:1.0,aspect:1.0,pan:[0.0;2]}; let size=[800.0;2];
        let start=std::time::Instant::now(); let mesh=Arc::new(mesh); let index=Index::new(&mesh,&positions,cam,size); let build=start.elapsed();
        let queries: Vec<_>=(0..100).map(|i|[200.0+(i*37%400) as f32,200.0+(i*71%400) as f32]).collect();
        let start=std::time::Instant::now(); let slow:Vec<_>=queries.iter().map(|&p|pick_surface(&mesh,&positions,&cam,p,size,8.0)).collect();let brute=start.elapsed();
        let start=std::time::Instant::now();let fast:Vec<_>=queries.iter().map(|&p|index.pick(true,&positions,p,8.0)).collect();let cached=start.elapsed();
        assert_eq!(slow,fast);
        eprintln!("6400 faces, 100 picks: build {build:?}, brute {brute:?}, cached {cached:?}");
    }
}
