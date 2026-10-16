use solid_kernel::step::{inventory, ErrorKind, Limits, Value};
fn file(body: &str) -> String {
    format!("ISO-10303-21; HEADER; FILE_DESCRIPTION(('inventory'),'2;1'); FILE_NAME('fixture','',(''),(''),'','',''); FILE_SCHEMA(('TEST')); ENDSEC; DATA; {body} ENDSEC; END-ISO-10303-21;")
}
#[test]
fn actual_bracket_has_all_faces_and_resolved_references() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    assert_eq!(data.counts()["ADVANCED_FACE"], 38);
    assert!(data.entities().len() > 1000);
    assert!(data.counts().contains_key("MANIFOLD_SOLID_BREP"));
    assert!(data.counts().contains_key("SI_UNIT"));
}
#[test]
fn lexical_constructs_and_complex_components_are_preserved() {
    let text=file("/* outside */ #1=THING('a''b;/*inside*/',#2,(1.,-2.3E-4,.5),LENGTH_MEASURE(2.),$,*); #2=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));");
    let data = inventory(&text, Limits::default()).unwrap();
    let e = &data.entities()[&1].components[0];
    assert_eq!(e.args[0], Value::String("a'b;/*inside*/".into()));
    assert_eq!(e.args[1], Value::Ref(2));
    assert_eq!(data.entities()[&2].components.len(), 3);
    assert_eq!(data.counts()["SI_UNIT"], 1);
}
#[test]
fn malformed_partial_and_unsupported_files_are_not_accepted() {
    for (body, kind) in [
        ("#1=A(#99);", ErrorKind::DanglingReference),
        ("#1=A(); #1=B();", ErrorKind::DuplicateEntity),
        ("#1=A(1e999);", ErrorKind::InvalidNumber),
        ("#1=A('open);", ErrorKind::Syntax),
        ("#1=();", ErrorKind::Syntax),
        ("#0=A();", ErrorKind::Syntax),
    ] {
        assert_eq!(
            inventory(&file(body), Limits::default()).unwrap_err().kind,
            kind,
            "{body}"
        );
    }
    assert!(inventory(&format!("{} trailing", file("#1=A();")), Limits::default()).is_err());
    let named = file("#1=A();").replace("DATA;", "DATA('x',('TEST'));");
    assert_eq!(
        inventory(&named, Limits::default()).unwrap_err().kind,
        ErrorKind::UnsupportedSection
    );
    assert!(inventory("ISO-10303-21; HEADER; /*", Limits::default()).is_err());
}
#[test]
fn budgets_prevent_unbounded_nesting_strings_and_entities() {
    let text = file("#1=A('12345',(1.,(2.,(3.)))); #2=B();");
    for limits in [
        Limits {
            bytes: 10,
            ..Limits::default()
        },
        Limits {
            entities: 1,
            ..Limits::default()
        },
        Limits {
            values: 1,
            ..Limits::default()
        },
        Limits {
            depth: 1,
            ..Limits::default()
        },
        Limits {
            string_bytes: 4,
            ..Limits::default()
        },
    ] {
        assert_eq!(inventory(&text, limits).unwrap_err().kind, ErrorKind::Limit);
    }
}

#[test]
fn integer_values_are_exact_and_underflow_is_not_silent() {
    let text = file("#1=THING(9007199254740993,0.E-999,1.E-3);");
    let parsed = inventory(&text, Limits::default()).unwrap();
    assert_eq!(
        parsed.entities()[&1].components[0].args[0],
        Value::Integer(9007199254740993)
    );
    assert_eq!(parsed.entities()[&1].components[0].args[1], Value::Real(0.));
    assert_eq!(
        inventory(&file("#1=A(1.E-999);"), Limits::default())
            .unwrap_err()
            .kind,
        ErrorKind::InvalidNumber
    );
}

#[test]
fn mandatory_header_and_all_retained_references_are_checked() {
    let text = file("#1=A();");
    assert!(inventory(
        &text.replace("FILE_SCHEMA(('TEST'));", ""),
        Limits::default()
    )
    .is_err());
    let bad = text.replace("('inventory')", "(#999)");
    assert_eq!(
        inventory(&bad, Limits::default()).unwrap_err().kind,
        ErrorKind::DanglingReference
    );
    let body = file("#7=A(#123);");
    let e = inventory(&body, Limits::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::DanglingReference);
    assert_eq!(&body[e.offset..e.offset + 2], "#7");
}

fn si() -> &'static str {
    "#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));"
}
#[test]
fn assigned_units_come_from_factors_not_labels_and_keep_contexts_separate() {
    let text=file(&format!("{} #2=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.); #3=(LENGTH_UNIT() NAMED_UNIT(#2) CONVERSION_BASED_UNIT('misleading label',#4)); #4=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1); #5=GLOBAL_UNIT_ASSIGNED_CONTEXT((#3)); #6=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1));",si()));
    let data = inventory(&text, Limits::default()).unwrap();
    let contexts = data.length_contexts().unwrap();
    assert_eq!(contexts.len(), 2);
    assert!((contexts[0].metres - 0.0254).abs() < 1e-15);
    assert_eq!(contexts[1].metres, 0.001);
    let actual = inventory(
        include_str!("../../nema17-motor-bracket.step"),
        Limits::default(),
    )
    .unwrap();
    assert!(actual
        .length_contexts()
        .unwrap()
        .iter()
        .all(|c| c.metres == 0.001));
}
#[test]
fn ambiguous_missing_cyclic_and_invalid_unit_factors_are_refused() {
    let text = file(&format!(
        "{} #2=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#1));",
        si()
    ));
    assert_eq!(
        inventory(&text, Limits::default())
            .unwrap()
            .length_contexts()
            .unwrap_err()
            .kind,
        ErrorKind::Units
    );
    assert!(inventory(&file(si()), Limits::default())
        .unwrap()
        .length_contexts()
        .is_err());
    for factor in ["0.", "-1.", "1."] {
        let text=file(&format!("#1=(LENGTH_UNIT() NAMED_UNIT(*) CONVERSION_BASED_UNIT('unit',#2)); #2=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE({factor}),#1); #3=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1));"));
        assert!(inventory(&text, Limits::default())
            .unwrap()
            .length_contexts()
            .is_err());
    }
    let unknown=file("#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.UNKNOWN.,.METRE.)); #2=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1));");
    assert!(inventory(&unknown, Limits::default())
        .unwrap()
        .length_contexts()
        .is_err());
}

#[test]
fn dimension_mismatch_and_unimplemented_unit_subtypes_are_not_ignored() {
    let wrong=file("#1=(LENGTH_UNIT() NAMED_UNIT(#2) SI_UNIT($,.METRE.)); #2=DIMENSIONAL_EXPONENTS(0.,1.,0.,0.,0.,0.,0.); #3=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1));");
    assert!(inventory(&wrong, Limits::default())
        .unwrap()
        .length_contexts()
        .is_err());
    let extra=file("#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.) CONVERSION_BASED_UNIT_WITH_OFFSET(10.)); #3=GLOBAL_UNIT_ASSIGNED_CONTEXT((#1));");
    assert!(inventory(&extra, Limits::default())
        .unwrap()
        .length_contexts()
        .is_err());
}
