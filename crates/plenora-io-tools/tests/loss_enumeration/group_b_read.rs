//! Native-only factories. These do not add shared Arrow element generators.
use super::*;
#[path = "group_b_read_dxf.rs"]
mod native_dxf;
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

#[allow(clippy::too_many_lines)] // Native grammar dispatch is indexed by element, not registered cells.
fn supports(driver: &str, element: E) -> bool {
    if matches!(driver, "shp" | "dxf" | "filegdb" | "gpkg") && contract_key(element).is_some() {
        return true;
    }
    match driver {
        "dxf" => matches!(
            element,
            E::Boolean
                | E::RingOrientation
                | E::FieldNames
                | E::FieldOrder
                | E::ExternalReference
                | E::SignedInteger
                | E::Floating
                | E::LayerMetadata
                | E::Extensions
                | E::Hierarchy
                | E::NativeMetadata
                | E::GeometryType
                | E::MissingValue
                | E::Color
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
            E::Boolean
                | E::Temporal
                | E::SchemaMetadata
                | E::FieldMetadata
                | E::NativeMetadata
                | E::Srid
                | E::ExtensionMetadata
                | E::UnknownDimensions
                | E::Identifier
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
            E::UnknownDimensions
                | E::FeatureId
                | E::NativeMetadata
                | E::Bbox
                | E::Inference
                | E::DbfDate
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
            E::FeatureId
                | E::EmptyPoint
                | E::EmptyGeometry
                | E::Subtypes
                | E::Domains
                | E::LayerMetadata
                | E::Hierarchy
                | E::Extensions
                | E::Bbox
                | E::Temporal
                | E::NativeMetadata
                | E::SignedZero
                | E::NotNull
                | E::SignedInteger
                | E::Aliases
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

#[allow(clippy::too_many_lines)] // SQL grammar and expected native schema belong to the same fixture.
fn gpkg_witness(path: &Path, element: E, changed: bool) -> Witness {
    let source = match element {
        E::Boolean => attribute(Arc::new(Int64Array::from(vec![i64::from(changed)])), true),
        E::Temporal => attribute(Arc::new(StringArray::from(vec!["1970-01-02"])), true),
        E::FieldMetadata => attribute(Arc::new(Int64Array::from(vec![7])), true),
        _ => base(),
    };
    let mut batch = super::gpkg::write(path, &source, element).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    if matches!(element, E::DefinitionFormat | E::CrsDefinition) {
        conn.execute(
            "UPDATE gpkg_spatial_ref_sys SET definition=?1 WHERE srs_id=4326",
            [WKT],
        )
        .unwrap();
    }
    if matches!(element, E::Boolean | E::Temporal) {
        // Rebuild the native SQL declaration while keeping its actual value.
        conn.execute_batch("ALTER TABLE features RENAME TO old_features")
            .unwrap();
        let kind = if element == E::Boolean {
            "BOOLEAN"
        } else {
            "DATE"
        };
        conn.execute_batch(&format!("CREATE TABLE features(geometry BLOB,value {kind},fid INTEGER PRIMARY KEY); INSERT INTO features SELECT * FROM old_features; DROP TABLE old_features")).unwrap();
    }
    if element == E::FieldMetadata {
        let mut fields: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        fields[1] = fields[1]
            .clone()
            .with_metadata([("gpkg.declared_type".to_owned(), "INTEGER".to_owned())]);
        batch =
            RecordBatch::try_new(Arc::new(Schema::new(fields)), batch.columns().to_vec()).unwrap();
    }
    if element == E::NativeMetadata {
        batch = metadata(
            &batch,
            "plenora.geometry.native.gpkg.geometry_type_name",
            "POINT",
            false,
        );
        batch = metadata(&batch, "plenora.geometry.native.gpkg.z", "0", false);
    }
    if element == E::Srid {
        batch = metadata(&batch, "plenora.geometry.srid", "4326", false);
    }
    if element == E::UnknownDimensions {
        conn.execute_batch("UPDATE gpkg_geometry_columns SET z=2,m=2")
            .unwrap();
        batch = metadata(&batch, "plenora.geometry.dimensions", "unknown", false);
    }
    let mut properties = Vec::new();
    let sql = match element {
        E::DataColumns => "CREATE TABLE gpkg_data_columns(table_name TEXT,column_name TEXT,name TEXT,title TEXT,description TEXT,mime_type TEXT,constraint_name TEXT); INSERT INTO gpkg_data_columns VALUES('features','geometry','native-name','native-title','native-description',NULL,NULL)",
        E::Styles => "CREATE TABLE layer_styles(f_table_name TEXT,styleQML TEXT); INSERT INTO layer_styles VALUES('features','native-style-payload')",
        E::Extensions => "CREATE TABLE gpkg_extensions(table_name TEXT,column_name TEXT,extension_name TEXT,definition TEXT,scope TEXT); INSERT INTO gpkg_extensions VALUES('features',NULL,'test_native','https://example.test/native','read-write')",
        E::LayerMetadata | E::ExtensionMetadata => "CREATE TABLE gpkg_metadata(id INTEGER PRIMARY KEY,md_scope TEXT,md_standard_uri TEXT,mime_type TEXT,metadata TEXT); INSERT INTO gpkg_metadata VALUES(1,'dataset','https://example.test/native','text/plain','native-layer-payload'); CREATE TABLE gpkg_metadata_reference(reference_scope TEXT,table_name TEXT,column_name TEXT,row_id_value INTEGER,timestamp TEXT,md_file_id INTEGER,md_parent_id INTEGER); INSERT INTO gpkg_metadata_reference VALUES('table','features',NULL,NULL,'2026-01-01T00:00:00Z',1,NULL)",
        E::SqlConstraints => "CREATE UNIQUE INDEX native_unique_geometry ON features(geometry)",
        _ => "",
    };
    if changed && !sql.is_empty() {
        conn.execute_batch(sql).unwrap();
        let payload = match element {
            E::DataColumns => "native-title",
            E::Styles => "native-style-payload",
            E::Extensions => "test_native",
            E::LayerMetadata | E::ExtensionMetadata => "native-layer-payload",
            E::SqlConstraints => "CREATE UNIQUE INDEX native_unique_geometry ON features(geometry)",
            _ => unreachable!(),
        };
        properties.push((
            if element == E::ExtensionMetadata {
                E::LayerMetadata
            } else {
                element
            },
            payload.to_owned(),
        ));
    }
    if matches!(element, E::Identifier | E::Description | E::SchemaMetadata) {
        let column = if element == E::Description {
            "description"
        } else {
            "identifier"
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
        E::SignedInteger | E::Inference => {
            attribute(Arc::new(StringArray::from(vec!["-2147483648"])), true)
        }
        E::FieldNames => attribute(
            Arc::new(StringArray::from(vec!["native field value"])),
            true,
        ),
        E::FieldOrder => attribute(Arc::new(StringArray::from(vec!["first"])), true),
        E::UnknownDimensions => generate(E::NullGeometry).unwrap().remove(0),
        E::EmptyPoint => geometry_batch(
            WkbValue::Point(coordinate(f64::NAN, f64::NAN)),
            GeometryType::Point,
        ),
        _ => base(),
    };
    let mut batch = super::shp::write(path, &source);
    if matches!(
        element,
        E::DbfDate | E::Temporal | E::SignedInteger | E::Inference
    ) {
        let file = path.join("features.dbf");
        let mut bytes = std::fs::read(&file).unwrap();
        let date = matches!(element, E::DbfDate | E::Temporal);
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
    if element == E::NativeMetadata {
        batch = metadata(
            &batch,
            "plenora.geometry.native.shp.shape_type",
            "point-xy",
            false,
        );
    }
    if element == E::UnknownDimensions {
        for extension in ["shp", "shx"] {
            let file = path.join(format!("features.{extension}"));
            let mut bytes = std::fs::read(&file).unwrap();
            bytes[32..36].copy_from_slice(&0_i32.to_le_bytes());
            std::fs::write(file, bytes).unwrap();
        }
        batch = metadata(&batch, "plenora.geometry.dimensions", "unknown", false);
    }
    if element == E::FeatureId {
        // SHP record numbers are one-based physical ordinals, not arbitrary
        // feature keys. The single delivered row reconstructs record 1.
        let bytes = std::fs::read(path.join("features.shp")).unwrap();
        assert_eq!(i32::from_be_bytes(bytes[100..104].try_into().unwrap()), 1);
        assert_eq!(batch.num_rows(), 1);
    }
    if element == E::Bbox {
        // Header envelope is exactly the independent point envelope. Compare
        // its four coordinates with the source before checking delivered WKB.
        let bytes = std::fs::read(path.join("features.shp")).unwrap();
        let bounds: Vec<_> = bytes[36..68]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b))
            .collect();
        assert_eq!(bounds, [1.0, 2.0, 1.0, 2.0]);
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

fn shp_empty(root: &Path, row: &mut Value) {
    let dir = tempfile::tempdir_in(root).unwrap();
    let input = dir.path().join("empty.shp");
    let source = dir.path().join("native");
    super::shp::write(&source, &base());
    let mut bytes = std::fs::read(source.join("features.shp")).unwrap();
    bytes.truncate(100);
    bytes[24..28].copy_from_slice(&78_i32.to_be_bytes());
    bytes[32..36].copy_from_slice(&5_i32.to_le_bytes());
    bytes.extend(1_i32.to_be_bytes());
    bytes.extend(24_i32.to_be_bytes());
    bytes.extend(5_i32.to_le_bytes());
    bytes.extend([0_u8; 40]); // bbox, numParts=0, numPoints=0
                              // Polygon payload is 44 bytes (22 words).
    bytes[24..28].copy_from_slice(&76_i32.to_be_bytes());
    bytes[104..108].copy_from_slice(&22_i32.to_be_bytes());
    std::fs::write(&input, &bytes).unwrap();
    let mut index = bytes[..100].to_vec();
    index[24..28].copy_from_slice(&54_i32.to_be_bytes());
    index.extend(50_i32.to_be_bytes());
    index.extend(22_i32.to_be_bytes());
    std::fs::write(input.with_extension("shx"), index).unwrap();
    for extension in ["dbf", "prj"] {
        std::fs::copy(
            source.join(format!("features.{extension}")),
            input.with_extension(extension),
        )
        .unwrap();
    }
    let output = dir.path().join("out.arrow");
    let (ok, report) = run(&input, &output, "shp", "ipc");
    let rule = plenora_io_model::error_table::Situation::InvalidContent.rule();
    let passed = !ok
        && report["error"]["category"] == json!(rule.category)
        && report["error"]["code"] == rule.wire_code
        && !output.exists();
    row["status"] = json!(if passed { "passed" } else { "failed" });
    row["variants_exercised"] = json!(1);
    row["witness"] =
        json!("native polygon record with zero parts/points; typed rejection and no output");
    row["failures"] = if passed { json!([]) } else { json!([report]) };
}

#[allow(clippy::too_many_lines)] // Execute every witness and keep the complete failure evidence.
pub(super) fn exercise(
    driver: &str,
    native: NativeDriver,
    element: E,
    root: &Path,
    row: &mut Value,
) -> bool {
    if driver == "shp" && element == E::EmptyGeometry {
        shp_empty(root, row);
        return true;
    }
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
            if matches!(element, E::Subtypes | E::Domains) {
                vec![attribute(Arc::new(Int64Array::from(vec![1])), true)]
            } else if matches!(
                element,
                E::LayerMetadata | E::Hierarchy | E::Extensions | E::Bbox | E::FeatureId
            ) {
                vec![base(), base()]
            } else if element == E::Temporal {
                vec![attribute(
                    Arc::new(StringArray::from(vec!["1970-01-02"])),
                    true,
                )]
            } else if element == E::SignedInteger {
                vec![attribute(
                    Arc::new(Int64Array::from(vec![-2_147_483_648_i64])),
                    true,
                )]
            } else {
                let e = if element == E::FieldOrder {
                    E::Utf8
                } else {
                    element
                };
                generate(e).unwrap()
            }
        } else {
            vec![base(), base()]
        };
    for (variant, source) in variants.iter().enumerate() {
        let dir = tempfile::tempdir_in(root).unwrap();
        let mut input = dir.path().join(format!("native.{}", extension(driver)));
        let output = dir.path().join("result.arrow");
        let mut witness = match driver {
            "dxf" => native_dxf::witness(&input, element, variant != 0),
            "gpkg" => gpkg_witness(&input, element, variant != 0),
            "shp" => {
                let witness = shp_witness(&input, element);
                input = input.join("features.shp");
                witness
            }
            "filegdb" => {
                let seed = dir.path().join("seed.gpkg");
                let mut batch = super::gpkg::write(&seed, source, E::GeometryType).unwrap();
                if element == E::Temporal {
                    let conn = rusqlite::Connection::open(&seed).unwrap();
                    conn.execute_batch("ALTER TABLE features RENAME TO old_features; CREATE TABLE features(geometry BLOB,value DATE,fid INTEGER PRIMARY KEY); INSERT INTO features SELECT * FROM old_features; DROP TABLE old_features").unwrap();
                }
                let native_wkb = dir.path().join("native.wkb");
                let result = Command::new(std::env::var("PLENORA_ENUM_PYTHON").unwrap())
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/loss_enumeration/group_b_filegdb.py"),
                    )
                    .arg(&seed)
                    .arg(&input)
                    .arg(&native_wkb)
                    .arg(
                        if element == E::SignedInteger
                            || element == E::Temporal
                            || matches!(element, E::Subtypes | E::Domains)
                            || variant != 0
                        {
                            format!("{element:?}")
                        } else {
                            String::new()
                        },
                    )
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
                if matches!(element, E::EmptyPoint | E::EmptyGeometry) && !native_wkb.exists() {
                    let mut arrays = batch.columns().to_vec();
                    arrays[0] = Arc::new(BinaryArray::from(vec![None::<&[u8]>]));
                    batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
                }
                if matches!(element, E::Subtypes | E::Domains) {
                    let mut fields: Vec<_> = batch
                        .schema()
                        .fields()
                        .iter()
                        .map(|f| f.as_ref().clone())
                        .collect();
                    let mut arrays = batch.columns().to_vec();
                    arrays[1] = Arc::new(arrow_array::Int32Array::from(vec![1]));
                    fields[1] = Field::new("value", DataType::Int32, true);
                    batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap();
                }
                if element == E::Temporal {
                    let mut fields: Vec<_> = batch
                        .schema()
                        .fields()
                        .iter()
                        .map(|f| f.as_ref().clone())
                        .collect();
                    let mut arrays = batch.columns().to_vec();
                    arrays[1] = Arc::new(
                        arrow_array::TimestampMicrosecondArray::from(vec![86_400_000_000])
                            .with_timezone("UTC"),
                    );
                    fields[1] = Field::new("value", arrays[1].data_type().clone(), true);
                    batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap();
                }
                Witness {
                    batch: batch
                        .project(&(0..batch.num_columns() - 1).collect::<Vec<_>>())
                        .unwrap(),
                    properties: match element {
                        E::Aliases if variant != 0 => {
                            vec![(E::Aliases, "independent native alias".to_owned())]
                        }
                        E::Subtypes => vec![(element, "Road".to_owned())],
                        E::Domains => vec![(element, "RoadType".to_owned())],
                        E::LayerMetadata if variant != 0 => {
                            vec![(element, "native-documentation".to_owned())]
                        }
                        E::Hierarchy if variant != 0 => vec![(element, "NativeGroup".to_owned())],
                        E::Extensions if variant != 0 => {
                            vec![(element, "native-extension".to_owned())]
                        }
                        _ => vec![],
                    },
                }
            }
            _ => unreachable!(),
        };
        if driver == "filegdb" && element == E::NativeMetadata {
            witness.batch = metadata(
                &witness.batch,
                "plenora.geometry.native.filegdb.ogr_geometry_type",
                "1",
                false,
            );
        }
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
            } else if payload == "two-native-attributes" {
                assert!(!after
                    .schema()
                    .fields()
                    .iter()
                    .any(|f| matches!(f.name().as_str(), "alpha" | "zeta")));
                observed.insert(
                    "valori attributi INSERT non rappresentati come colonne".to_owned(),
                    2,
                );
                observed.insert("blocco INSERT esploso".to_owned(), 1);
            } else if !payload_present(&after, *property, payload) {
                let NativeRule::Transformed(change) = native_rule(native, D::Read, *property)
                else {
                    panic!("native property must have a table transformation")
                };
                *observed.entry(change.category().to_owned()).or_default() += 1;
            }
        }
        if driver == "filegdb" {
            observed.insert(
                "native_feature_id_not_preserved".to_owned(),
                u64::try_from(witness.batch.num_rows()).unwrap(),
            );
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
