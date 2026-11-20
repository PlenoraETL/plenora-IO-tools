//! The table owns the product; fixtures never register cells themselves.
//! A missing native witness is a failure, not a successful capability check.
#[path = "constraints.rs"]
mod constraints;
#[path = "geometry.rs"]
mod geometry;
#[path = "group_a.rs"]
mod group_a;
#[path = "group_b.rs"]
mod group_b;
#[path = "native.rs"]
mod native;
#[path = "transport.rs"]
mod transport;
#[path = "values.rs"]
mod values;
use super::{coordinate, native_batch, save};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array,
    Float64Array, Int64Array, Int8Array, RecordBatch, StringArray, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_core::loss_table::{
    native_rule, native_write_transport_rule, read_applicability, NativeDriver, NativeElement as E,
    NativeRule, ReadApplicability, TransportDirection as D, DRIVERS, NATIVE_ELEMENTS,
};
use plenora_io_model::{
    contract::GeometryType,
    limits::WkbLimits,
    wkb::{decode_wkb, encode_wkb, WkbFlavor, WkbGeometry, WkbValue},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
    sync::Arc,
};

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
        let index = fields
            .iter()
            .position(plenora_io_model::geometry::is_geometry_field)
            .unwrap();
        let mut values = fields[index].metadata().clone();
        values.insert(key.to_owned(), value.to_owned());
        fields[index] = fields[index].clone().with_metadata(values);
    }
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(fields, root)),
        batch.columns().to_vec(),
    )
    .unwrap()
}

/// Factories are indexed by element, never by driver/direction/cell.
/// Read needs native grammar witnesses; Write exercises the Arrow transport
/// envelope, never native object resolution in the destination format.
#[allow(clippy::too_many_lines)] // One factory dispatch for table elements, not cells.
fn generate(element: E) -> Result<Vec<RecordBatch>, &'static str> {
    let base = || geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    let one = |array: ArrayRef| vec![attribute(array, true)];
    Ok(match element {
        E::Utf8 => one(Arc::new(StringArray::from(vec!["\u{e8}\u{6f22}\u{5b57}"]))),
        E::Null => one(Arc::new(StringArray::from(vec![None::<&str>]))),
        E::EmptyString => one(Arc::new(StringArray::from(vec![""]))),
        E::Boolean => [false, true]
            .into_iter()
            .map(|v| attribute(Arc::new(BooleanArray::from(vec![v])), true))
            .collect(),
        E::SignedInteger => vec![
            attribute(Arc::new(Int8Array::from(vec![i8::MIN])), true),
            attribute(
                Arc::new(arrow_array::Int32Array::from(vec![i32::MIN])),
                true,
            ),
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
        E::NestedDictionary => vec![
            attribute(
                Arc::new(arrow_array::ListArray::from_iter_primitive::<
                    arrow_array::types::Int32Type,
                    _,
                    _,
                >(vec![Some(vec![Some(1), None, Some(3)])])),
                true,
            ),
            attribute(
                Arc::new(arrow_array::StructArray::from(vec![(
                    Arc::new(Field::new("nested", DataType::Int64, true)),
                    Arc::new(Int64Array::from(vec![Some(7)])) as ArrayRef,
                )])),
                true,
            ),
            attribute(
                Arc::new(
                    arrow_array::DictionaryArray::<arrow_array::types::Int8Type>::try_new(
                        Int8Array::from(vec![0]),
                        Arc::new(StringArray::from(vec!["dictionary value"])),
                    )
                    .unwrap(),
                ),
                true,
            ),
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
                b.schema()
                    .field(1)
                    .clone()
                    .with_name("long_field_name_\u{e9}"),
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
        | E::CrsResolution
        | E::AxisOrder
        | E::ExtensionName => vec![base()],
        E::CrsId => vec![
            base(),
            metadata(&base(), "plenora.geometry.crs_id", "EPSG:4326", false),
        ],
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
                .flat_map(|ring| {
                    let polygon = WkbValue::Polygon(vec![ring]);
                    [
                        geometry_batch(polygon.clone(), GeometryType::Polygon),
                        geometry_batch(
                            WkbValue::MultiPolygon(vec![super::child(polygon)]),
                            GeometryType::MultiPolygon,
                        ),
                    ]
                })
                .collect()
        }
        E::Dimensions => {
            let mut result = generate(E::Z)?;
            result.extend(generate(E::M)?);
            result
        }
        E::SpatialSemantics => vec![metadata(
            &base(),
            "plenora.geometry.spatial_semantics",
            "geometry",
            false,
        )],
        E::NumberPrecision => one(Arc::new(Float64Array::from(vec![1.123_456_789_012_345]))),
        E::PrimaryGeometry => vec![metadata(&base(), "plenora.primary_geometry", "0", true)],
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
        // Foreign native objects use the common Arrow transport factory.
        _ => {
            return transport::generate(element)
                .ok_or("no independent semantic fixture/native adapter")
        }
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
        "plenora.geometry.types"
        | "plenora.geometry.dimensions"
        | "plenora.geometry.crs_id"
        | "crs"
        | "plenora.geometry.crs_definition" => E::FieldMetadata,
        "plenora.geometry.encoding" => E::GeometryEncoding,
        "plenora.geometry.spatial_semantics" => E::SpatialSemantics,
        "plenora.geometry.precision" => E::Precision,
        "plenora.geometry.srid" => E::Srid,
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
            let semantic = match key.as_str() {
                "plenora.geometry.dimensions" if value != "unknown" => Some(E::Dimensions),
                "plenora.geometry.crs_id" => Some(E::CrsId),
                "plenora.geometry.crs_definition" => Some(E::CrsDefinition),
                _ => None,
            };
            if let Some(element) = semantic {
                out.push(Difference {
                    element,
                    detail: format!(
                        "representation {key} {}",
                        if after.contains_key(key) {
                            "derived"
                        } else {
                            "absent"
                        }
                    ),
                });
            }
        }
    }
}

/// Decode before comparing: byte order and ISO/EWKB serialization are not loss.
fn compare_geometry(a: &[u8], b: &[u8], out: &mut Vec<Difference>) {
    let a = decode_wkb(a, &WkbLimits::default()).unwrap();
    let b = decode_wkb(b, &WkbLimits::default()).unwrap();
    let a = geometry::observe(&a);
    let b = geometry::observe(&b);
    let type_detail = format!("WKB types {:?} -> {:?}", a.types, b.types);
    for (changed, element, detail) in [
        (a.types != b.types, E::GeometryType, type_detail.as_str()),
        (
            a.dimensions
                .iter()
                .map(|d| format!("{d:?}"))
                .collect::<BTreeSet<_>>()
                != b.dimensions
                    .iter()
                    .map(|d| format!("{d:?}"))
                    .collect::<BTreeSet<_>>(),
            E::Dimensions,
            "WKB dimensions",
        ),
        (
            a.srids.iter().collect::<BTreeSet<_>>() != b.srids.iter().collect::<BTreeSet<_>>(),
            E::Srid,
            "WKB SRID",
        ),
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

const fn attribute_element(data_type: &DataType) -> E {
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 => E::Utf8,
        DataType::Boolean => E::Boolean,
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => E::SignedInteger,
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            E::UnsignedInteger
        }
        DataType::Float32 | DataType::Float64 => E::Floating,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => E::Temporal,
        DataType::Binary | DataType::LargeBinary => E::Binary,
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => E::Decimal,
        _ => E::NestedDictionary,
    }
}

#[allow(clippy::too_many_lines)] // Keep independent schema/value observations together.
fn compare(before: &RecordBatch, after: &RecordBatch) -> Vec<Difference> {
    let mut out = Vec::new();
    let a = before.schema();
    let b = after.schema();
    let mut root = a.metadata().clone();
    // Explicit selection of the only geometry and implicit unique selection
    // carry the same primary geometry. A multi-geometry selection is retained.
    if !b.metadata().contains_key("plenora.primary_geometry")
        && a.fields()
            .iter()
            .filter(|f| plenora_io_model::geometry::is_geometry_field(f))
            .count()
            == 1
        && b.fields()
            .iter()
            .filter(|f| plenora_io_model::geometry::is_geometry_field(f))
            .count()
            == 1
        && root.get("plenora.primary_geometry").is_some_and(|id| {
            a.fields().iter().any(|f| {
                plenora_io_model::geometry::is_geometry_field(f)
                    && f.metadata().get("plenora.field_id") == Some(id)
            })
        })
    {
        root.remove("plenora.primary_geometry");
    }
    compare_metadata(&root, b.metadata(), false, &mut out);
    if before.num_rows() != after.num_rows() {
        out.push(Difference {
            element: E::MissingValue,
            detail: "row count".into(),
        });
    }
    let mut matched_fields = BTreeSet::new();
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
            compare_metadata(
                field.metadata(),
                &arrow_schema::Metadata::new(),
                true,
                &mut out,
            );
            out.push(Difference {
                element: attribute_element(field.data_type()),
                detail: format!("missing attribute {i}"),
            });
            continue;
        };
        matched_fields.insert(j);
        let target = b.field(j);
        for (changed, element, detail) in [
            (i != j, E::FieldOrder, "field order"),
            (field.name() != target.name(), E::FieldNames, "field name"),
            (
                !field.is_nullable() && target.is_nullable(),
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
                        element: if source.is_null(row) {
                            E::NullGeometry
                        } else {
                            let decoded = decode_wkb(source.value(row), &WkbLimits::default()).unwrap();
                            if matches!(decoded.value, WkbValue::Point(point) if point.x.is_nan() && point.y.is_nan())
                            {
                                E::EmptyPoint
                            } else if geometry::observe(&decoded).coordinates.is_empty() {
                                E::EmptyGeometry
                            } else {
                                E::MissingValue
                            }
                        },
                        detail: "geometry null".into(),
                    });
                } else if !source.is_null(row) {
                    compare_geometry(source.value(row), target.value(row), &mut out);
                }
            }
        } else {
            match values::equal(before.column(i), after.column(j)) {
                Ok(true) => {}
                Ok(false) => out.push(Difference {
                    element: E::MissingValue,
                    detail: format!("attribute values {i}"),
                }),
                Err(reason) => out.push(Difference {
                    element: E::MissingValue,
                    detail: format!("INCOMPLETE: attribute {i}: {reason}"),
                }),
            }
        }
    }
    // A native primary key is generated support, not a lost source field.
    for (index, field) in b.fields().iter().enumerate() {
        if !matched_fields.contains(&index) && !field.metadata().contains_key("gpkg.primary_key") {
            out.push(Difference {
                element: E::FieldNames,
                detail: format!("additional field {index}"),
            });
        }
    }
    out
}

/// The only difference -> category mapping. A preserved/rejected rule cannot
/// hide a measured difference: it produces an explicit unmapped defect.
fn category(driver: NativeDriver, direction: D, difference: &Difference) -> String {
    if difference.detail.starts_with("INCOMPLETE:") {
        return difference.detail.clone();
    }
    if direction == D::Write {
        if driver == NativeDriver::Dxf
            && difference.element == E::GeometryType
            && difference.detail.starts_with("WKB types [MultiPolygon")
        {
            return "MultiPolygon esplosa in entit\u{e0} DXF".to_owned();
        }

        if difference.element == E::Dimensions
            && difference.detail.starts_with("representation")
            && native_rule(driver, direction, E::Dimensions) == NativeRule::Preserved
        {
            return category(
                driver,
                direction,
                &Difference {
                    element: E::FieldMetadata,
                    detail: difference.detail.clone(),
                },
            );
        }
        if difference.element == E::CrsId {
            return "crs_id_not_preserved_absent".to_owned();
        }
        if difference.element == E::CrsDefinition {
            return if difference.detail.ends_with("absent") {
                "crs_definition_not_preserved_absent"
            } else {
                "crs_definition_not_preserved_derived"
            }
            .to_owned();
        }
        if difference.detail.starts_with("missing attribute") && driver == NativeDriver::Kml {
            return "kml_extended_data_not_preserved".to_owned();
        }
    }
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

fn report_categories(report: &Value, section: &str) -> BTreeMap<String, u64> {
    report["result"][section]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["conteggio"].as_u64().unwrap() > 0)
        .map(|r| {
            (
                r["categoria"].as_str().unwrap().to_owned(),
                r["conteggio"].as_u64().unwrap(),
            )
        })
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/s1i-generated");
    let root = if std::env::var_os("PLENORA_LOSS_GROUP_A").is_some() {
        root.with_file_name("s1i-a-generated")
    } else {
        root
    };
    let root = std::env::var_os("PLENORA_ENUM_OUTPUT").map_or(root, std::path::PathBuf::from);
    std::fs::create_dir_all(&root).unwrap();
    let mut rows = Vec::new();
    for driver in DRIVERS {
        if (std::env::var_os("PLENORA_LOSS_GROUP_A").is_some() && !group_a::owns(driver.driver))
            || std::env::var("PLENORA_ENUM_DRIVERS")
                .is_ok_and(|filter| !filter.split(',').any(|name| name == driver.driver))
        {
            continue;
        }
        let native = driver.qualified_native.unwrap();
        for element in NATIVE_ELEMENTS {
            if std::env::var("PLENORA_ENUM_ELEMENTS")
                .is_ok_and(|filter| !filter.split(',').any(|name| name == format!("{element:?}")))
            {
                continue;
            }
            for direction in [D::Read, D::Write] {
                if std::env::var("PLENORA_ENUM_DIRECTION")
                    .is_ok_and(|filter| filter != format!("{direction:?}"))
                {
                    continue;
                }
                let expected = if direction == D::Write {
                    native_write_transport_rule(native, *element)
                } else {
                    native_rule(native, direction, *element)
                };
                let mut row = json!({"driver":driver.driver,"direction":format!("{direction:?}"),"element":format!("{element:?}"),"expected":format!("{expected:?}")});
                if driver.driver == "filegdb" && !cfg!(feature = "gdal-backend") {
                    row["status"] = json!("not-executed");
                    row["reason"] = json!("gdal-backend unavailable in this build; full evidence requires the feature-on job");
                    rows.push(row);
                    continue;
                }
                if direction == D::Read {
                    if let ReadApplicability::NotApplicable { reason, reference } =
                        read_applicability(native, *element)
                    {
                        assert!(!reason.is_empty() && !reference.is_empty());
                        row["status"] = json!("not-applicable");
                        row["reason"] = json!(reason);
                        row["reference"] = json!(reference);
                        rows.push(row);
                        continue;
                    }
                }
                if group_a::exercise(driver.driver, native, *element, direction, &root, &mut row) {
                    rows.push(row);
                    continue;
                }
                if direction == D::Read && driver.driver != "ipc" {
                    native::exercise(driver.driver, native, *element, &root, &mut row);
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
                                let domain_rejection =
                                    constraints::rejection(driver.driver, &before);
                                if let Some((situation, reason)) = domain_rejection {
                                    row["variant_constraints"][variant.to_string()] = json!({"situation":format!("{situation:?}"),"reason":reason});
                                }
                                let (ok, written) = run(&seed, &output, "ipc", driver.driver);
                                if !ok {
                                    // A capability rejection must be the error-table rule,
                                    // not an arbitrary failure in the setup or parser.
                                    let rejection = if let Some((situation, _)) = domain_rejection {
                                        situation.rule()
                                    } else if matches!(
                                        element,
                                        E::EmptyPoint
                                            | E::EmptyGeometry
                                            | E::NullGeometry
                                            | E::EmptyString
                                    ) {
                                        plenora_io_model::error_table::Situation::UnrepresentableWriteValue.rule()
                                    } else {
                                        plenora_io_model::error_table::Situation::UnrepresentableWriteType.rule()
                                    };
                                    if (expected != NativeRule::Rejected
                                        && domain_rejection.is_none())
                                        || written["error"]["category"] != json!(rejection.category)
                                        || written["error"]["code"] != rejection.wire_code
                                        || (written["error"]["details"]["capability_reason"]
                                            == "reprojection_required"
                                            && domain_rejection.is_none_or(|(_, reason)| {
                                                !reason.starts_with("fixed CRS")
                                            }))
                                        || output.exists()
                                    {
                                        failures.push(json!({"variant":variant,"unexpected_rejection":written}));
                                    }
                                    continue;
                                }
                                if expected == NativeRule::Rejected || domain_rejection.is_some() {
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
                                let after = without_external_crs(super::load(&back), driver.driver);
                                let differences = compare(&before, &after);
                                let mut observed = BTreeMap::<String, u64>::new();
                                for difference in &differences {
                                    // A declaration change is already counted as an Arrow
                                    // metadata change. Only formats with a separate native
                                    // dimension declaration have that additional observation.
                                    if difference.element == E::Dimensions
                                        && difference.detail.starts_with("representation")
                                        && native_rule(native, direction, E::Dimensions)
                                            == NativeRule::Preserved
                                    {
                                        continue;
                                    }
                                    let count = observed
                                        .entry(category(native, direction, difference))
                                        .or_default();
                                    if native == NativeDriver::Gpkg
                                        && difference.element == E::FieldOrder
                                    {
                                        *count = 1;
                                    } else {
                                        *count += 1;
                                    }
                                }
                                if direction == D::Write {
                                    for difference in &differences {
                                        let extra = match difference.detail.as_str() {
                                            "metadata plenora.geometry.srid" => {
                                                Some("srid_not_preserved_absent")
                                            }
                                            "metadata plenora.geometry.native.geoparquet.bbox" => {
                                                Some("geometry_bbox_not_preserved")
                                            }
                                            _ => None,
                                        };
                                        if let Some(extra) = extra {
                                            *observed.entry(extra.to_owned()).or_default() += 1;
                                        }
                                    }
                                }
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
    let keys: BTreeSet<_> = rows
        .iter()
        .map(|row| format!("{}:{}:{}", row["driver"], row["direction"], row["element"]))
        .collect();
    assert_eq!(keys.len(), rows.len(), "duplicate matrix cells");
    if [
        "PLENORA_LOSS_GROUP_A",
        "PLENORA_ENUM_DRIVERS",
        "PLENORA_ENUM_ELEMENTS",
        "PLENORA_ENUM_DIRECTION",
    ]
    .iter()
    .all(|key| std::env::var_os(key).is_none())
    {
        assert_eq!(rows.len(), DRIVERS.len() * NATIVE_ELEMENTS.len() * 2);
    }
    let discrepancies = rows.iter().filter(|row| row["status"] == "failed").count();
    let mut summary = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let mut required_missing = 0;
    for row in &mut rows {
        let outcome = match row["status"].as_str().unwrap() {
            "passed" => "passed",
            "not-applicable" => "not-applicable",
            "not-executed" => {
                assert!(row["driver"] == "filegdb" && !cfg!(feature = "gdal-backend"));
                "not-executed"
            }
            _ => "missing",
        };
        row["outcome"] = json!(outcome);
        let key = format!(
            "{} {}",
            row["driver"].as_str().unwrap(),
            row["direction"].as_str().unwrap()
        );
        let counts = summary.entry(key).or_insert_with(|| {
            ["passed", "not-applicable", "not-executed", "missing"]
                .into_iter()
                .map(|key| (key.to_owned(), 0))
                .collect()
        });
        *counts.get_mut(outcome).unwrap() += 1;
        if outcome == "missing" {
            required_missing += 1;
        }
    }
    std::fs::write(root.join("summary.json"),serde_json::to_vec_pretty(&json!({"counts":summary,"cells":rows.len(),"discrepancies":discrepancies,"required_scope_missing":required_missing,"scope":"all drivers, native Read and Arrow transport Write"})).unwrap()).unwrap();
    std::fs::write(
        root.join("coverage.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    assert!(!rows.is_empty(), "driver filter selected no matrix cells");
    let remaining = rows
        .iter()
        .filter(|r| {
            !matches!(
                r["status"].as_str(),
                Some("passed" | "not-applicable" | "not-executed")
            )
        })
        .map(|r| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                r["driver"],
                r["direction"],
                r["element"],
                r["status"],
                r.get("reason").unwrap_or(&Value::Null)
            )
        })
        .collect::<Vec<_>>();
    std::fs::write(root.join("remaining.tsv"), remaining.join("\n")).unwrap();
    if std::env::var_os("PLENORA_LOSS_GROUP_A").is_some() {
        group_a::write_audit(&root, &rows);
    }
    assert_eq!(
        required_missing, 0,
        "missing evidence in full Read/Write scope"
    );
    let failed = remaining.len();
    eprintln!("Enumeration artifacts: {}", root.display());
    assert_eq!(
        failed,
        0,
        "{failed}/{} cells without successful evidence; target/s1i-generated/coverage.json",
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

#[test]
fn null_geometry_rows_survive_xlsx_including_trailing_rows() {
    let base = generate(E::NullGeometry).unwrap().remove(0);
    let batch = RecordBatch::try_new(
        base.schema(),
        vec![Arc::new(BinaryArray::from(vec![None::<&[u8]>, None]))],
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.arrow");
    let output = dir.path().join("native.xlsx");
    let back = dir.path().join("back.arrow");
    save(&input, &batch);
    let (ok, report) = run(&input, &output, "ipc", "xls");
    assert!(ok, "{report}");
    let (ok, report) = run(&output, &back, "xls", "ipc");
    assert!(ok, "{report}");
    let after = native_batch(&output, &back, "xls");
    assert_eq!(after.num_rows(), 2);
    assert_eq!(after.column(0).null_count(), 2);
}
