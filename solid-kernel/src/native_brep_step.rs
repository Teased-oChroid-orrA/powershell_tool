//! STEP (AP214, `ADVANCED_BREP_SHAPE_REPRESENTATION`) text of a [`NativeBrep`], millimetres, no OpenCASCADE.
//!
//! The file has the entity skeleton OpenCASCADE writes (product definition, one `MANIFOLD_SOLID_BREP`, length / angle / solid-angle
//! units, a distance uncertainty) with analytic surfaces and bare `LINE` / `CIRCLE` edge curves (no p-curves: the importer in
//! `analytic_step` does not need them). Round trip: `analytic_step::AnalyticModel::read` reads it back.
use crate::native_brep::{Curve, NativeBrep, Surface, Use};
use crate::V3;

struct Writer {
    lines: Vec<String>,
}

/// A STEP real: shortest round-trip decimal, always with a decimal point.
fn real(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.")
    }
}
fn triple(v: V3) -> String {
    format!("({},{},{})", real(v[0]), real(v[1]), real(v[2]))
}

impl Writer {
    fn add(&mut self, body: String) -> usize {
        let id = self.lines.len() + 1;
        self.lines.push(format!("#{id} = {body};"));
        id
    }
    fn point(&mut self, p: V3) -> usize {
        self.add(format!("CARTESIAN_POINT('',{})", triple(p)))
    }
    fn direction(&mut self, d: V3) -> usize {
        self.add(format!("DIRECTION('',{})", triple(d)))
    }
    fn placement(&mut self, origin: V3, axis: V3, xdir: V3) -> usize {
        let (p, a, x) = (self.point(origin), self.direction(axis), self.direction(xdir));
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{a},#{x})"))
    }
    fn surface(&mut self, s: &Surface) -> usize {
        match s {
            Surface::Plane { origin, normal, xdir } => {
                let p = self.placement(*origin, *normal, *xdir);
                self.add(format!("PLANE('',#{p})"))
            }
            Surface::Cylinder { origin, axis, xdir, radius } => {
                let p = self.placement(*origin, *axis, *xdir);
                self.add(format!("CYLINDRICAL_SURFACE('',#{p},{})", real(*radius)))
            }
            Surface::Cone { origin, axis, xdir, radius, semi_angle } => {
                let p = self.placement(*origin, *axis, *xdir);
                self.add(format!("CONICAL_SURFACE('',#{p},{},{})", real(*radius), real(*semi_angle)))
            }
            Surface::Sphere { centre, axis, xdir, radius } => {
                let p = self.placement(*centre, *axis, *xdir);
                self.add(format!("SPHERICAL_SURFACE('',#{p},{})", real(*radius)))
            }
            Surface::Torus { centre, axis, xdir, major, minor } => {
                let p = self.placement(*centre, *axis, *xdir);
                self.add(format!("TOROIDAL_SURFACE('',#{p},{},{})", real(*major), real(*minor)))
            }
        }
    }
    fn curve(&mut self, c: &Curve) -> usize {
        match c {
            Curve::Line { origin, direction } => {
                let (p, d) = (self.point(*origin), self.direction(*direction));
                let v = self.add(format!("VECTOR('',#{d},1.)"));
                self.add(format!("LINE('',#{p},#{v})"))
            }
            Curve::Circle { centre, axis, xdir, radius } => {
                let p = self.placement(*centre, *axis, *xdir);
                self.add(format!("CIRCLE('',#{p},{})", real(*radius)))
            }
        }
    }
    fn loop_(&mut self, uses: &[Use], edge_ids: &[usize]) -> usize {
        let oriented: Vec<String> = uses
            .iter()
            .map(|u| {
                let id = self.add(format!("ORIENTED_EDGE('',*,*,#{},{})", edge_ids[u.edge], if u.forward { ".T." } else { ".F." }));
                format!("#{id}")
            })
            .collect();
        self.add(format!("EDGE_LOOP('',({}))", oriented.join(",")))
    }
}

impl NativeBrep {
    /// The STEP text of the solid (length unit millimetre).
    pub fn to_step_text(&self) -> String {
        let mut w = Writer { lines: Vec::new() };
        // Product skeleton (the numbering below is fixed; the body is added after it).
        w.add("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#2)".into());
        w.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')".into());
        w.add("SHAPE_DEFINITION_REPRESENTATION(#4,#10)".into());
        w.add("PRODUCT_DEFINITION_SHAPE('','',#5)".into());
        w.add("PRODUCT_DEFINITION('design','',#6,#9)".into());
        w.add("PRODUCT_DEFINITION_FORMATION('','',#7)".into());
        w.add("PRODUCT('Native solid','Native solid','',(#8))".into());
        w.add("PRODUCT_CONTEXT('',#2,'mechanical')".into());
        w.add("PRODUCT_DEFINITION_CONTEXT('part definition',#2,'design')".into());
        // #10 is the shape representation; its item list and context are filled in once their ids are known.
        let rep_slot = w.lines.len();
        w.lines.push(String::new());
        let origin = w.placement([0.; 3], [0., 0., 1.], [1., 0., 0.]);
        let vertices: Vec<usize> = self
            .vertices
            .iter()
            .map(|&p| {
                let pt = w.point(p);
                w.add(format!("VERTEX_POINT('',#{pt})"))
            })
            .collect();
        let edges: Vec<usize> = self
            .edges
            .iter()
            .map(|e| {
                let c = w.curve(&e.curve);
                w.add(format!("EDGE_CURVE('',#{},#{},#{c},.T.)", vertices[e.start], vertices[e.end]))
            })
            .collect();
        let mut face_ids = Vec::new();
        for f in &self.faces {
            let surface = w.surface(&f.surface);
            let mut bounds = Vec::new();
            let outer = w.loop_(&f.outer, &edges);
            bounds.push(format!("#{}", w.add(format!("FACE_OUTER_BOUND('',#{outer},.T.)"))));
            for h in &f.holes {
                let l = w.loop_(h, &edges);
                bounds.push(format!("#{}", w.add(format!("FACE_BOUND('',#{l},.T.)"))));
            }
            let id = w.add(format!("ADVANCED_FACE('',({}),#{surface},{})", bounds.join(","), if f.same_sense { ".T." } else { ".F." }));
            face_ids.push(format!("#{id}"));
        }
        let shell = w.add(format!("CLOSED_SHELL('',({}))", face_ids.join(",")));
        let solid = w.add(format!("MANIFOLD_SOLID_BREP('',#{shell})"));
        let ctx = w.lines.len() + 1;
        let (len_u, ang_u, sol_u, unc) = (ctx + 1, ctx + 2, ctx + 3, ctx + 4);
        w.add(format!(
            "( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{unc})) GLOBAL_UNIT_ASSIGNED_CONTEXT ((#{len_u},#{ang_u},#{sol_u})) REPRESENTATION_CONTEXT('Context #1','3D Context with UNIT and UNCERTAINTY') )"
        ));
        w.add("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )".into());
        w.add("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )".into());
        w.add("( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() )".into());
        w.add(format!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#{len_u},'distance_accuracy_value','confusion accuracy')"));
        w.add("PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#7))".into());
        w.lines[rep_slot] = format!("#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{origin},#{solid}),#{ctx});");
        let mut out = String::from(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Native solid model'),'2;1');\nFILE_NAME('Native solid model','1979-01-01T00:00:00',('Author'),('solid-kernel'),'solid-kernel native_brep','solid-kernel','Unknown');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\nENDSEC;\nDATA;\n",
        );
        for l in &w.lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        out
    }
}
