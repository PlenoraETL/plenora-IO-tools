//! The table owns the product; fixtures never register cells themselves.
//! A missing native witness is a failure, not a successful capability check.
#[path = "geometry.rs"]
mod geometry;
use super::{coordinate, native_batch, save};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array,
    Float64Array, Int64Array, Int8Array, RecordBatch, StringArray, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_core::loss_table::{
    native_rule, NativeDriver, NativeElement as E, NativeRule, TransportDirection as D, DRIVERS,
    NATIVE_ELEMENTS,
};
use plenora_io_model::{
    contract::GeometryType,
    limits::WkbLimits,
    wkb::{decode_wkb, encode_wkb, WkbFlavor, WkbGeometry, WkbValue},
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path, process::Command, sync::Arc};

fn geometry_batch(value: WkbValue, kind: GeometryType) -> RecordBatch {
    super::geometry_batch(value, kind)
}

fn attribute(array: ArrayRef, nullable: bool) -> RecordBatch {
    let base = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(
            vec![
                base.schema().field(0).clone(),
                Field::new("value", array.data_type().clone(), nullable)
                    .with_metadata([("plenora.field_id".to_owned(), "1".to_owned())]),
            ],
            base.schema().metadata().clone(),
        )),
        vec![base.column(0).clone(), array],
    )
    .unwrap()
}

fn metadata(batch: &RecordBatch, key: &str, value: &str, schema: bool) -> RecordBatch {
    let mut fields: Vec<_> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect();
    let mut root = batch.schema().metadata().clone();
    if schema {
        root.insert(key.to_owned(), value.to_owned());
    } else {
        let mut values = fields[0].metadata().clone();
        values.insert(key.to_owned(), value.to_owned());
        fields[0] = fields[0].clone().with_metadata(values);
    }
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(fields, root)),
        batch.columns().to_vec(),
    )
    .unwrap()
}

/// Factories are indexed by element, never by driver/direction/cell.
/// Native-only objects must get a native adapter; a made-up Arrow key would
/// exercise metadata removal rather than (for example) XREF resolution.
#[allow(clippy::too_many_lines)] // One factory dispatch for table elements, not cells.
fn generate(element: E) -> Result<Vec<RecordBatch>, &'static str> {
    let base = || geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    let one = |array: ArrayRef| vec![attribute(array, true)];
    Ok(match element {
        E::Utf8 => one(Arc::new(StringArray::from(vec!["è漢字"]))),
        E::Null => one(Arc::new(StringArray::from(vec![None::<&str>]))),
        E::EmptyString => one(Arc::new(StringArray::from(vec![""]))),
        E::Boolean => [false, true]
            .into_iter()
            .map(|v| attribute(Arc::new(BooleanArray::from(vec![v])), true))
            .collect(),
        E::SignedInteger => vec![
            attribute(Arc::new(Int8Array::from(vec![i8::MIN])), true),
            attribute(Arc::new(Int64Array::from(vec![i64::MAX])), true),
        ],
        E::UnsignedInteger => vec![
            attribute(Arc::new(UInt8Array::from(vec![u8::MAX])), true),
            attribute(Arc::new(UInt64Array::from(vec![u64::MAX])), true),
        ],
        E::Floating => vec![
            attribute(Arc::new(Float32Array::from(vec![1.25])), true),
            attribute(Arc::new(Float64Array::from(vec![1.25])), true),
        ],
        E::Binary => one(Arc::new(BinaryArray::from(vec![b"\x00\xff".as_slice()]))),
        E::Temporal => one(Arc::new(Date32Array::from(vec![1]))),
        E::Decimal => one(Arc::new(
            Decimal128Array::from(vec![123])
                .with_precision_and_scale(10, 2)
                .unwrap(),
        )),
        E::NotNull => vec![attribute(Arc::new(StringArray::from(vec!["value"])), false)],
        E::FieldOrder => {
            let b = attribute(Arc::new(StringArray::from(vec!["value"])), true);
            vec![b.project(&[1, 0]).unwrap()]
        }
        E::FieldNames => {
            let b = attribute(Arc::new(StringArray::from(vec!["value"])), true);
            let fields = vec![
                b.schema().field(0).clone(),
                b.schema().field(1).clone().with_name("long_field_name_é"),
            ];
            vec![RecordBatch::try_new(
                Arc::new(Schema::new_with_metadata(
                    fields,
                    b.schema().metadata().clone(),
                )),
                b.columns().to_vec(),
            )
            .unwrap()]
        }
        E::SchemaMetadata => vec![metadata(&base(), "qualification.private", "value", true)],
        E::FieldMetadata => vec![metadata(&base(), "qualification.private", "value", false)],
        E::NativeMetadata => vec![metadata(
            &base(),
            "plenora.geometry.native.qualification",
            "value",
            false,
        )],
        E::FieldIdentity => vec![metadata(&base(), "plenora.field_id", "42", false)],
        E::ContractVersion
        | E::GeometryEncoding
        | E::Precision
        | E::TypesDeclaration
        | E::CrsId
        | E::CrsResolution
        | E::AxisOrder
        | E::ExtensionName => vec![base()],
        E::ExtensionMetadata => vec![metadata(&base(), "ARROW:extension:metadata", "{}", false)],
        E::NullGeometry => vec![RecordBatch::try_new(
            base().schema(),
            vec![Arc::new(BinaryArray::from(vec![None::<&[u8]>]))],
        )
        .unwrap()],
        E::EmptyPoint => vec![geometry_batch(
            WkbValue::Point(coordinate(f64::NAN, f64::NAN)),
            GeometryType::Point,
        )],
        E::EmptyGeometry => vec![geometry_batch(
            WkbValue::MultiPolygon(vec![]),
            GeometryType::MultiPolygon,
        )],
        E::SignedZero => vec![geometry_batch(
            WkbValue::Point(coordinate(-0.0, 2.0)),
            GeometryType::Point,
        )],
        E::CoordinatePrecision => vec![geometry_batch(
            WkbValue::Point(coordinate(1.123_456_789_012_345, 1e-20)),
            GeometryType::Point,
        )],
        E::GeometryType | E::RingOrientation => {
            let ring = vec![
                coordinate(0.0, 0.0),
                coordinate(1.0, 0.0),
                coordinate(1.0, 1.0),
                coordinate(0.0, 0.0),
            ];
            let mut reversed = ring.clone();
            reversed.reverse();
            [ring, reversed]
                .into_iter()
                .map(|r| geometry_batch(WkbValue::Polygon(vec![r]), GeometryType::Polygon))
                .collect()
        }
        E::Z | E::M => {
            let mut c = coordinate(1.0, 2.0);
            let dimension = if element == E::Z {
                c.z = Some(3.0);
                "xyz"
            } else {
                c.m = Some(3.0);
                "xym"
            };
            let mut b = base();
            let g = WkbGeometry {
                value: WkbValue::Point(c),
                dimensions: if element == E::Z {
                    plenora_io_model::contract::CoordinateDimensions::Xyz
                } else {
                    plenora_io_model::contract::CoordinateDimensions::Xym
                },
                srid: None,
            };
            let bytes = encode_wkb(&g, WkbFlavor::Iso).unwrap();
            b = metadata(&b, "plenora.geometry.dimensions", dimension, false);
            vec![RecordBatch::try_new(
                b.schema(),
                vec![Arc::new(BinaryArray::from(vec![bytes.as_slice()]))],
            )
            .unwrap()]
        }
        E::UnknownDimensions => vec![metadata(
            &base(),
            "plenora.geometry.dimensions",
            "unknown",
            false,
        )],
        // These require semantic fixtures/adapters; never silently substitute a
        // point or an opaque string and label the corresponding cell covered.
        _ => return Err("no independent semantic fixture/native adapter"),
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Difference {
    element: E,
    detail: String,
}

fn metadata_element(key: &str, field: bool) -> E {
    match key {
        "plenora.contract.version" => E::ContractVersion,
        "plenora.field_id" => E::FieldIdentity,
        "plenora.geometry.types_declaration" => E::TypesDeclaration,
        "plenora.geometry.types" => E::GeometryType,
        "plenora.geometry.dimensions" => E::Dimensions,
        "plenora.geometry.encoding" => E::GeometryEncoding,
        "plenora.geometry.spatial_semantics" => E::SpatialSemantics,
        "plenora.geometry.precision" => E::Precision,
        "plenora.geometry.srid" => E::Srid,
        "plenora.geometry.crs_id" | "crs" => E::CrsId,
        "plenora.geometry.crs_definition" => E::CrsDefinition,
        "plenora.geometry.crs_resolution" => E::CrsResolution,
        "plenora.geometry.crs_definition_format" => E::DefinitionFormat,
        "plenora.geometry.axis_order" => E::AxisOrder,
        "plenora.primary_geometry" => E::PrimaryGeometry,
        "ARROW:extension:name" => E::ExtensionName,
        "ARROW:extension:metadata" => E::ExtensionMetadata,
        _ if key.starts_with("plenora.geometry.native.") => E::NativeMetadata,
        _ if field => E::FieldMetadata,
        _ => E::SchemaMetadata,
    }
}

fn compare_metadata(
    before: &arrow_schema::Metadata,
    after: &arrow_schema::Metadata,
    field: bool,
    out: &mut Vec<Difference>,
) {
    for (key, value) in before {
        if after.get(key) != Some(value) {
            out.push(Difference {
                element: metadata_element(key, field),
                detail: format!("metadata {key}"),
            });
        }
    }
}

/// Decode before comparing: byte order and ISO/EWKB serialization are not loss.
fn compare_geometry(a: &[u8], b: &[u8], out: &mut Vec<Difference>) {
    let a = decode_wkb(a, &WkbLimits::default()).unwrap();
    let b = decode_wkb(b, &WkbLimits::default()).unwrap();
    let a = geometry::observe(&a);
    let b = geometry::observe(&b);
    for (changed, element, detail) in [
        (a.types != b.types, E::GeometryType, "WKB types"),
        (
            a.dimensions != b.dimensions,
            E::Dimensions,
            "WKB dimensions",
        ),
        (a.srids != b.srids, E::Srid, "WKB SRID"),
        (
            a.coordinates != b.coordinates,
            E::CoordinatePrecision,
            "exact coordinates",
        ),
        (
            a.orientations != b.orientations,
            E::RingOrientation,
            "ring orientation",
        ),
    ] {
        if changed {
            out.push(Difference {
                element,
                detail: detail.into(),
            });
        }
    }
}

#[allow(clippy::too_many_lines)] // Keep independent schema/value observations together.
fn compare(before: &RecordBatch, after: &RecordBatch) -> Vec<Difference> {
    let mut out = Vec::new();
    let a = before.schema();
    let b = after.schema();
    compare_metadata(a.metadata(), b.metadata(), false, &mut out);
    if before.num_rows() != after.num_rows() {
        out.push(Difference {
            element: E::MissingValue,
            detail: "row count".into(),
        });
    }
    for (i, field) in a.fields().iter().enumerate() {
        let geometry = plenora_io_model::geometry::is_geometry_field(field);
        let matched = b
            .fields()
            .iter()
            .position(|f| f.name() == field.name())
            .or_else(|| {
                geometry
                    .then(|| {
                        b.fields()
                            .iter()
                            .position(|f| plenora_io_model::geometry::is_geometry_field(f))
                    })
                    .flatten()
            });
        let Some(j) = matched else {
            out.push(Difference {
                element: E::FieldNames,
                detail: format!("missing field {i}"),
            });
            continue;
        };
        let target = b.field(j);
        for (changed, element, detail) in [
            (i != j, E::FieldOrder, "field order"),
            (field.name() != target.name(), E::FieldNames, "field name"),
            (
                field.is_nullable() != target.is_nullable(),
                E::NotNull,
                "nullability",
            ),
        ] {
            if changed {
                out.push(Difference {
                    element,
                    detail: detail.into(),
                });
            }
        }
        compare_metadata(field.metadata(), target.metadata(), true, &mut out);
        if field.data_type() != target.data_type() {
            let element = match field.data_type() {
                DataType::Boolean => E::Boolean,
                DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
                    E::SignedInteger
                }
                DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
                    E::UnsignedInteger
                }
                DataType::Float32 | DataType::Float64 => E::Floating,
                DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => E::Temporal,
                _ => E::StructuredValue,
            };
            out.push(Difference {
                element,
                detail: format!("type {:?} -> {:?}", field.data_type(), target.data_type()),
            });
        }
        if geometry {
            let source = before
                .column(i)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            let target = after
                .column(j)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            for row in 0..source.len().min(target.len()) {
                if source.is_null(row) != target.is_null(row) {
                    out.push(Difference {
                        element: E::MissingValue,
                        detail: "geometry null".into(),
                    });
                } else if !source.is_null(row) {
                    compare_geometry(source.value(row), target.value(row), &mut out);
                }
            }
        } else if field.data_type() == target.data_type() && !before.column(i).eq(after.column(j)) {
            out.push(Difference {
                element: E::MissingValue,
                detail: format!("attribute values {i}"),
            });
        }
    }
    if b.fields().len() > a.fields().len() {
        out.push(Difference {
            element: E::FieldNames,
            detail: "additional fields".into(),
        });
    }
    out
}

/// The only difference -> category mapping. A preserved/rejected rule cannot
/// hide a measured difference: it produces an explicit unmapped defect.
fn category(driver: NativeDriver, direction: D, difference: &Difference) -> String {
    match native_rule(driver, direction, difference.element) {
        NativeRule::Transformed(change) => change.category().to_owned(),
        rule => format!("UNMAPPED:{:?}:{rule:?}", difference.element),
    }
}

fn run(source: &Path, target: &Path, from: &str, to: &str) -> (bool, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plenora-io"));
    command
        .arg("convert")
        .arg(source)
        .arg(target)
        .args(["--from", from, "--to", to, "--layer", "0"]);
    if matches!(from, "csv" | "xls") {
        command.args([
            "--assume-crs",
            "OGC:CRS84",
            "--in-opt",
            "wkt_column=geometry",
        ]);
    }
    let result = command.output().unwrap();
    (
        result.status.success(),
        serde_json::from_slice(&result.stdout)
            .unwrap_or_else(|_| json!({"stderr":String::from_utf8_lossy(&result.stderr)})),
    )
}

fn report_categories(report: &Value, section: &str) -> BTreeSet<String> {
    report["result"][section]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["conteggio"].as_u64().unwrap() > 0)
        .map(|r| r["categoria"].as_str().unwrap().to_owned())
        .collect()
}

fn extension(driver: &str) -> &str {
    match driver {
        "shp" => "shp.d",
        "filegdb" => "gdb",
        "xls" => "xlsx",
        "geoparquet" => "parquet",
        "ipc" => "arrow",
        other => other,
    }
}

fn without_external_crs(batch: RecordBatch, driver: &str) -> RecordBatch {
    if !matches!(driver, "csv" | "xls") {
        return batch;
    }
    let fields = batch
        .schema()
        .fields()
        .iter()
        .map(|field| {
            if !plenora_io_model::geometry::is_geometry_field(field) {
                return field.as_ref().clone();
            }
            let mut metadata = field.metadata().clone();
            for key in [
                "crs",
                "plenora.geometry.crs_id",
                "plenora.geometry.crs_definition",
                "plenora.geometry.crs_definition_format",
                "plenora.geometry.srid",
            ] {
                metadata.remove(key);
            }
            metadata.insert(
                "plenora.geometry.crs_resolution".to_owned(),
                "missing".to_owned(),
            );
            metadata.insert(
                "plenora.geometry.axis_order".to_owned(),
                "unknown".to_owned(),
            );
            field.as_ref().clone().with_metadata(metadata)
        })
        .collect::<Vec<_>>();
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(
            fields,
            batch.schema().metadata().clone(),
        )),
        batch.columns().to_vec(),
    )
    .unwrap()
}

#[test]
#[allow(clippy::too_many_lines)] // Collect all failures before enforcing the product.
fn generated_product_requires_operational_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/s1e-generated");
    std::fs::create_dir_all(&root).unwrap();
    let mut rows = Vec::new();
    for driver in DRIVERS {
        let native = driver.qualified_native.unwrap();
        for element in NATIVE_ELEMENTS {
            for direction in [D::Read, D::Write] {
                let expected = native_rule(native, direction, *element);
                let mut row = json!({"driver":driver.driver,"direction":format!("{direction:?}"),"element":format!("{element:?}"),"expected":format!("{expected:?}")});
                if driver.driver == "filegdb" && !cfg!(feature = "gdal-backend") {
                    row["status"] = json!("not-executed");
                    row["reason"] = json!("gdal-backend unavailable");
                } else if direction == D::Read && driver.driver != "ipc" {
                    row["status"] = json!("missing-native-witness");
                } else {
                    match generate(*element) {
                        Err(reason) => {
                            row["status"] = json!("missing-generator");
                            row["reason"] = json!(reason);
                        }
                        Ok(fixtures) => {
                            let mut failures = Vec::new();
                            for (variant, before) in fixtures.iter().enumerate() {
                                // Native setup requirement, not the property under
                                // test. Fixed-CRS writers require CRS84; GPKG needs
                                // an integer authority identifier.
                                let before = if driver.driver == "gpkg" && *element != E::CrsId {
                                    metadata(before, "plenora.geometry.crs_id", "EPSG:4326", false)
                                } else {
                                    before.clone()
                                };
                                let dir = tempfile::tempdir_in(&root).unwrap();
                                let seed = dir.path().join("source.arrow");
                                let output = dir
                                    .path()
                                    .join(format!("native.{}", extension(driver.driver)));
                                let back = dir.path().join("back.arrow");
                                save(&seed, &before);
                                let (ok, written) = run(&seed, &output, "ipc", driver.driver);
                                if !ok {
                                    // A capability rejection must be the error-table rule,
                                    // not an arbitrary failure in the setup or parser.
                                    let rejection = plenora_io_model::error_table::Situation::UnrepresentableWriteType.rule();
                                    if expected != NativeRule::Rejected
                                        || written["error"]["category"] != json!(rejection.category)
                                        || written["error"]["code"] != rejection.wire_code
                                        || written["error"]["details"]["capability_reason"]
                                            == "reprojection_required"
                                        || output.exists()
                                    {
                                        failures.push(json!({"variant":variant,"unexpected_rejection":written}));
                                    }
                                    continue;
                                }
                                if expected == NativeRule::Rejected {
                                    failures.push(
                                        json!({"variant":variant,"unexpected_acceptance":true}),
                                    );
                                    continue;
                                }
                                let (ok, read) = run(&output, &back, driver.driver, "ipc");
                                if !ok {
                                    failures.push(json!({"variant":variant,"read_failed":read}));
                                    continue;
                                }
                                let after = without_external_crs(
                                    native_batch(&output, &back, driver.driver),
                                    driver.driver,
                                );
                                let differences = compare(&before, &after);
                                let observed: BTreeSet<_> = differences
                                    .iter()
                                    .map(|d| category(native, direction, d))
                                    .collect();
                                let declared = report_categories(
                                    if direction == D::Read {
                                        &read
                                    } else {
                                        &written
                                    },
                                    if direction == D::Read {
                                        "read_loss"
                                    } else {
                                        "write_loss"
                                    },
                                );
                                if observed != declared {
                                    failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}")}));
                                }
                            }
                            row["status"] = json!(if failures.is_empty() {
                                "passed"
                            } else {
                                "failed"
                            });
                            row["failures"] = json!(failures);
                        }
                    }
                }
                rows.push(row);
            }
        }
    }
    std::fs::write(
        root.join("coverage.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    let failed = rows.iter().filter(|r| r["status"] != "passed").count();
    assert_eq!(
        failed,
        0,
        "{failed}/{} cells without successful evidence; target/s1e-generated/coverage.json",
        rows.len()
    );
}

#[test]
fn comparator_detects_changes_without_consulting_reports() {
    let before = generate(E::FieldMetadata).unwrap().remove(0);
    assert!(compare(&before, &before).is_empty());
    let after = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    assert!(compare(&before, &after)
        .iter()
        .any(|d| d.element == E::FieldMetadata));
    let before = generate(E::SignedZero).unwrap().remove(0);
    let after = geometry_batch(WkbValue::Point(coordinate(0.0, 2.0)), GeometryType::Point);
    assert!(compare(&before, &after)
        .iter()
        .any(|d| d.element == E::CoordinatePrecision));
}
