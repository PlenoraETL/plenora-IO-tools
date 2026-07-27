//! Gate trasversale sui descrittori reali. Questi test impediscono che un
//! singolo driver aggiri le invarianti comuni di ADR-IO 1/3/4.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{DataType, Field, Schema};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryEncoding,
    GeometryType, LayerId, SpatialSemantics,
};
use plenora_core::crs::{CrsKind, CrsResolution, ResolvedCrs};
use plenora_core::geometry::{
    with_geometry_contract_metadata, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
};
use plenora_core::{CapabilityReason, PlenoraError};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsWriteSupport, Direction, FormatDescriptor,
    FormatDriver, NullabilitySupport, ProjectionSupport, ReadOptions, ReaderConcurrency, Runtime,
    Sink, Source, TypeCoercionPolicy, WriteLayer, WriteOptions, WritePlan,
};
use plenora_io_core::{BatchTarget, ProjectionMode, ReadRequest};

const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

fn drivers() -> Vec<Box<dyn FormatDriver>> {
    vec![
        Box::new(driver_geoparquet::GeoParquetDriver),
        Box::new(driver_geojson::GeoJsonDriver),
        Box::new(driver_csv::CsvDriver),
        Box::new(driver_gpkg::GpkgDriver),
        Box::new(driver_shp::ShpDriver),
        Box::new(driver_kml::KmlDriver),
        Box::new(driver_xls::XlsDriver),
        Box::new(driver_dxf::DxfDriver),
        Box::new(driver_filegdb::FileGdbDriver),
        Box::new(driver_ipc::IpcDriver),
    ]
}

fn resolved(id: &str) -> CrsResolution {
    CrsResolution::resolved(ResolvedCrs {
        id: Some(id.to_owned()),
        kind: CrsKind::Geographic,
        definition: Some(WGS84_WKT.to_owned()),
    })
}

fn valid_crs(descriptor: &FormatDescriptor) -> CrsResolution {
    match descriptor
        .write_capabilities
        .expect("driver scrivibile senza capability")
        .crs
    {
        CrsWriteSupport::Fixed(id) => resolved(id),
        CrsWriteSupport::Embedded | CrsWriteSupport::None => resolved("EPSG:4326"),
    }
}

fn geometry_plan(
    descriptor: &FormatDescriptor,
    crs: CrsResolution,
    dimensions: CoordinateDimensions,
    encoding: GeometryEncoding,
    semantics: SpatialSemantics,
    geometry_types: Vec<GeometryType>,
) -> WritePlan {
    let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), "geometry", crs, true);
    geometry.dimensions = dimensions;
    geometry.encoding = encoding;
    geometry.spatial_semantics = semantics;
    geometry.srid = (encoding == GeometryEncoding::Ewkb).then_some(4326);
    geometry.geometry_types = geometry_types;
    let base = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    )]));
    let field = with_geometry_contract_metadata(&base, &geometry);
    WritePlan {
        layers: vec![WriteLayer {
            name: format!("{}_layer", descriptor.id),
            contract: DataContract {
                schema: Arc::new(Schema::new(vec![field])),
                geometry: Some(geometry),
            },
        }],
    }
}

fn valid_geometry_plan(descriptor: &FormatDescriptor) -> WritePlan {
    let support = descriptor
        .write_capabilities
        .expect("driver scrivibile senza capability")
        .geometry;
    let dimensions = support
        .dimensions
        .iter()
        .copied()
        .find(|dimension| *dimension != CoordinateDimensions::Unknown)
        .or_else(|| support.dimensions.first().copied())
        .expect("driver del catalogo senza dimensioni geometriche");
    geometry_plan(
        descriptor,
        valid_crs(descriptor),
        dimensions,
        *support
            .encodings
            .first()
            .expect("driver del catalogo senza encoding geometrico"),
        *support
            .spatial_semantics
            .first()
            .expect("driver del catalogo senza semantica geometrica"),
        vec![GeometryType::Point],
    )
}

fn attribute_plan(layer_names: &[&str], field: Field) -> WritePlan {
    WritePlan {
        layers: layer_names
            .iter()
            .map(|name| WriteLayer {
                name: (*name).to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::new(vec![field.clone()])),
                    geometry: None,
                },
            })
            .collect(),
    }
}

fn assert_capability(driver: &str, result: plenora_core::Result<()>, expected: CapabilityReason) {
    assert!(
        matches!(
            result,
            Err(PlenoraError::Capability { reason, .. }) if reason == expected
        ),
        "{driver}: atteso {expected:?}"
    );
}

#[test]
fn descriptor_matrix_is_internally_coherent() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        assert!(
            descriptor.descriptor_version >= 3,
            "{}: descriptor legacy",
            descriptor.id
        );
        assert_eq!(
            descriptor.write_mode.is_some(),
            descriptor.write_capabilities.is_some(),
            "{}: write mode e capability incoerenti",
            descriptor.id
        );
        if let Some(capabilities) = descriptor.write_capabilities {
            assert_eq!(
                descriptor.multi_layer, capabilities.multi_layer,
                "{}: multi_layer incoerente",
                descriptor.id
            );
            assert_ne!(
                descriptor.direction,
                Direction::Read,
                "{}: capability di scrittura su driver read-only",
                descriptor.id
            );
        }
    }
}

#[test]
fn projection_contract_is_machine_readable_and_fail_closed() {
    let request = ReadRequest {
        layer: LayerId(0),
        projected_fields: Some(vec![FieldId(0)]),
        projection_mode: ProjectionMode::Required,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        batch_target: BatchTarget::default(),
    };
    let mut exact = Vec::new();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        match descriptor.projection_support {
            ProjectionSupport::Exact => {
                plenora_io_core::validate_read_projection(descriptor, &request)
                    .unwrap_or_else(|error| panic!("{}: exact respinta: {error}", descriptor.id));
                exact.push(descriptor.id);
            }
            ProjectionSupport::None => assert!(
                matches!(
                    plenora_io_core::validate_read_projection(descriptor, &request),
                    Err(PlenoraError::ProjectionUnsupported { driver })
                        if driver == descriptor.id
                ),
                "{}: Required non respinta fail-closed",
                descriptor.id
            ),
        }
    }
    exact.sort_unstable();
    assert_eq!(exact, vec!["geoparquet", "ipc"]);
}

#[test]
fn every_writable_driver_accepts_its_declared_baseline() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities.is_none() {
            continue;
        }
        validate_write(
            descriptor,
            &valid_geometry_plan(descriptor),
            &Default::default(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "{}: il contratto baseline derivato dal descrittore è stato respinto: {error}",
                descriptor.id
            )
        });
    }
}

#[test]
fn every_driver_rejects_invalid_layer_lifecycle() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities.is_none() {
            continue;
        }
        assert_capability(
            descriptor.id,
            validate_write(
                descriptor,
                &WritePlan { layers: Vec::new() },
                &Default::default(),
            ),
            CapabilityReason::EmptyWritePlan,
        );
        if descriptor.multi_layer {
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(
                        &["duplicate", "duplicate"],
                        Field::new("value", DataType::Utf8, true),
                    ),
                    &Default::default(),
                ),
                CapabilityReason::DuplicateLayerName,
            );
        } else {
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(&["one", "two"], Field::new("value", DataType::Utf8, true)),
                    &Default::default(),
                ),
                CapabilityReason::MultipleLayers,
            );
        }
    }
}

#[test]
fn crs_matrix_fails_closed() {
    let mut embedded = 0;
    let mut fixed = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities else {
            continue;
        };
        if !capabilities.geometry.supported {
            continue;
        }
        match capabilities.crs {
            CrsWriteSupport::Embedded => {
                let support = capabilities.geometry;
                let plan = geometry_plan(
                    descriptor,
                    CrsResolution::Missing,
                    support.dimensions[0],
                    support.encodings[0],
                    support.spatial_semantics[0],
                    vec![GeometryType::Point],
                );
                assert_capability(
                    descriptor.id,
                    validate_write(descriptor, &plan, &Default::default()),
                    CapabilityReason::CrsUnresolved,
                );
                embedded += 1;
            }
            CrsWriteSupport::Fixed(_) => {
                let support = capabilities.geometry;
                let plan = geometry_plan(
                    descriptor,
                    resolved("EPSG:3857"),
                    support.dimensions[0],
                    support.encodings[0],
                    support.spatial_semantics[0],
                    vec![GeometryType::Point],
                );
                assert_capability(
                    descriptor.id,
                    validate_write(descriptor, &plan, &Default::default()),
                    CapabilityReason::ReprojectionRequired,
                );
                fixed += 1;
            }
            CrsWriteSupport::None => {}
        }
    }
    assert!(embedded >= 5, "copertura CRS embedded insufficiente");
    assert!(fixed >= 2, "copertura CRS fisso insufficiente");
}

#[test]
fn geometry_capability_matrix_rejects_every_unsupported_axis() {
    let all_dimensions = [
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
        CoordinateDimensions::Unknown,
    ];
    let all_encodings = [GeometryEncoding::Wkb, GeometryEncoding::Ewkb];
    let all_semantics = [SpatialSemantics::Geometry, SpatialSemantics::Geography];
    let (mut dimensions_checked, mut encodings_checked, mut semantics_checked, mut mixed_checked) =
        (0, 0, 0, 0);

    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities else {
            continue;
        };
        let support = capabilities.geometry;
        if !support.supported {
            continue;
        }
        if let Some(unsupported) = all_dimensions
            .iter()
            .find(|dimension| !support.dimensions.contains(dimension))
        {
            let plan = geometry_plan(
                descriptor,
                valid_crs(descriptor),
                *unsupported,
                support.encodings[0],
                support.spatial_semantics[0],
                vec![GeometryType::Point],
            );
            assert_capability(
                descriptor.id,
                validate_write(descriptor, &plan, &Default::default()),
                CapabilityReason::CoordinateDimensions,
            );
            dimensions_checked += 1;
        }
        if let Some(unsupported) = all_encodings
            .iter()
            .find(|encoding| !support.encodings.contains(encoding))
        {
            let plan = geometry_plan(
                descriptor,
                valid_crs(descriptor),
                support.dimensions[0],
                *unsupported,
                support.spatial_semantics[0],
                vec![GeometryType::Point],
            );
            assert_capability(
                descriptor.id,
                validate_write(descriptor, &plan, &Default::default()),
                CapabilityReason::GeometryEncoding,
            );
            encodings_checked += 1;
        }
        if let Some(unsupported) = all_semantics
            .iter()
            .find(|semantics| !support.spatial_semantics.contains(semantics))
        {
            let plan = geometry_plan(
                descriptor,
                valid_crs(descriptor),
                support.dimensions[0],
                support.encodings[0],
                *unsupported,
                vec![GeometryType::Point],
            );
            assert_capability(
                descriptor.id,
                validate_write(descriptor, &plan, &Default::default()),
                CapabilityReason::SpatialSemantics,
            );
            semantics_checked += 1;
        }
        if !support.mixed_types {
            let plan = geometry_plan(
                descriptor,
                valid_crs(descriptor),
                support.dimensions[0],
                support.encodings[0],
                support.spatial_semantics[0],
                vec![GeometryType::Point, GeometryType::LineString],
            );
            assert_capability(
                descriptor.id,
                validate_write(descriptor, &plan, &Default::default()),
                CapabilityReason::MixedGeometry,
            );
            mixed_checked += 1;
        }
    }

    assert!(dimensions_checked >= 3);
    assert!(encodings_checked >= 6);
    assert!(semantics_checked >= 8);
    assert!(mixed_checked >= 1);
}

#[test]
fn field_type_and_limit_matrix_is_enforced() {
    let mut long_names_checked = 0;
    let mut rejected_types_checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities else {
            continue;
        };

        let limits = plenora_core::limits::Limits {
            max_columns: 0,
            ..Default::default()
        };
        assert!(matches!(
            validate_write(
                descriptor,
                &attribute_plan(&["layer"], Field::new("v", DataType::Utf8, true)),
                &limits
            ),
            Err(PlenoraError::LimitExceeded(_))
        ));

        if let Some(max_bytes) = capabilities.field_names.max_bytes {
            let name = "x".repeat(max_bytes + 1);
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], Field::new(name, DataType::Utf8, true)),
                    &Default::default(),
                ),
                CapabilityReason::FieldNameTooLong,
            );
            long_names_checked += 1;
        }

        if capabilities.type_coercion == TypeCoercionPolicy::Reject
            && !capabilities
                .allowed_types
                .contains(&plenora_io_core::ArrowTypeClass::Nested)
        {
            let nested = DataType::List(Arc::new(Field::new("item", DataType::Int64, true)));
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], Field::new("nested", nested, true)),
                    &Default::default(),
                ),
                CapabilityReason::TypeNotRepresentable,
            );
            rejected_types_checked += 1;
        }

        if matches!(
            capabilities.attributes,
            AttributeWriteSupport::None | AttributeWriteSupport::NamedSubset(_)
        ) {
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(
                        &["layer"],
                        Field::new("__not_a_native_attribute__", DataType::Utf8, true),
                    ),
                    &Default::default(),
                ),
                CapabilityReason::TypeNotRepresentable,
            );
        }

        if capabilities.nullability == NullabilitySupport::NoNulls {
            assert_capability(
                descriptor.id,
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], Field::new("nullable", DataType::Utf8, true)),
                    &Default::default(),
                ),
                CapabilityReason::Nullability,
            );
        }
    }
    assert!(long_names_checked >= 1);
    assert!(rejected_types_checked >= 1);
}

fn extension(driver: &str) -> &'static str {
    match driver {
        "geoparquet" => "parquet",
        "geojson" => "geojson",
        "csv" => "csv",
        "gpkg" => "gpkg",
        "shp" => "shp",
        "kml" => "kml",
        "xls" => "xlsx",
        "dxf" => "dxf",
        "ipc" => "arrow",
        other => panic!("estensione non definita per {other}"),
    }
}

fn output_path(directory: &tempfile::TempDir, driver: &str) -> PathBuf {
    directory
        .path()
        .join(format!("conformance.{}", extension(driver)))
}

fn read_request() -> ReadRequest {
    ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        batch_target: BatchTarget::default(),
    }
}

fn materialize_empty_dataset(driver: &dyn FormatDriver, directory: &tempfile::TempDir) -> PathBuf {
    let descriptor = driver.descriptor();
    let output = output_path(directory, descriptor.id);
    let plan = valid_geometry_plan(descriptor);
    let batch = RecordBatch::new_empty(plan.layers[0].contract.schema.clone());
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
        .unwrap_or_else(|error| panic!("{}: create dataset vuoto: {error}", descriptor.id));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("{}: write dataset vuoto: {error}", descriptor.id));
    writer
        .finish()
        .unwrap_or_else(|error| panic!("{}: finish dataset vuoto: {error}", descriptor.id));
    output
}

fn read_options(driver: &str) -> ReadOptions {
    let mut options = ReadOptions::default();
    if matches!(driver, "csv" | "dxf" | "xls") {
        options.assume_crs = Some("EPSG:4326".to_owned());
    }
    if matches!(driver, "csv" | "xls") {
        options
            .format_options
            .insert("wkt_column".to_owned(), "geometry".to_owned());
    }
    options
}

#[test]
fn required_projection_is_rejected_at_reader_open_by_non_exact_drivers() {
    let mut checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime != Runtime::PureRust
            || descriptor.projection_support != ProjectionSupport::None
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = materialize_empty_dataset(driver.as_ref(), &directory);
        let dataset = driver
            .open(Source::Path(output), &read_options(descriptor.id))
            .unwrap_or_else(|error| panic!("{}: open dataset vuoto: {error}", descriptor.id));
        let request = ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(vec![FieldId(0)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget::default(),
        };
        assert!(
            matches!(
                dataset.open_layer_reader(&request),
                Err(PlenoraError::ProjectionUnsupported { driver })
                    if driver == descriptor.id
            ),
            "{}: Required non respinta all'apertura",
            descriptor.id
        );
        checked += 1;
    }
    assert_eq!(checked, 7, "catalogo non-exact pure Rust inatteso");
}

#[test]
fn single_active_reader_is_enforced_by_every_pure_rust_descriptor() {
    let mut checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime != Runtime::PureRust
            || descriptor.reader_concurrency != ReaderConcurrency::SingleActiveReader
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = materialize_empty_dataset(driver.as_ref(), &directory);
        let dataset = driver
            .open(Source::Path(output), &read_options(descriptor.id))
            .unwrap_or_else(|error| panic!("{}: open dataset vuoto: {error}", descriptor.id));
        let first = dataset
            .open_layer_reader(&read_request())
            .unwrap_or_else(|error| panic!("{}: primo reader: {error}", descriptor.id));
        assert!(
            matches!(
                dataset.open_layer_reader(&read_request()),
                Err(PlenoraError::ReaderBusy { driver, layer: 0 })
                    if driver == descriptor.id
            ),
            "{}: secondo reader concorrente non respinto",
            descriptor.id
        );
        drop(first);
        dataset
            .open_layer_reader(&read_request())
            .unwrap_or_else(|error| panic!("{}: reader dopo drop: {error}", descriptor.id));
        checked += 1;
    }
    assert_eq!(checked, 3, "catalogo SingleActiveReader inatteso");
}

#[test]
fn independent_reader_descriptor_allows_two_live_readers() {
    let driver = driver_ipc::IpcDriver;
    assert_eq!(
        driver.descriptor().reader_concurrency,
        ReaderConcurrency::MultipleIndependentReaders
    );
    let directory = tempfile::tempdir().unwrap();
    let output = materialize_empty_dataset(&driver, &directory);
    let dataset = driver
        .open(Source::Path(output), &ReadOptions::default())
        .unwrap();

    let first = dataset.open_layer_reader(&read_request()).unwrap();
    let second = dataset.open_layer_reader(&read_request()).unwrap();
    drop((first, second));
}

#[test]
fn create_is_no_clobber_for_every_pure_rust_writer() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime != Runtime::PureRust || descriptor.write_capabilities.is_none() {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = output_path(&directory, descriptor.id);
        std::fs::write(&output, b"existing").unwrap();
        let result = driver.create(
            Sink::Path(output.clone()),
            &valid_geometry_plan(descriptor),
            &WriteOptions::default(),
        );
        assert!(
            matches!(result, Err(PlenoraError::OutputExists(_))),
            "{}: create non ha rispettato no-clobber",
            descriptor.id
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"existing",
            "{}: create ha modificato un output preesistente",
            descriptor.id
        );
    }
}

#[test]
fn dropping_writer_never_publishes_partial_output() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime != Runtime::PureRust || descriptor.write_capabilities.is_none() {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = output_path(&directory, descriptor.id);
        let writer = driver
            .create(
                Sink::Path(output.clone()),
                &valid_geometry_plan(descriptor),
                &WriteOptions::default(),
            )
            .unwrap_or_else(|error| panic!("{}: create valido: {error}", descriptor.id));
        drop(writer);
        assert!(
            !output.exists(),
            "{}: output pubblicato senza finish",
            descriptor.id
        );
        let residuals = std::fs::read_dir(directory.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            residuals.is_empty(),
            "{}: drop ha lasciato {} residui temporanei",
            descriptor.id,
            residuals.len()
        );
    }
}
