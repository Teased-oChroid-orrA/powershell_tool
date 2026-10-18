//! Generates `tests/data/assembly-two-instances.step` (one box part, two placed instances, written by hand around an OpenCASCADE
//! export of the box) and prints how OpenCASCADE places it, the independent reference for `step_assembly`.
//! `cargo run -p solid-kernel --features occt --example assembly-probe [-- write]`
use solid_kernel::occt::Brep;

fn assembly() -> String {
    let base = Brep::box_from_corners([0., 0., 0.], [2., 1., 3.]).to_step_text().unwrap();
    let (child_rep, child_axis, ctx) = ("#10", "#11", "#345");
    let mut extra = String::new();
    extra.push_str(&format!("#900 = PRODUCT('ASSY','ASSY','',(#8));\n#901 = PRODUCT_DEFINITION_FORMATION('','',#900);\n#902 = PRODUCT_DEFINITION('design','',#901,#9);\n#903 = PRODUCT_DEFINITION_SHAPE('','',#902);\n#904 = SHAPE_REPRESENTATION('',(#905,#931,#941),{ctx});\n#905 = AXIS2_PLACEMENT_3D('',#906,#907,#908);\n#906 = CARTESIAN_POINT('',(0.,0.,0.));\n#907 = DIRECTION('',(0.,0.,1.));\n#908 = DIRECTION('',(1.,0.,0.));\n#910 = SHAPE_DEFINITION_REPRESENTATION(#903,#904);\n"));
    // Instance A: x' = +y, z' = +z, at (10, 5, 0). Instance B: z' = +y, x' = +x, at (-6, 3, 4).
    for (nauo, pds, itd, place, rr, cdsr, loc, axis, refd, loc_id) in [
        (920, 921, 930, 931, 932, 933, "(10.,5.,0.)", "(0.,0.,1.)", "(0.,1.,0.)", 934),
        (940, 951, 950, 941, 952, 953, "(-6.,3.,4.)", "(0.,1.,0.)", "(1.,0.,0.)", 944),
    ] {
        extra.push_str(&format!("#{nauo} = NEXT_ASSEMBLY_USAGE_OCCURRENCE('{nauo}','','',#902,#5,$);\n#{pds} = PRODUCT_DEFINITION_SHAPE('Placement','Placement of an item',#{nauo});\n#{itd} = ITEM_DEFINED_TRANSFORMATION('','',{child_axis},#{place});\n#{place} = AXIS2_PLACEMENT_3D('',#{loc_id},#{a},#{r});\n#{loc_id} = CARTESIAN_POINT('',{loc});\n#{a} = DIRECTION('',{axis});\n#{r} = DIRECTION('',{refd});\n#{rr} = ( REPRESENTATION_RELATIONSHIP('','',{child_rep},#904) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{itd}) SHAPE_REPRESENTATION_RELATIONSHIP() );\n#{cdsr} = CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{rr},#{pds});\n", a = loc_id + 1000, r = loc_id + 2000));
    }
    let at = base.rfind("ENDSEC;").expect("end of data");
    format!("{}{}{}", &base[..at], extra, &base[at..])
}

fn main() {
    let text = assembly();
    if std::env::args().any(|a| a == "write") {
        std::fs::write("solid-kernel/tests/data/assembly-two-instances.step", &text).unwrap();
    }
    let b = Brep::from_step_text(&text).unwrap();
    let (lo, hi) = b.bounds();
    println!("OpenCASCADE bounds of the assembly: {lo:?} .. {hi:?}");
}
