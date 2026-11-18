//! Text DXF fixtures: entities and GEODATA are written as native group codes.
use super::*;
pub(super) const fn supports(element: E) -> bool {
    matches!(element, E::CoordinatePrecision | E::SignedZero | E::Z)
}
pub(super) fn write(path: &Path, source: &RecordBatch) -> RecordBatch {
    let column = source
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let geometry = decode_wkb(column.value(0), &WkbLimits::default()).unwrap();
    let WkbValue::Point(c) = geometry.value else {
        panic!("point required")
    };
    let definition = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#;
    let text=format!("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1027\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nPOINT\n8\n0\n10\n{}\n20\n{}\n30\n{}\n0\nENDSEC\n0\nSECTION\n2\nOBJECTS\n0\nGEODATA\n100\nAcDbGeoData\n90\n2\n301\n{definition}\n0\nENDSEC\n0\nEOF\n",c.x,c.y,c.z.unwrap_or(0.0));
    std::fs::write(path, text).unwrap();
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata([("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned())]),
            Field::new("layer", DataType::Utf8, true),
            Field::new("dxf_type", DataType::Utf8, true),
            Field::new("text", DataType::Utf8, true),
        ])),
        vec![
            source.column(0).clone(),
            Arc::new(StringArray::from(vec!["0"])),
            Arc::new(StringArray::from(vec!["POINT"])),
            Arc::new(StringArray::from(vec![None::<&str>])),
        ],
    )
    .unwrap()
}
