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

#[test]
fn actual_bracket_retains_every_body_face_loop_and_edge_identity() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let bodies = data.raw_bodies().unwrap();
    assert_eq!(bodies.len(), 1);
    let b = &bodies[0];
    assert_eq!(b.faces.len(), 38);
    assert_eq!(b.edges.len(), 114);
    assert_eq!(b.vertices.len(), 70);
    assert_eq!(b.faces.values().map(|f| f.bounds.len()).sum::<usize>(), 52);
    assert_eq!(
        b.faces
            .values()
            .flat_map(|f| &f.bounds)
            .map(|b| b.edges.len())
            .sum::<usize>(),
        228
    );
    assert!(
        b.faces
            .values()
            .flat_map(|f| &f.bounds)
            .all(|b| !b.explicitly_outer),
        "unspecified outer-loop roles must not be guessed"
    );
}
#[test]
fn broken_or_partial_body_topology_is_refused() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let body = inventory(source, Limits::default())
        .unwrap()
        .raw_bodies()
        .unwrap()
        .remove(0);
    let shell = body.shell;
    let start = source.find(&format!("#{shell} = CLOSED_SHELL")).unwrap();
    let end = start + source[start..].find(';').unwrap() + 1;
    let face = *body.faces.keys().next().unwrap();
    let changed = format!(
        "{}#{shell}=CLOSED_SHELL('',(#{face}));{}",
        &source[..start],
        &source[end..]
    );
    let data = inventory(&changed, Limits::default()).unwrap();
    assert_eq!(data.raw_bodies().unwrap_err().kind, ErrorKind::Topology);
}

fn replace_record(source: &str, offset: usize, record: &str) -> String {
    let end = offset + source[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &source[..offset], record, &source[end..])
}
#[test]
fn broken_loop_is_rejected_without_geometric_welding_or_partial_success() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let bodies = data.raw_bodies().unwrap();
    let body = &bodies[0];
    let bound = body
        .faces
        .values()
        .flat_map(|f| &f.bounds)
        .find(|b| {
            b.edges
                .iter()
                .any(|u| body.edges[&u.edge].start != body.edges[&u.edge].end)
        })
        .unwrap();
    let remove = bound
        .edges
        .iter()
        .position(|u| body.edges[&u.edge].start != body.edges[&u.edge].end)
        .unwrap();
    let ids = bound
        .edges
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != remove)
        .map(|(_, u)| format!("#{}", u.id))
        .collect::<Vec<_>>()
        .join(",");
    let changed = replace_record(
        source,
        data.entities()[&bound.loop_id].offset,
        &format!("#{}=EDGE_LOOP('',({ids}));", bound.loop_id),
    );
    let error = inventory(&changed, Limits::default())
        .unwrap()
        .raw_bodies()
        .unwrap_err();
    assert!(error.detail.contains("not closed"), "{error}");
}
#[test]
fn duplicated_faces_and_an_additional_unsupported_body_are_not_silently_skipped() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let mut ids = body
        .faces
        .keys()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>();
    ids.push(ids[0].clone());
    let changed = replace_record(
        source,
        data.entities()[&body.shell].offset,
        &format!("#{}=CLOSED_SHELL('',({}));", body.shell, ids.join(",")),
    );
    assert!(inventory(&changed, Limits::default())
        .unwrap()
        .raw_bodies()
        .unwrap_err()
        .detail
        .contains("repeated face"));
    let end = source.rfind("ENDSEC;").unwrap();
    let changed = format!(
        "{}#999999=BREP_WITH_VOIDS('',#{},());\n{}",
        &source[..end],
        body.shell,
        &source[end..]
    );
    assert!(inventory(&changed, Limits::default())
        .unwrap()
        .raw_bodies()
        .unwrap_err()
        .detail
        .contains("unsupported body kind"));
}

#[test]
fn topology_expansion_budgets_are_cumulative_across_shared_shells() {
    use solid_kernel::step::TopologyLimits;
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let end = source.rfind("ENDSEC;").unwrap();
    let repeated = format!(
        "{}#999999=MANIFOLD_SOLID_BREP('second',#{});\n{}",
        &source[..end],
        body.shell,
        &source[end..]
    );
    let shared = inventory(&repeated, Limits::default()).unwrap();
    assert_eq!(shared.raw_bodies().unwrap().len(), 2);
    for limits in [
        TopologyLimits {
            bodies: 1,
            ..TopologyLimits::default()
        },
        TopologyLimits {
            faces: 38,
            ..TopologyLimits::default()
        },
        TopologyLimits {
            edges: 114,
            ..TopologyLimits::default()
        },
        TopologyLimits {
            vertices: 70,
            ..TopologyLimits::default()
        },
        TopologyLimits {
            edge_uses: 228,
            ..TopologyLimits::default()
        },
        TopologyLimits {
            label_bytes: 5,
            ..TopologyLimits::default()
        },
    ] {
        assert_eq!(
            shared.raw_bodies_with_limits(limits).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
}

#[test]
fn source_scoped_references_survive_reload_but_refuse_reused_ids() {
    use solid_kernel::step::{DocumentFingerprint, EntityReference};
    let text = file("#1=THING('original');");
    let first = inventory(&text, Limits::default()).unwrap();
    let reloaded = inventory(&text, Limits::default()).unwrap();
    // Independently calculated SHA-256 of the exact fixture bytes.
    assert_eq!(
        first.document_fingerprint(),
        DocumentFingerprint([
            112, 230, 40, 32, 149, 49, 213, 65, 26, 24, 170, 253, 105, 14, 14, 57, 122, 202, 221,
            210, 178, 16, 31, 2, 129, 245, 156, 237, 107, 116, 25, 222
        ])
    );
    let reference = first.entity_reference(1).unwrap();
    assert_eq!(reference, reloaded.entity_reference(1).unwrap());
    assert_eq!(reloaded.resolve_reference(reference).unwrap().id, 1);
    assert!(first.entity_reference(999).is_none());
    let missing = EntityReference {
        document: first.document_fingerprint(),
        entity: 999,
    };
    assert_eq!(
        first.resolve_reference(missing).unwrap_err().kind,
        ErrorKind::DanglingReference
    );
    for changed in [file("#1=THING('different');"), format!("{text}\n")] {
        let other = inventory(&changed, Limits::default()).unwrap();
        assert_ne!(first.document_fingerprint(), other.document_fingerprint());
        assert!(other.entity_reference(1).is_some());
        assert_eq!(
            other.resolve_reference(reference).unwrap_err().kind,
            ErrorKind::DanglingReference
        );
    }
    let forged = EntityReference {
        document: DocumentFingerprint([0; 32]),
        entity: 1,
    };
    assert!(first.resolve_reference(forged).is_err());
}
