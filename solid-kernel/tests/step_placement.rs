//! Native STEP import of parts inside product structure (issue #46, Phase 168): a mapped item that only shows the part in a PMI
//! annotation view places nothing; a part placed by an assembly usage (or a shape mapped item) is read in the root's frame, one
//! model per instance; a degree plane-angle unit stated in SI radians scales a cone's semi-angle. Fixtures: `tests/data/README.md`.
//! Everything that cannot be placed rigidly and unambiguously is refused with its reason.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::native_surface::Limits as SurfaceLimits;
use solid_kernel::step::{inventory, Limits};
use solid_kernel::step_body::BodyContext;

const PLATE: &str = include_str!("data/edit-plate.step");
const PMI: &str = include_str!("data/pmi-view-single-part.step");
const PLACED: &str = include_str!("data/placed-single-part.step");
const TWO: &str = include_str!("data/assembly-two-instances.step");
const MAPPED: &str = include_str!("data/assembly-mapped-items.step");
const CONE: &str = include_str!("data/cone.step");
const CONE_DEG: &str = include_str!("data/cone-degrees.step");

fn read(source: &str) -> Result<AnalyticModel, String> {
    let data = inventory(source, Limits::default()).map_err(|e| e.to_string())?;
    AnalyticModel::read(&data, Options::default()).map_err(|e| e.detail)
}

fn read_all(source: &str) -> Result<Vec<AnalyticModel>, String> {
    let data = inventory(source, Limits::default()).map_err(|e| e.to_string())?;
    AnalyticModel::read_all(&data, Options::default()).map_err(|e| e.detail)
}

fn refused(result: Result<impl Sized, String>, expect: &str) {
    match result {
        Ok(_) => panic!("expected a refusal containing {expect:?}"),
        Err(e) => assert!(e.contains(expect), "{expect:?} not in {e:?}"),
    }
}

fn sub(s: &str, from: &str, to: &str) -> String {
    assert_eq!(s.matches(from).count(), 1, "{from}");
    s.replace(from, to)
}

fn extent(m: &AnalyticModel) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for id in m.body().vertices.keys() {
        let p = m.vertex(*id).unwrap();
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-12)
}

fn same_vertices(a: &AnalyticModel, b: &AnalyticModel) {
    assert_eq!(a.body().vertices.len(), b.body().vertices.len());
    for id in a.body().vertices.keys() {
        assert_eq!(a.vertex(*id), b.vertex(*id), "vertex #{id}");
    }
}

#[test]
fn a_part_shown_in_a_pmi_view_reads_exactly_like_the_bare_part() {
    let bare = read(PLATE).unwrap();
    same_vertices(&read(PMI).unwrap(), &bare);
    // AP242 writes the view as a complex draughting model that is also a shape representation
    let complex = sub(
        PMI,
        "#510 = DRAUGHTING_MODEL('PMI view',(#505),#429);",
        "#510 = ( DRAUGHTING_MODEL() REPRESENTATION('PMI view',(#505),#429) SHAPE_REPRESENTATION() );",
    );
    same_vertices(&read(&complex).unwrap(), &bare);
    assert_eq!(read_all(PMI).unwrap().len(), 1);
}

#[test]
fn the_same_mapped_item_in_a_shape_representation_places_the_part() {
    // Held by a shape representation, the mapped item is product structure: the plate is moved by the item's placement
    // (local x -> +x, local z -> +y, so local y -> -z, at (100, 50, 0)). The plate is 40 x 30 x 10 from the origin.
    let shape = sub(
        PMI,
        "#510 = DRAUGHTING_MODEL('PMI view',(#505),#429);",
        "#510 = SHAPE_REPRESENTATION('placed',(#505),#429);",
    );
    let m = read(&shape).unwrap();
    let (lo, hi) = extent(&m);
    assert!(near(lo, [100., 50., -30.]) && near(hi, [140., 60., 0.]), "{lo:?}..{hi:?}");
    // the placed model lives in its own moved document
    let data = inventory(&shape, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    assert_ne!(context.document(&data).document_fingerprint(), data.document_fingerprint());
    assert_eq!(context.placement().translation, [100., 50., 0.]);
    assert_eq!(m.document_fingerprint(), context.document(&data).document_fingerprint());
}

#[test]
fn a_mapped_item_that_places_nothing_or_is_malformed_is_refused() {
    // in no representation at all
    refused(read(&sub(PMI, "#510 = DRAUGHTING_MODEL('PMI view',(#505),#429);\n", "")), "item of no representation");
    // its map is not a REPRESENTATION_MAP
    refused(read(&sub(PMI, "#505 = MAPPED_ITEM('',#504,#506);", "#505 = MAPPED_ITEM('',#500,#506);")), "is not a REPRESENTATION_MAP");
    refused(read(&sub(PMI, "#505 = MAPPED_ITEM('',#504,#506);", "#505 = MAPPED_ITEM('',#504);")), "malformed MAPPED_ITEM");
}

#[test]
fn a_part_placed_by_an_assembly_usage_reads_in_the_root_frame() {
    // One instance of the 2 x 1 x 3 box: local x -> +y, local y -> -x at (10, 5, 0), so it lands on x 9..10, y 5..7, z 0..3. The
    // solid sits in an ADVANCED_BREP_SHAPE_REPRESENTATION related to the part's placement-only SHAPE_REPRESENTATION.
    let m = read(PLACED).unwrap();
    let (lo, hi) = extent(&m);
    assert!(near(lo, [9., 5., 0.]) && near(hi, [10., 7., 3.]), "{lo:?}..{hi:?}");
    let data = inventory(PLACED, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    assert_eq!(context.placement().rotation, [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]]);
    // every vertex is exactly the placement applied to the source coordinates
    let raw = data.raw_bodies().unwrap().remove(0);
    for (id, v) in &raw.vertices {
        assert!(near(m.vertex(*id).unwrap(), context.placement().apply(v.coordinates)));
    }
    // the placed shell samples like any other
    let surface = m.sampled_surface(SurfaceLimits::default(), |_| true).unwrap();
    assert!(!surface.mesh.triangles.is_empty());
}

#[test]
fn placements_that_are_not_rigid_unambiguous_or_consistent_are_refused() {
    // a scaling (or mirroring) operator instead of an axis placement
    let scaled = sub(
        PLACED,
        "#931 = AXIS2_PLACEMENT_3D('',#934,#1934,#2934);",
        "#931 = CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,#1934,#2934,#934,2.,$);",
    );
    refused(read(&scaled), "only a rigid AXIS2_PLACEMENT_3D");
    // the solid's representation is in metres, the part's in millimetres
    let metres = sub(
        PLACED,
        "#960 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#15),#345);",
        "#960 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#15),#970);\n#970 = ( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#971)) REPRESENTATION_CONTEXT('','') );\n#971 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.) );",
    );
    refused(read(&metres), "different length units");
    // a usage whose part is its own assembly: a cycle, so no root
    refused(
        read(&sub(PLACED, "NEXT_ASSEMBLY_USAGE_OCCURRENCE('920','','',#902,#5,$)", "NEXT_ASSEMBLY_USAGE_OCCURRENCE('920','','',#5,#5,$)")),
        "0 root product definitions",
    );
    // a placement that belongs to both representations: which way it maps is not guessed
    refused(
        read(&sub(PLACED, "#904 = SHAPE_REPRESENTATION('',(#905,#931),#345);", "#904 = SHAPE_REPRESENTATION('',(#905,#931,#11),#345);")),
        "cannot tell",
    );
    // a mapped representation that maps itself (a cycle)
    refused(read_all(&sub(MAPPED, "#900 = REPRESENTATION_MAP(#11,#10);", "#900 = REPRESENTATION_MAP(#902,#901);")), "root representations");
    // a second solid that no instance places
    let stray = sub(
        PLACED,
        "#961 = SHAPE_REPRESENTATION_RELATIONSHIP('','',#10,#960);",
        "#961 = SHAPE_REPRESENTATION_RELATIONSHIP('','',#10,#960);\n#980 = MANIFOLD_SOLID_BREP('',#16);\n#981 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#980),#345);",
    );
    refused(read_all(&stray), "solid #980 is not placed by the product structure");
}

#[test]
fn an_assembly_reads_one_model_per_instance_and_the_single_body_read_refuses_it() {
    for source in [TWO, MAPPED] {
        let models = read_all(source).unwrap();
        assert_eq!(models.len(), 2);
        let mut extents: Vec<_> = models.iter().map(extent).collect();
        extents.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        assert!(near(extents[0].0, [-6., 3., 3.]) && near(extents[0].1, [-4., 6., 4.]), "{:?}", extents[0]);
        assert!(near(extents[1].0, [9., 5., 0.]) && near(extents[1].1, [10., 7., 3.]), "{:?}", extents[1]);
        // one solid, two instances: there is no single body
        refused(read(source), "exactly one supported body");
    }
}

#[test]
fn a_degree_unit_stated_in_si_radians_scales_the_cone_semi_angle() {
    let (radians, degrees) = (read(CONE).unwrap(), read(CONE_DEG).unwrap());
    let sample = |m: &AnalyticModel| m.sampled_surface(SurfaceLimits::default(), |_| true).unwrap().mesh;
    let (a, b) = (sample(&radians), sample(&degrees));
    assert_eq!(a.vertices.len(), b.vertices.len());
    let worst = a
        .vertices
        .iter()
        .zip(&b.vertices)
        .map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).fold(0., f64::max))
        .fold(0., f64::max);
    // 32.00538320810856 degrees x 0.0174532925199433 is the radian value to a few ulps
    assert!(worst < 1e-12, "{worst}");
}

#[test]
fn plane_angle_units_that_are_not_stated_in_si_radians_or_are_ambiguous_are_refused() {
    let unit = "#88 = PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925199433),#90);";
    // the conversion's basis is not the SI radian
    refused(read(&sub(CONE_DEG, unit, &unit.replace("#90)", "#85)"))), "unsupported plane-angle unit");
    // the conversion is not a plane-angle measure
    refused(read(&sub(CONE_DEG, unit, &unit.replace("PLANE_ANGLE_MEASURE(", "LENGTH_MEASURE("))), "unsupported plane-angle unit");
    // a non-zero dimension
    refused(
        read(&sub(CONE_DEG, "#89 = DIMENSIONAL_EXPONENTS(0.,0.,0.,0.,0.,0.,0.);", "#89 = DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);")),
        "unsupported plane-angle unit",
    );
    // a prefixed radian
    refused(
        read(&sub(CONE, "#84 = ( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );", "#84 = ( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT(.MILLI.,.RADIAN.) );")),
        "unsupported plane-angle unit",
    );
    // two plane-angle units assigned in the body's context
    refused(read(&sub(CONE_DEG, "(#83,#84,#85))", "(#83,#84,#85,#90))")), "ambiguous plane-angle units");
    // none assigned
    refused(read(&sub(CONE, "(#83,#84,#85))", "(#83,#85))")), "needs a plane-angle unit assigned");
}

#[test]
fn a_solid_listed_by_several_representations_of_one_context_or_beside_a_curve_set_binds_unambiguously() {
    let bare = read(PLATE).unwrap();
    // an AP203 validation-property representation that lists the solid again, in the same context
    let validation = sub(
        PLATE,
        "#434 = PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#7));",
        "#434 = PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#7));\n#520 = SHAPE_REPRESENTATION('',(#15),#429);",
    );
    same_vertices(&read(&validation).unwrap(), &bare);
    // datum-target wireframe beside the solid
    let curves = sub(
        PLATE,
        "#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#11,#15),#429);",
        "#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#11,#15,#520),#429);\n#520 = GEOMETRIC_CURVE_SET('datum target',(#12));",
    );
    same_vertices(&read(&curves).unwrap(), &bare);
    // a general geometric set may hold surfaces: still refused
    refused(
        read(&sub(
            PLATE,
            "#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#11,#15),#429);",
            "#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#11,#15,#520),#429);\n#520 = GEOMETRIC_SET('',(#12));",
        )),
        "unsupported representation item",
    );
}
