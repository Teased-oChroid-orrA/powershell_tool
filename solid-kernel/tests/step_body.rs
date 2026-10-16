use solid_kernel::{
    step::{inventory, Limits, Value},
    step_body::BodyContext,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn replace(source: &str, offset: usize, record: &str) -> String {
    let end = offset + source[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &source[..offset], record, &source[end..])
}
#[test]
fn body_context_preserves_source_coordinates_and_document_bound_units() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let binding = BodyContext::read(&data).unwrap();
    assert_eq!(binding.millimetres_per_source_unit(), 1.);
    assert!(data.resolve_reference(binding.length_context()).is_ok());
    let context = data
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|c| c.context == binding.length_context().entity)
        .unwrap();
    let unit = &data.entities()[&context.unit];
    let changed = replace(
        SOURCE,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let metres = inventory(&changed, Limits::default()).unwrap();
    let metric = BodyContext::read(&metres).unwrap();
    assert_eq!(metric.millimetres_per_source_unit(), 1000.);
    for (id, vertex) in &binding.body().vertices {
        assert_eq!(vertex.point, metric.body().vertices[id].point);
        assert_eq!(vertex.coordinates, metric.body().vertices[id].coordinates);
    }
    assert!(metres.resolve_reference(binding.length_context()).is_err());
}
#[test]
fn ambiguous_or_missing_direct_representations_refuse() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let binding = BodyContext::read(&data).unwrap();
    let representation = data
        .entities()
        .values()
        .find(|e| e.components[0].name == "ADVANCED_BREP_SHAPE_REPRESENTATION")
        .unwrap();
    let ctx = match representation.components[0].args[2] {
        Value::Ref(id) => id,
        _ => panic!("fixture"),
    };
    let record = format!(
        "#999999=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{}),#{ctx});",
        binding.body().id
    );
    let end = SOURCE.rfind("ENDSEC;").unwrap();
    let duplicated = format!("{}{record}{}", &SOURCE[..end], &SOURCE[end..]);
    assert!(
        BodyContext::read(&inventory(&duplicated, Limits::default()).unwrap())
            .unwrap_err()
            .detail
            .contains("multiple body representations")
    );
    let absent = replace(
        SOURCE,
        representation.offset,
        &format!("#{}=SHAPE_REPRESENTATION('',(),#{ctx});", representation.id),
    );
    assert!(
        BodyContext::read(&inventory(&absent, Limits::default()).unwrap())
            .unwrap_err()
            .detail
            .contains("no direct body")
    );
}
