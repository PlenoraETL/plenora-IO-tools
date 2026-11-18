//! Native-only factories. These do not add shared Arrow element generators.
use super::*;
#[path = "group_b_read_parquet.rs"]
mod native_parquet;
const WKT: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#;

struct Witness {
    batch: RecordBatch,
    // Native properties with no Arrow representation. The returned contract
    // and fields are checked for their payload before counting an omission.
    properties: Vec<(E, String)>,
}

fn base() -> RecordBatch {
    geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point)
}

const fn contract_key(element: E) -> Option<(&'static str, &'static str, bool)> {
    Some(match element {
        E::ContractVersion => ("plenora.contract.version", "1", true),
        E::GeometryEncoding => ("plenora.geometry.encoding", "wkb", false),
        E::SpatialSemantics => ("plenora.geometry.spatial_semantics", "geometry", false),
        E::Precision => ("plenora.geometry.precision", "float64", false),
        E::TypesDeclaration => ("plenora.geometry.types_declaration", "exact", false),
        E::CrsResolution => ("plenora.geometry.crs_resolution", "resolved", false),
        E::CrsId => ("plenora.geometry.crs_id", "EPSG:4326", false),
        E::FieldIdentity => ("plenora.field_id", "0", false),
        E::ExtensionName => ("ARROW:extension:name", "geoarrow.wkb", false),
        E::AxisOrder => ("plenora.geometry.axis_order", "lon_lat", false),
        E::PrimaryGeometry => ("plenora.primary_geometry", "0", true),
        E::DefinitionFormat => ("plenora.geometry.crs_definition_format", "wkt", false),
        E::CrsDefinition => ("plenora.geometry.crs_definition", WKT, false),
        _ => return None,
    })
}

fn supports(driver: &str, element: E) -> bool {
    if matches!(driver, "shp" | "dxf" | "filegdb" | "gpkg") && contract_key(element).is_some() {
        return true;
    }
    match driver {
        "dxf" => matches!(
            element,
            E::Color
                | E::Linetype
                | E::Lineweight
                | E::Thickness
                | E::Handle
                | E::FeatureId
                | E::Xdata
                | E::ExtendedData
                | E::FeatureMetadata
                | E::PaperSpace
                | E::Styles
                | E::Utf8
                | E::EmptyString
                | E::Null
                | E::TextStyle
                | E::Width
                | E::NumberPrecision
                | E::Dimensions
        ),
        "gpkg" => matches!(
            element,
            E::Identifier
                | E::Description
                | E::DataColumns
                | E::LayerMetadata
                | E::Extensions
                | E::Styles
                | E::SqlConstraints
                | E::FeatureId
        ),
        "shp" => matches!(
            element,
            E::DbfDate
                | E::Temporal
                | E::SignedInteger
                | E::FieldNames
                | E::FieldOrder
                | E::EmptyPoint
                | E::GeometryType
                | E::RingOrientation
        ),
        "filegdb" => matches!(
            element,
            E::Aliases
                | E::EmptyString
                | E::FieldNames
                | E::FieldOrder
                | E::Binary
                | E::NullGeometry
                | E::M
                | E::Dimensions
                | E::GeometryType
                | E::RingOrientation
                | E::CoordinatePrecision
                | E::NumberPrecision
        ),
        _ => false,
    }
}

fn dxf_witness(path: &Path, element: E, changed: bool) -> Witness {
    let source = if element == E::NumberPrecision {
        geometry_batch(
            WkbValue::Point(coordinate(1.123_456_789_012_345, 1e-20)),
            GeometryType::Point,
        )
    } else if element == E::Dimensions && changed {
        generate(E::Z).unwrap().remove(0)
    } else {
        base()
    };
    let mut batch = super::dxf::write(path, &source);
    if element == E::Dimensions {
        batch = metadata(
            &batch,
            "plenora.geometry.dimensions",
            if changed { "xyz" } else { "xy" },
            false,
        );
    }
    let mut text = std::fs::read_to_string(path).unwrap();
    let mut properties = Vec::new();
    let (code, value) = match element {
        E::Color | E::Styles => ("62", "3"),
        E::Linetype => ("6", "DASHED"),
        E::Lineweight => ("370", "25"),
        E::Thickness => ("39", "3.5"),
        E::Handle | E::FeatureId => ("5", "A1"),
        E::Xdata | E::ExtendedData | E::FeatureMetadata => {
            ("1001", "NATIVE_TEST\n1000\nindependent-xdata")
        }
        E::PaperSpace => ("67", "1"),
        _ => ("", ""),
    };
    if changed && !code.is_empty() {
        text = text.replacen("0\nPOINT\n", &format!("0\nPOINT\n{code}\n{value}\n"), 1);
        properties.push((element, value.to_owned()));
    }
    if matches!(element, E::Utf8 | E::EmptyString | E::TextStyle | E::Null) {
        let value = if element == E::Utf8 {
            if changed {
                "  caf\u{e9}  "
            } else {
                "caf\u{e9}"
            }
        } else {
            ""
        };
        if element != E::Null {
            text = text.replacen("0\nPOINT\n", "0\nTEXT\n", 1);
            text = text.replacen(
                "8\n0\n",
                &format!("8\n0\n1\n{value}\n40\n2.5\n7\nNATIVE_STYLE\n"),
                1,
            );
            let mut arrays = batch.columns().to_vec();
            arrays[2] = Arc::new(StringArray::from(vec!["TEXT"]));
            arrays[3] = Arc::new(StringArray::from(vec![value]));
            batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
            properties.push((E::TextStyle, "NATIVE_STYLE".to_owned()));
            properties.push((E::GeometryType, "TEXT".to_owned()));
        }
    }
    if element == E::Width {
        let width = if changed { "3.5" } else { "0" };
        let entity =
            format!("0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n43\n{width}\n10\n1\n20\n2\n10\n3\n20\n4\n");
        let start = text.find("0\nPOINT\n").unwrap();
        let end = start + text[start..].find("0\nENDSEC\n").unwrap();
        text.replace_range(start..end, &entity);
        let line = geometry_batch(
            WkbValue::LineString(vec![coordinate(1.0, 2.0), coordinate(3.0, 4.0)]),
            GeometryType::LineString,
        );
        let mut arrays = batch.columns().to_vec();
        arrays[0] = line.column(0).clone();
        arrays[2] = Arc::new(StringArray::from(vec!["LWPOLYLINE"]));
        batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
        if changed {
            properties.push((element, width.to_owned()));
        }
    }
    std::fs::write(path, text).unwrap();
    Witness { batch, properties }
}

fn gpkg_witness(path: &Path, element: E, changed: bool) -> Witness {
    let mut batch = super::gpkg::write(path, &base(), element).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    if matches!(element, E::DefinitionFormat | E::CrsDefinition) {
        conn.execute(
            "UPDATE gpkg_spatial_ref_sys SET definition=?1 WHERE srs_id=4326",
            [WKT],
        )
        .unwrap();
    }
    let mut properties = Vec::new();
    let sql = match element {
        E::DataColumns => "CREATE TABLE gpkg_data_columns(table_name TEXT,column_name TEXT,name TEXT,title TEXT,description TEXT,mime_type TEXT,constraint_name TEXT); INSERT INTO gpkg_data_columns VALUES('features','geometry','native-name','native-title','native-description',NULL,NULL)",
        E::Styles => "CREATE TABLE layer_styles(f_table_name TEXT,styleQML TEXT); INSERT INTO layer_styles VALUES('features','native-style-payload')",
        E::Extensions => "CREATE TABLE gpkg_extensions(table_name TEXT,column_name TEXT,extension_name TEXT,definition TEXT,scope TEXT); INSERT INTO gpkg_extensions VALUES('features',NULL,'test_native','https://example.test/native','read-write')",
        E::LayerMetadata => "CREATE TABLE gpkg_metadata(id INTEGER PRIMARY KEY,md_scope TEXT,md_standard_uri TEXT,mime_type TEXT,metadata TEXT); INSERT INTO gpkg_metadata VALUES(1,'dataset','https://example.test/native','text/plain','native-layer-payload'); CREATE TABLE gpkg_metadata_reference(reference_scope TEXT,table_name TEXT,column_name TEXT,row_id_value INTEGER,timestamp TEXT,md_file_id INTEGER,md_parent_id INTEGER); INSERT INTO gpkg_metadata_reference VALUES('table','features',NULL,NULL,'2026-01-01T00:00:00Z',1,NULL)",
        E::SqlConstraints => "CREATE UNIQUE INDEX native_unique_geometry ON features(geometry)",
        _ => "",
    };
    if changed && !sql.is_empty() {
        conn.execute_batch(sql).unwrap();
        let payload = match element {
            E::DataColumns => "native-title",
            E::Styles => "native-style-payload",
            E::Extensions => "test_native",
            E::LayerMetadata => "native-layer-payload",
            E::SqlConstraints => "CREATE UNIQUE INDEX native_unique_geometry ON features(geometry)",
            _ => unreachable!(),
        };
        properties.push((element, payload.to_owned()));
    }
    if matches!(element, E::Identifier | E::Description) {
        let column = if element == E::Identifier {
            "identifier"
        } else {
            "description"
        };
        let value = if changed {
            "independent native label"
        } else {
            ""
        };
        conn.execute(&format!("UPDATE gpkg_contents SET {column}=?1"), [value])
            .unwrap();
        batch = metadata(&batch, &format!("gpkg.{column}"), value, true);
    }
    Witness { batch, properties }
}

fn shp_witness(path: &Path, element: E) -> Witness {
    if matches!(element, E::GeometryType | E::RingOrientation) {
        return shp_polygon(path);
    }
    let source = match element {
        E::DbfDate | E::Temporal => attribute(Arc::new(StringArray::from(vec!["19700102"])), true),
        E::SignedInteger => attribute(Arc::new(StringArray::from(vec!["-2147483648"])), true),
        E::FieldNames => attribute(
            Arc::new(StringArray::from(vec!["native field value"])),
            true,
        ),
        E::FieldOrder => attribute(Arc::new(StringArray::from(vec!["first"])), true),
        E::EmptyPoint => geometry_batch(
            WkbValue::Point(coordinate(f64::NAN, f64::NAN)),
            GeometryType::Point,
        ),
        _ => base(),
    };
    let mut batch = super::shp::write(path, &source);
    if matches!(element, E::DbfDate | E::Temporal | E::SignedInteger) {
        let file = path.join("features.dbf");
        let mut bytes = std::fs::read(&file).unwrap();
        let date = element != E::SignedInteger;
        bytes[43] = if date { b'D' } else { b'N' };
        // Replace the single fixed-size character slot with a native slot.
        let width: u8 = if date { 8 } else { 20 };
        bytes[48] = width;
        bytes[10..12].copy_from_slice(&(u16::from(width) + 1).to_le_bytes());
        bytes.truncate(65);
        bytes.push(b' ');
        if date {
            bytes.extend(b"19700102");
        } else {
            bytes.extend(format!("{:>20}", -2_147_483_648_i64).as_bytes());
        }
        bytes.push(26);
        std::fs::write(file, bytes).unwrap();
        let array: ArrayRef = if date {
            Arc::new(Date32Array::from(vec![1]))
        } else {
            Arc::new(Int64Array::from(vec![-2_147_483_648_i64]))
        };
        let fields = vec![
            batch.schema().field(0).clone(),
            Field::new("value", array.data_type().clone(), true),
        ];
        batch = RecordBatch::try_new(
            Arc::new(Schema::new(fields)),
            vec![batch.column(0).clone(), array],
        )
        .unwrap();
    }
    Witness {
        batch,
        properties: vec![],
    }
}

fn shp_polygon(path: &Path) -> Witness {
    // One clockwise exterior and one counterclockwise interior: both native
    // orientations must reach WKB unchanged. A SHP polygon is multipart.
    let rings = vec![
        vec![
            coordinate(0.0, 0.0),
            coordinate(0.0, 4.0),
            coordinate(4.0, 4.0),
            coordinate(4.0, 0.0),
            coordinate(0.0, 0.0),
        ],
        vec![
            coordinate(1.0, 1.0),
            coordinate(2.0, 1.0),
            coordinate(2.0, 2.0),
            coordinate(1.0, 2.0),
            coordinate(1.0, 1.0),
        ],
    ];
    let mut batch = super::shp::write(path, &base());
    let mut record = 5_i32.to_le_bytes().to_vec();
    for v in [0.0_f64, 0.0, 4.0, 4.0] {
        record.extend(v.to_le_bytes());
    }
    for n in [2_i32, 10, 0, 5] {
        record.extend(n.to_le_bytes());
    }
    for c in rings.iter().flatten() {
        record.extend(c.x.to_le_bytes());
        record.extend(c.y.to_le_bytes());
    }
    let words = i32::try_from(record.len() / 2).unwrap();
    let shp_path = path.join("features.shp");
    let mut shp = std::fs::read(&shp_path).unwrap();
    shp.truncate(100);
    shp[24..28].copy_from_slice(&(54 + words).to_be_bytes());
    shp[32..36].copy_from_slice(&5_i32.to_le_bytes());
    for (i, v) in [0.0_f64, 0.0, 4.0, 4.0].into_iter().enumerate() {
        shp[36 + 8 * i..44 + 8 * i].copy_from_slice(&v.to_le_bytes());
    }
    let mut shx = shp.clone();
    shx[24..28].copy_from_slice(&54_i32.to_be_bytes());
    shx.extend(50_i32.to_be_bytes());
    shx.extend(words.to_be_bytes());
    shp.extend(1_i32.to_be_bytes());
    shp.extend(words.to_be_bytes());
    shp.extend(record);
    std::fs::write(shp_path, shp).unwrap();
    std::fs::write(path.join("features.shx"), shx).unwrap();
    let expected = geometry_batch(
        WkbValue::MultiPolygon(vec![super::super::super::child(WkbValue::Polygon(rings))]),
        GeometryType::MultiPolygon,
    );
    batch = RecordBatch::try_new(batch.schema(), expected.columns().to_vec()).unwrap();
    Witness {
        batch,
        properties: vec![],
    }
}

fn payload_present(batch: &RecordBatch, element: E, payload: &str) -> bool {
    // A coincidentally equal value in an unrelated field (e.g. contract
    // version "1" versus paper-space flag 1) is not property preservation.
    let name = format!("{element:?}").to_ascii_lowercase();
    let schema = batch.schema();
    schema
        .metadata()
        .iter()
        .chain(schema.fields().iter().flat_map(|f| f.metadata().iter()))
        .any(|(k, v)| k.to_ascii_lowercase().ends_with(&format!(".{name}")) && v == payload)
        || schema.fields().iter().zip(batch.columns()).any(|(f, a)| {
            f.name().eq_ignore_ascii_case(&name)
                && a.as_any()
                    .downcast_ref::<StringArray>()
                    .is_some_and(|a| a.iter().flatten().any(|v| v == payload))
        })
}

#[allow(clippy::too_many_lines)] // Execute every witness and keep the complete failure evidence.
pub(super) fn exercise(
    driver: &str,
    native: NativeDriver,
    element: E,
    root: &Path,
    row: &mut Value,
) -> bool {
    if driver == "geoparquet" && matches!(element, E::Bbox | E::Covering | E::FeatureId) {
        native_parquet::exercise(native, element, root, row);
        return true;
    }
    if !supports(driver, element) {
        return false;
    }
    if driver == "filegdb" && std::env::var_os("PLENORA_ENUM_PYTHON").is_none() {
        return false;
    }
    let mut failures = Vec::new();
    let variants =
        if driver == "filegdb" && contract_key(element).is_none() && element != E::Aliases {
            let e = if element == E::FieldOrder {
                E::Utf8
            } else {
                element
            };
            generate(e).unwrap()
        } else {
            vec![base(), base()]
        };
    for (variant, source) in variants.iter().enumerate() {
        let dir = tempfile::tempdir_in(root).unwrap();
        let mut input = dir.path().join(format!("native.{}", extension(driver)));
        let output = dir.path().join("result.arrow");
        let mut witness = match driver {
            "dxf" => dxf_witness(&input, element, variant != 0),
            "gpkg" => gpkg_witness(&input, element, variant != 0),
            "shp" => {
                let witness = shp_witness(&input, element);
                input = input.join("features.shp");
                witness
            }
            "filegdb" => {
                let seed = dir.path().join("seed.gpkg");
                let mut batch = super::gpkg::write(&seed, source, E::GeometryType).unwrap();
                let native_wkb = dir.path().join("native.wkb");
                let result = Command::new(std::env::var("PLENORA_ENUM_PYTHON").unwrap())
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/loss_enumeration/group_b_filegdb.py"),
                    )
                    .arg(&seed)
                    .arg(&input)
                    .arg(&native_wkb)
                    .arg(if element == E::Aliases && variant != 0 {
                        "Aliases"
                    } else {
                        ""
                    })
                    .output()
                    .unwrap();
                if !result.status.success() {
                    failures.push(json!({"variant":variant,"generator_error":String::from_utf8_lossy(&result.stderr),"fixture":dir.keep()}));
                    continue;
                }
                if native_wkb.exists() {
                    let bytes = std::fs::read(&native_wkb).unwrap();
                    let mut arrays = batch.columns().to_vec();
                    arrays[0] = Arc::new(BinaryArray::from(vec![bytes.as_slice()]));
                    batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
                }
                Witness {
                    batch: batch
                        .project(&(0..batch.num_columns() - 1).collect::<Vec<_>>())
                        .unwrap(),
                    properties: if element == E::Aliases && variant != 0 {
                        vec![(E::Aliases, "independent native alias".to_owned())]
                    } else {
                        vec![]
                    },
                }
            }
            _ => unreachable!(),
        };
        if let Some((key, value, schema)) = contract_key(element) {
            if element == E::PrimaryGeometry {
                witness.batch = metadata(&witness.batch, "plenora.field_id", "0", false);
            }
            let definition;
            let value = if driver == "filegdb" && element == E::CrsDefinition {
                definition = std::fs::read_to_string(dir.path().join("native.wkb.wkt")).unwrap();
                definition.as_str()
            } else {
                value
            };
            witness.batch = metadata(&witness.batch, key, value, schema);
        }
        let (ok, report) = run(&input, &output, driver, "ipc");
        if !ok {
            failures
                .push(json!({"variant":variant,"native_read_failed":report,"fixture":dir.keep()}));
            continue;
        }
        let after = native_batch(&input, &output, driver);
        let differences = compare(&witness.batch, &after);
        let mut observed = BTreeMap::<String, u64>::new();
        for difference in &differences {
            *observed
                .entry(category(native, D::Read, difference))
                .or_default() += 1;
        }
        for (property, payload) in &witness.properties {
            if driver == "dxf" && *property == E::GeometryType && payload == "TEXT" {
                let array = after
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                if matches!(
                    decode_wkb(array.value(0), &WkbLimits::default())
                        .unwrap()
                        .value,
                    WkbValue::Point(_)
                ) {
                    *observed
                        .entry("TEXT rappresentato come punto".to_owned())
                        .or_default() += 1;
                }
            } else if !payload_present(&after, *property, payload) {
                let NativeRule::Transformed(change) = native_rule(native, D::Read, *property)
                else {
                    panic!("native property must have a table transformation")
                };
                *observed.entry(change.category().to_owned()).or_default() += 1;
            }
        }
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"schema":format!("{:?}",after.schema()),"fixture":dir.keep()}));
        }
    }
    row["status"] = json!(if failures.is_empty() {
        "passed"
    } else {
        "failed"
    });
    row["variants_exercised"] = json!(variants.len());
    row["witness"] = json!("native-only element factory; direct format construction; independent values and native properties; exact report counts");
    row["failures"] = json!(failures);
    true
}
