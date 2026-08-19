//! Gate trasversale sui descrittori reali. Questi test impediscono che un
//! singolo driver aggiri le invarianti comuni di ADR-IO 1/3/4.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsRepresentationCapabilities, CrsRepresentationState,
    CrsWriteSupport, DeterminismLevel, Direction, Fidelity, FormatDescriptor, FormatDriver,
    NullabilitySupport, PredicatePruningSupport, ProjectionSupport, ReadOptions, ReaderConcurrency,
    Runtime, Sink, Source, SpatialPruningSupport, TypeCoercionPolicy, WriteLayer, WriteOptions,
    WritePlan, ALL_GEOMETRY_TYPES,
};
use plenora_io_core::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryEncoding,
    GeometryType, LayerId, SpatialSemantics,
};
use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
use plenora_io_model::geometry::{
    with_geometry_contract_metadata, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
};
/// Tetto di colonne, dai limiti della pipeline.
///
/// Era `colonne_predefinite()`: il tipo legacy non esiste piu' nel
/// percorso core/driver (S4.e).
/// La conformita' verifica le capability di scrittura, non lo schema delle
/// opzioni: nessuno di questi piani ne dichiara, e la mappa vuota e' valida
/// per tutti e dieci i driver.
fn senza_opzioni() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::new()
}

fn colonne_predefinite() -> usize {
    usize::try_from(plenora_io_model::budget::PipelineLimits::default().max_columns())
        .unwrap_or(usize::MAX)
}
use plenora_io_model::{CancellationToken, CapabilityReason};

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
    CrsResolution::resolved(ResolvedCrs::new(
        Some(id.to_owned()),
        CrsKind::Geographic,
        Some(WGS84_WKT.to_owned()),
    ))
}

fn valid_crs(descriptor: &FormatDescriptor) -> CrsResolution {
    match descriptor
        .write_capabilities()
        .expect("driver scrivibile senza capability")
        .crs
    {
        CrsWriteSupport::Fixed(id) => resolved(id),
        CrsWriteSupport::Embedded | CrsWriteSupport::EmbeddedOptional | CrsWriteSupport::None => {
            resolved("EPSG:4326")
        }
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
    geometry.set_exact_geometry_types(geometry_types);
    let base = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    )]));
    let field = with_geometry_contract_metadata(&base, &geometry);
    WritePlan {
        layers: vec![WriteLayer {
            name: format!("{}_layer", descriptor.id()),
            contract: DataContract {
                schema: Arc::new(Schema::new(vec![field])),
                geometry: Some(geometry),
            },
        }],
    }
}

fn valid_geometry_plan(descriptor: &FormatDescriptor) -> WritePlan {
    let support = descriptor
        .write_capabilities()
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

fn attribute_plan(layer_names: &[&str], field: &Field) -> WritePlan {
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

fn assert_capability(
    driver: &str,
    result: plenora_io_model::Result<()>,
    expected: CapabilityReason,
) {
    assert!(
        matches!(
            result,
            Err(error) if error.capability_reason == Some(expected)
        ),
        "{driver}: atteso {expected:?}"
    );
}

#[test]
fn descriptor_matrix_is_internally_coherent() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        assert!(
            descriptor.descriptor_version() >= 5,
            "{}: descriptor legacy",
            descriptor.id()
        );
        assert!(
            descriptor.driver_version() >= 2,
            "{}: versione implementazione non aggiornata",
            descriptor.id()
        );
        assert_eq!(
            descriptor.write_mode().is_some(),
            descriptor.write_capabilities().is_some(),
            "{}: write mode e capability incoerenti",
            descriptor.id()
        );
        assert_eq!(
            descriptor.write_mode().is_some(),
            descriptor.write_determinism().is_some(),
            "{}: write mode e determinismo incoerenti",
            descriptor.id()
        );
        assert_ne!(
            descriptor.read_determinism(),
            DeterminismLevel::Unordered,
            "{}: sorgente locale dichiarata non ordinata senza snapshot remoto",
            descriptor.id()
        );
        if let Some(capabilities) = descriptor.write_capabilities() {
            assert_eq!(
                capabilities.geometry.supported,
                !capabilities.geometry.geometry_types.is_empty(),
                "{}: tipi geometrici e supporto incoerenti",
                descriptor.id()
            );
            let unique_types = capabilities
                .geometry
                .geometry_types
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                unique_types.len(),
                capabilities.geometry.geometry_types.len(),
                "{}: tipi geometrici duplicati",
                descriptor.id()
            );
            assert_eq!(
                descriptor.multi_layer(),
                capabilities.multi_layer,
                "{}: multi_layer incoerente",
                descriptor.id()
            );
            assert_ne!(
                descriptor.direction(),
                Direction::Read,
                "{}: capability di scrittura su driver read-only",
                descriptor.id()
            );
        }
    }
}

#[test]
fn pruning_capabilities_match_the_implemented_native_paths() {
    let mut predicate = Vec::new();
    let mut spatial = Vec::new();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.predicate_pruning_support() != PredicatePruningSupport::None {
            predicate.push((descriptor.id(), descriptor.predicate_pruning_support()));
        }
        if descriptor.spatial_pruning_support() != SpatialPruningSupport::None {
            spatial.push((descriptor.id(), descriptor.spatial_pruning_support()));
        }
    }
    assert_eq!(
        predicate,
        vec![(
            "geoparquet",
            PredicatePruningSupport::NumericMinMaxStatistics,
        )]
    );
    assert_eq!(
        spatial,
        vec![
            ("geoparquet", SpatialPruningSupport::BoundingBoxStatistics,),
            ("gpkg", SpatialPruningSupport::OptionalRtreeIndex),
        ]
    );
}

#[test]
fn projection_contract_is_machine_readable_and_fail_closed() {
    let request = ReadRequest {
        layer: LayerId(0),
        projected_fields: Some(vec![FieldId(0)]),
        projection_mode: ProjectionMode::Required,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };
    let mut exact = Vec::new();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        match descriptor.projection_support() {
            ProjectionSupport::Exact => {
                plenora_io_core::validate_read_projection(descriptor, &request)
                    .unwrap_or_else(|error| panic!("{}: exact respinta: {error}", descriptor.id()));
                exact.push(descriptor.id());
            }
            ProjectionSupport::None => assert!(
                matches!(
                    plenora_io_core::validate_read_projection(descriptor, &request),
                    Err(error)
                        if error.code == plenora_io_model::IoErrorCode::ProjectionUnsupported
                            && error.driver.as_deref() == Some(descriptor.id())
                ),
                "{}: Required non respinta fail-closed",
                descriptor.id()
            ),
        }
    }
    exact.sort_unstable();
    assert_eq!(
        exact,
        vec![
            "csv",
            "filegdb",
            "geojson",
            "geoparquet",
            "gpkg",
            "ipc",
            "shp"
        ]
    );
}

#[test]
fn every_writable_driver_accepts_its_declared_baseline() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities().is_none() {
            continue;
        }
        validate_write(
            descriptor,
            &valid_geometry_plan(descriptor),
            colonne_predefinite(),
            &senza_opzioni(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "{}: il contratto baseline derivato dal descrittore è stato respinto: {error}",
                descriptor.id()
            )
        });
    }
}

#[test]
fn every_driver_rejects_invalid_layer_lifecycle() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities().is_none() {
            continue;
        }
        assert_capability(
            descriptor.id(),
            validate_write(
                descriptor,
                &WritePlan { layers: Vec::new() },
                colonne_predefinite(),
                &senza_opzioni(),
            ),
            CapabilityReason::EmptyWritePlan,
        );
        if descriptor.multi_layer() {
            assert_capability(
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(
                        &["duplicate", "duplicate"],
                        &Field::new("value", DataType::Utf8, true),
                    ),
                    colonne_predefinite(),
                    &senza_opzioni(),
                ),
                CapabilityReason::DuplicateLayerName,
            );
        } else {
            assert_capability(
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(&["one", "two"], &Field::new("value", DataType::Utf8, true)),
                    colonne_predefinite(),
                    &senza_opzioni(),
                ),
                CapabilityReason::MultipleLayers,
            );
        }
    }
}

#[test]
fn crs_matrix_fails_closed() {
    let mut embedded = 0;
    let mut optional_embedded = 0;
    let mut fixed = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities() else {
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
                    descriptor.id(),
                    validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
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
                    descriptor.id(),
                    validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
                    CapabilityReason::ReprojectionRequired,
                );
                fixed += 1;
            }
            CrsWriteSupport::EmbeddedOptional => {
                let support = capabilities.geometry;
                let plan = geometry_plan(
                    descriptor,
                    CrsResolution::Missing,
                    support.dimensions[0],
                    support.encodings[0],
                    support.spatial_semantics[0],
                    vec![GeometryType::Point],
                );
                assert!(
                    validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni())
                        .is_ok(),
                    "{} dichiara CRS embedded opzionale ma rifiuta lo stato missing",
                    descriptor.id()
                );
                optional_embedded += 1;
            }
            CrsWriteSupport::None => {}
        }
    }
    assert!(embedded >= 5, "copertura CRS embedded insufficiente");
    assert_eq!(
        optional_embedded, 1,
        "il profilo CRS embedded opzionale deve essere esercitato"
    );
    assert!(fixed >= 2, "copertura CRS fisso insufficiente");
}

#[test]
fn combined_crs_propagates_to_ipc_and_fails_closed_for_shapefile() {
    let ipc = driver_ipc::IpcDriver;
    let mut ipc_plan = valid_geometry_plan(ipc.descriptor());
    ipc_plan.layers[0].contract.geometry.as_mut().unwrap().srid = Some(3003);
    assert!(
        validate_write(
            ipc.descriptor(),
            &ipc_plan,
            colonne_predefinite(),
            &senza_opzioni()
        )
        .is_ok(),
        "IPC deve preservare crs_id e srid discordanti senza sceglierne uno"
    );

    let shp = driver_shp::ShpDriver;
    let mut shp_plan = valid_geometry_plan(shp.descriptor());
    shp_plan.layers[0].contract.geometry.as_mut().unwrap().srid = Some(3003);
    assert_capability(
        "shp",
        validate_write(
            shp.descriptor(),
            &shp_plan,
            colonne_predefinite(),
            &senza_opzioni(),
        ),
        CapabilityReason::CrsRepresentationsInconsistent,
    );
}

#[test]
fn every_writer_declares_the_reviewed_crs_representation_matrix() {
    use CrsRepresentationState::{Absent, Derived, Preserved};

    let expected = HashMap::from([
        (
            "ipc",
            CrsRepresentationCapabilities::new(Preserved, Preserved, Preserved),
        ),
        (
            "geoparquet",
            CrsRepresentationCapabilities::new(Preserved, Absent, Absent),
        ),
        (
            "gpkg",
            CrsRepresentationCapabilities::new(Preserved, Derived, Derived),
        ),
        (
            "shp",
            CrsRepresentationCapabilities::new(Derived, Absent, Preserved),
        ),
        (
            "dxf",
            CrsRepresentationCapabilities::new(Derived, Absent, Preserved),
        ),
        (
            "filegdb",
            CrsRepresentationCapabilities::new(Derived, Absent, Derived),
        ),
        (
            "geojson",
            CrsRepresentationCapabilities::new(Derived, Absent, Absent),
        ),
        (
            "kml",
            CrsRepresentationCapabilities::new(Derived, Absent, Absent),
        ),
        (
            "csv",
            CrsRepresentationCapabilities::new(Absent, Absent, Absent),
        ),
        (
            "xls",
            CrsRepresentationCapabilities::new(Absent, Absent, Absent),
        ),
    ]);

    for driver in drivers() {
        let descriptor = driver.descriptor();
        assert_eq!(
            descriptor.write_capabilities().unwrap().crs_representations,
            expected[descriptor.id()],
            "{} ha una capability CRS diversa dalla matrice revisionata",
            descriptor.id()
        );
    }
}

// La matrice copre in un solo test tutti gli assi di capability geometrica per
// ogni driver del catalogo: spezzarla perderebbe la garanzia che l'insieme sia
// esaustivo.
#[allow(clippy::too_many_lines)]
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
    let (
        mut dimensions_checked,
        mut encodings_checked,
        mut semantics_checked,
        mut types_checked,
        mut unresolved_checked,
        mut mixed_checked,
        mut writable_geometry_profiles,
    ) = (0, 0, 0, 0, 0, 0, 0);

    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities() else {
            continue;
        };
        let support = capabilities.geometry;
        if !support.supported {
            continue;
        }
        writable_geometry_profiles += 1;
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
                descriptor.id(),
                validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
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
                descriptor.id(),
                validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
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
                descriptor.id(),
                validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
                CapabilityReason::SpatialSemantics,
            );
            semantics_checked += 1;
        }
        if let Some(unsupported) = ALL_GEOMETRY_TYPES
            .iter()
            .find(|geometry_type| !support.geometry_types.contains(geometry_type))
        {
            let plan = geometry_plan(
                descriptor,
                valid_crs(descriptor),
                support.dimensions[0],
                support.encodings[0],
                support.spatial_semantics[0],
                vec![*unsupported],
            );
            assert_capability(
                descriptor.id(),
                validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
                CapabilityReason::GeometryNotSupported,
            );
            types_checked += 1;

            let mut unresolved = valid_geometry_plan(descriptor);
            let geometry = unresolved.layers[0].contract.geometry.as_mut().unwrap();
            geometry.geometry_types.clear();
            geometry.types_declaration = plenora_io_model::contract::TypesDeclaration::Unresolved;
            assert_capability(
                descriptor.id(),
                validate_write(
                    descriptor,
                    &unresolved,
                    colonne_predefinite(),
                    &senza_opzioni(),
                ),
                CapabilityReason::GeometryNotSupported,
            );
            unresolved_checked += 1;
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
                descriptor.id(),
                validate_write(descriptor, &plan, colonne_predefinite(), &senza_opzioni()),
                CapabilityReason::MixedGeometry,
            );
            mixed_checked += 1;
        }
    }

    assert!(dimensions_checked >= 3);
    assert!(encodings_checked >= 6);
    assert!(semantics_checked >= 8);
    assert_eq!(types_checked, writable_geometry_profiles);
    assert_eq!(unresolved_checked, writable_geometry_profiles);
    assert!(mixed_checked >= 2);
}

#[test]
fn field_type_and_limit_matrix_is_enforced() {
    let mut long_names_checked = 0;
    let mut rejected_types_checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        let Some(capabilities) = descriptor.write_capabilities() else {
            continue;
        };

        // Tetto di colonne a zero: il piano ha una colonna, quindi il
        // rifiuto deve arrivare prima di qualunque scrittura.
        assert!(matches!(
            validate_write(
                descriptor,
                &attribute_plan(&["layer"], &Field::new("v", DataType::Utf8, true)),
                0, &senza_opzioni()),
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));

        if let Some(max_bytes) = capabilities.field_names.max_bytes {
            let name = "x".repeat(max_bytes + 1);
            assert_capability(
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], &Field::new(name, DataType::Utf8, true)),
                    colonne_predefinite(),
                    &senza_opzioni(),
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
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], &Field::new("nested", nested, true)),
                    colonne_predefinite(),
                    &senza_opzioni(),
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
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(
                        &["layer"],
                        &Field::new("__not_a_native_attribute__", DataType::Utf8, true),
                    ),
                    colonne_predefinite(),
                    &senza_opzioni(),
                ),
                CapabilityReason::TypeNotRepresentable,
            );
        }

        if capabilities.nullability == NullabilitySupport::NoNulls {
            assert_capability(
                descriptor.id(),
                validate_write(
                    descriptor,
                    &attribute_plan(&["layer"], &Field::new("nullable", DataType::Utf8, true)),
                    colonne_predefinite(),
                    &senza_opzioni(),
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
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    }
}

fn materialize_empty_dataset(driver: &dyn FormatDriver, directory: &tempfile::TempDir) -> PathBuf {
    let descriptor = driver.descriptor();
    let output = output_path(directory, descriptor.id());
    let plan = valid_geometry_plan(descriptor);
    let batch = RecordBatch::new_empty(plan.layers[0].contract.schema.clone());
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap_or_else(|error| panic!("{}: create dataset vuoto: {error}", descriptor.id()));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("{}: write dataset vuoto: {error}", descriptor.id()));
    writer
        .finish()
        .unwrap_or_else(|error| panic!("{}: finish dataset vuoto: {error}", descriptor.id()));
    output
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(21);
    bytes.push(1);
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

fn require_ok<T, E: std::fmt::Display>(
    result: Result<T, E>,
    driver_id: &str,
    operation: &str,
) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{driver_id}: {operation}: {error}"),
    }
}

fn materialize_point_dataset(driver: &dyn FormatDriver, directory: &tempfile::TempDir) -> PathBuf {
    let descriptor = driver.descriptor();
    let output = output_path(directory, descriptor.id());
    let plan = valid_geometry_plan(descriptor);
    let geometry = point_wkb(12.5, -7.25);
    let batch = RecordBatch::try_new(
        plan.layers[0].contract.schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())]))],
    )
    .unwrap();
    let mut writer = require_ok(
        driver.create(Sink::Path(output.clone()), &plan, &opzioni_scrittura()),
        descriptor.id(),
        "create determinismo",
    );
    require_ok(writer.write(&batch), descriptor.id(), "write determinismo");
    require_ok(writer.finish(), descriptor.id(), "finish determinismo");
    output
}

fn read_all_batches(driver: &dyn FormatDriver, source: PathBuf) -> Vec<RecordBatch> {
    let descriptor = driver.descriptor();
    let dataset = require_ok(
        driver.open(Source::Path(source), read_options(descriptor.id())),
        descriptor.id(),
        "open determinismo",
    );
    let mut reader = require_ok(
        dataset.open_layer_reader(&read_request()),
        descriptor.id(),
        "reader determinismo",
    );
    let mut batches = Vec::new();
    while let Some(batch) = require_ok(reader.next_batch(), descriptor.id(), "next determinismo") {
        batches.push(batch);
    }
    batches
}

#[test]
fn repeated_local_operations_preserve_semantic_results() {
    let mut checked = Vec::new();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust || descriptor.write_capabilities().is_none() {
            continue;
        }
        let first_directory = tempfile::tempdir().unwrap();
        let second_directory = tempfile::tempdir().unwrap();
        let first = materialize_point_dataset(driver.as_ref(), &first_directory);
        let second = materialize_point_dataset(driver.as_ref(), &second_directory);

        assert_eq!(
            read_all_batches(driver.as_ref(), first),
            read_all_batches(driver.as_ref(), second),
            "{}: due esecuzioni equivalenti hanno prodotto risultati semantici diversi",
            descriptor.id()
        );
        checked.push(descriptor.id());
    }
    assert_eq!(
        checked,
        vec![
            "geoparquet",
            "geojson",
            "csv",
            "gpkg",
            "shp",
            "kml",
            "xls",
            "dxf",
            "ipc",
        ]
    );
}

#[test]
fn conditional_writers_report_planned_loss_instead_of_empty_reports() {
    let mut checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust
            || descriptor.fidelity_class() != Fidelity::Conditional
            || descriptor.write_capabilities().is_none()
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = output_path(&directory, descriptor.id());
        let plan = valid_geometry_plan(descriptor);
        let batch = RecordBatch::new_empty(plan.layers[0].contract.schema.clone());
        let mut writer = driver
            .create(Sink::Path(output), &plan, &opzioni_scrittura())
            .unwrap_or_else(|error| panic!("{}: create: {error}", descriptor.id()));

        let preventive = writer.fidelity_assessment().level;
        writer
            .write(&batch)
            .unwrap_or_else(|error| panic!("{}: write: {error}", descriptor.id()));
        let published = writer
            .finish()
            .unwrap_or_else(|error| panic!("{}: finish: {error}", descriptor.id()));
        assert_eq!(
            published.fidelity.level,
            preventive,
            "{}: assessment finale divergente",
            descriptor.id()
        );
        match preventive {
            Fidelity::Conditional => assert!(
                published.loss.is_empty(),
                "{}: loss inattesa",
                descriptor.id()
            ),
            Fidelity::Approximating => assert!(
                !published.loss.is_empty(),
                "{}: perdita pianificata senza LossReport",
                descriptor.id()
            ),
            Fidelity::Lossless => {
                panic!(
                    "{}: classe Conditional degradata a Lossless",
                    descriptor.id()
                )
            }
        }
        checked += 1;
    }
    // 6 dopo il fix #8 della review 2026-08-15: GeoJSON e' passato da
    // `Fidelity::Lossless` a `Fidelity::Conditional`, in linea con il
    // principio scritto in `IMPLEMENTATION_STATUS.md` ("un report vuoto
    // significa 'nessuna perdita osservata', non `Lossless`") e con il fatto
    // che il writer non conserva `id`, `bbox` ne' foreign members.
    assert_eq!(checked, 6, "catalogo Conditional pure Rust inatteso");
}

/// Opzioni di lettura sul modello unificato (S4.d).
/// Opzioni di scrittura sul modello unificato.
///
/// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
/// `OperationBudget`, che nasce da una costruzione che puo' fallire.
fn opzioni_scrittura() -> WriteOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle non costruibile: {error:?}"),
    }
}

fn read_options(driver: &str) -> ReadOptions {
    let mut options = opzioni_lettura();
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
        if descriptor.runtime() != Runtime::PureRust
            || descriptor.projection_support() != ProjectionSupport::None
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = materialize_empty_dataset(driver.as_ref(), &directory);
        let dataset = driver
            .open(Source::Path(output), read_options(descriptor.id()))
            .unwrap_or_else(|error| panic!("{}: open dataset vuoto: {error}", descriptor.id()));
        let request = ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(vec![FieldId(0)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        };
        assert!(
            matches!(
                dataset.open_layer_reader(&request),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::ProjectionUnsupported
                        && error.driver.as_deref() == Some(descriptor.id())
            ),
            "{}: Required non respinta all'apertura",
            descriptor.id()
        );
        checked += 1;
    }
    assert_eq!(checked, 3, "catalogo non-exact pure Rust inatteso");
}

#[test]
fn every_exact_pure_rust_reader_supports_an_empty_projection() {
    let mut checked = Vec::new();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust
            || descriptor.projection_support() != ProjectionSupport::Exact
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = materialize_point_dataset(driver.as_ref(), &directory);
        let dataset = match driver.open(Source::Path(output), read_options(descriptor.id())) {
            Ok(dataset) => dataset,
            Err(error) => panic!("{}: open projection: {error}", descriptor.id()),
        };
        let mut reader = match dataset.open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(Vec::new()),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        }) {
            Ok(reader) => reader,
            Err(error) => panic!("{}: projection vuota: {error}", descriptor.id()),
        };
        let batch = match reader.next_batch() {
            Ok(Some(batch)) => batch,
            Ok(None) => panic!("{}: projection ha perso la riga", descriptor.id()),
            Err(error) => panic!("{}: next projection: {error}", descriptor.id()),
        };
        assert_eq!(batch.num_rows(), 1, "{}", descriptor.id());
        assert_eq!(batch.num_columns(), 0, "{}", descriptor.id());
        checked.push(descriptor.id());
    }
    checked.sort_unstable();
    assert_eq!(
        checked,
        vec!["csv", "geojson", "geoparquet", "gpkg", "ipc", "shp"]
    );
}

#[test]
fn single_active_reader_is_enforced_by_every_pure_rust_descriptor() {
    let mut checked = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust
            || descriptor.reader_concurrency() != ReaderConcurrency::SingleActiveReader
        {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = materialize_empty_dataset(driver.as_ref(), &directory);
        let dataset = driver
            .open(Source::Path(output), read_options(descriptor.id()))
            .unwrap_or_else(|error| panic!("{}: open dataset vuoto: {error}", descriptor.id()));
        let first = dataset
            .open_layer_reader(&read_request())
            .unwrap_or_else(|error| panic!("{}: primo reader: {error}", descriptor.id()));
        assert!(
            matches!(
                dataset.open_layer_reader(&read_request()),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                        && error.driver.as_deref() == Some(descriptor.id())
            ),
            "{}: secondo reader concorrente non respinto",
            descriptor.id()
        );
        drop(first);
        dataset
            .open_layer_reader(&read_request())
            .unwrap_or_else(|error| panic!("{}: reader dopo drop: {error}", descriptor.id()));
        checked += 1;
    }
    assert_eq!(checked, 3, "catalogo SingleActiveReader inatteso");
}

#[test]
fn independent_reader_descriptor_allows_two_live_readers() {
    let driver = driver_ipc::IpcDriver;
    assert_eq!(
        driver.descriptor().reader_concurrency(),
        ReaderConcurrency::MultipleIndependentReaders
    );
    let directory = tempfile::tempdir().unwrap();
    let output = materialize_empty_dataset(&driver, &directory);
    let dataset = driver
        .open(Source::Path(output), opzioni_lettura())
        .unwrap();

    let first = dataset.open_layer_reader(&read_request()).unwrap();
    let second = dataset.open_layer_reader(&read_request()).unwrap();
    drop((first, second));
}

#[test]
fn create_is_no_clobber_for_every_pure_rust_writer() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust || descriptor.write_capabilities().is_none() {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = output_path(&directory, descriptor.id());
        std::fs::write(&output, b"existing").unwrap();
        let result = driver.create(
            Sink::Path(output.clone()),
            &valid_geometry_plan(descriptor),
            &opzioni_scrittura(),
        );
        assert!(
            matches!(
                result,
                Err(error) if error.code == plenora_io_model::IoErrorCode::OutputExists
            ),
            "{}: create non ha rispettato no-clobber",
            descriptor.id()
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"existing",
            "{}: create ha modificato un output preesistente",
            descriptor.id()
        );
    }
}

#[test]
fn dropping_writer_never_publishes_partial_output() {
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.runtime() != Runtime::PureRust || descriptor.write_capabilities().is_none() {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = output_path(&directory, descriptor.id());
        let writer = driver
            .create(
                Sink::Path(output.clone()),
                &valid_geometry_plan(descriptor),
                &opzioni_scrittura(),
            )
            .unwrap_or_else(|error| panic!("{}: create valido: {error}", descriptor.id()));
        drop(writer);
        assert!(
            !output.exists(),
            "{}: output pubblicato senza finish",
            descriptor.id()
        );
        let residuals = std::fs::read_dir(directory.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            residuals.is_empty(),
            "{}: drop ha lasciato {} residui temporanei",
            descriptor.id(),
            residuals.len()
        );
    }
}

// ---------------------------------------------------------------------------
// L0.7 / S6 — lo schema dichiarativo delle `format_options`
// ---------------------------------------------------------------------------

use plenora_io_model::format_options::{FaseOpzione, ValoreAmmesso};

/// Il registro delle opzioni, composto in core dall'elenco dei driver.
fn registro_opzioni() -> plenora_io_core::DriverRegistry {
    let mut registry = plenora_io_core::DriverRegistry::new();
    for driver in drivers() {
        registry.register(driver);
    }
    registry
}

/// La destinazione che un valore enumerato pretende, quando la pretende.
///
/// `publish_mode` e' l'unico caso oggi: il suo valore deve concordare con il
/// suffisso del sink, un vincolo che lega due cose — opzione e destinazione —
/// e che nessuna grammatica su un singolo valore puo' esprimere. La tabella
/// sta qui, esplicita, invece di far saltare il caso al test: se domani
/// un'altra opzione avesse lo stesso accoppiamento, si aggiunge una riga e
/// resta visibile che l'accoppiamento esiste.
fn destinazione_richiesta(chiave: &str, valore: &str) -> Option<&'static str> {
    match (chiave, valore) {
        ("publish_mode", "shapefile_directory_dataset") => Some("shp.d"),
        ("publish_mode", "loose_shapefile_set") => Some("shp"),
        _ => None,
    }
}

fn opzioni_scrittura_con(chiave: &str, valore: &str) -> WriteOptions {
    let mut opts = opzioni_scrittura();
    opts.format_options
        .insert(chiave.to_owned(), valore.to_owned());
    opts
}

/// Scrive un dataset minimo e restituisce l'esito, senza `unwrap`.
fn scrive_con(
    driver: &dyn FormatDriver,
    destinazione: PathBuf,
    opts: &WriteOptions,
) -> plenora_io_model::Result<()> {
    let descriptor = driver.descriptor();
    let plan = valid_geometry_plan(descriptor);
    let batch = RecordBatch::new_empty(plan.layers[0].contract.schema.clone());
    let mut writer = driver.create(Sink::Path(destinazione), &plan, opts)?;
    writer.write(&batch)?;
    writer.finish()?;
    Ok(())
}

#[test]
fn every_driver_has_a_schema_for_options() {
    let registro = registro_opzioni();
    let schemi = registro.format_options();
    let attesi: Vec<&str> = drivers()
        .iter()
        .map(|driver| driver.descriptor().id())
        .collect();

    assert_eq!(
        schemi.len(),
        attesi.len(),
        "il registro non copre l'elenco dei driver"
    );
    for id in &attesi {
        assert!(
            schemi.iter().any(|(registrato, _)| registrato == id),
            "{id}: assente dal registro delle format_options"
        );
    }

    // Il registro e' derivato, quindi la copertura e' garantita dal tipo: qui
    // si sorveglia che resti derivato. Cio' che invece un tipo non garantisce
    // e' che le chiavi siano sensate — nessun duplicato, nessuna vuota — e che
    // un default dichiarato sia esso stesso valido secondo la propria forma.
    for (id, schema) in schemi {
        let mut viste = std::collections::BTreeSet::new();
        for opzione in schema.opzioni {
            assert!(!opzione.chiave.is_empty(), "{id}: opzione con chiave vuota");
            assert!(
                viste.insert(opzione.chiave),
                "{id}: chiave '{}' dichiarata due volte",
                opzione.chiave
            );
            assert!(
                !opzione.descrizione.is_empty(),
                "{id}: '{}' senza descrizione",
                opzione.chiave
            );
            if let Some(predefinito) = opzione.predefinito {
                assert!(
                    opzione.valore.verifica(predefinito).is_ok(),
                    "{id}: il default '{predefinito}' di '{}' non rispetta la propria forma",
                    opzione.chiave
                );
            }
        }
    }
}

#[test]
fn unknown_option_key_produces_typed_error_not_silent_ignore() {
    let directory = tempfile::tempdir().unwrap();
    for driver in drivers() {
        let descriptor = driver.descriptor();
        // Senza il tier GDB, FileGDB rifiuta per driver indisponibile prima di
        // guardare le opzioni. Non e' un'ignoranza silenziosa — e' un rifiuto
        // che precede — e pretendere qui la categoria dello schema misurerebbe
        // l'ordine dei controlli invece del controllo. Con la feature attiva
        // (gate «test FileGDB feature-on») il driver rientra nel giro.
        if descriptor.id() == "filegdb" && !cfg!(feature = "gdal-backend") {
            continue;
        }
        let mut opts = read_options(descriptor.id());
        opts.format_options
            .insert("optzione_inesistente".to_owned(), "x".to_owned());

        let esito = driver.open(
            Source::Path(directory.path().join("non-serve-che-esista")),
            opts,
        );
        let Err(errore) = esito else {
            panic!(
                "{}: una chiave sconosciuta e' stata ignorata",
                descriptor.id()
            )
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration,
            "{}: categoria sbagliata per una chiave sconosciuta ({errore})",
            descriptor.id()
        );
        let testo = errore.to_string();
        assert!(
            testo.contains("optzione_inesistente"),
            "{}: l'errore non nomina la chiave rifiutata: {errore}",
            descriptor.id()
        );
        // L'errore deve elencare cosa si accetta, non solo cosa si rifiuta:
        // altrimenti chi ha sbagliato a scrivere non sa dove guardare.
        let attese = descriptor.format_options().chiavi(FaseOpzione::Lettura);
        if attese.is_empty() {
            assert!(
                testo.contains("nessuna"),
                "{}: schema vuoto, ma l'errore non lo dice: {errore}",
                descriptor.id()
            );
        } else {
            for chiave in attese {
                assert!(
                    testo.contains(chiave),
                    "{}: l'errore non elenca '{chiave}': {errore}",
                    descriptor.id()
                );
            }
        }
    }
}

#[test]
fn unknown_compression_value_produces_typed_error_not_snappy_default() {
    let directory = tempfile::tempdir().unwrap();
    let driver = driver_geoparquet::GeoParquetDriver;

    // Il caso peggiore del censimento: `zstsd` e' un refuso di `zstd`, e prima
    // produceva un file valido compresso con snappy — nessun errore, nessun
    // avviso, e la compressione richiesta semplicemente non applicata.
    let destinazione = directory.path().join("refuso.parquet");
    let esito = scrive_con(
        &driver,
        destinazione.clone(),
        &opzioni_scrittura_con("compression", "zstsd"),
    );
    let Err(errore) = esito else {
        panic!("un valore di compressione ignoto e' degradato a snappy in silenzio")
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration,
        "categoria sbagliata: {errore}"
    );
    let testo = errore.to_string();
    assert!(
        testo.contains("zstsd"),
        "l'errore non nomina il valore rifiutato: {errore}"
    );
    assert!(
        testo.contains("zstd"),
        "l'errore non elenca i valori ammessi: {errore}"
    );

    // E il file non deve esistere: un errore che lascia un artefatto parziale
    // sposta il problema piu' avanti invece di fermarlo.
    assert!(
        !destinazione.exists(),
        "il rifiuto ha lasciato un file al suo posto"
    );
}

#[test]
fn una_chiave_di_scrittura_passata_in_lettura_e_rifiutata() {
    let directory = tempfile::tempdir().unwrap();
    let mut provati = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        for opzione in descriptor.format_options().opzioni {
            if opzione.fase != FaseOpzione::Scrittura {
                continue;
            }
            provati += 1;
            let mut opts = read_options(descriptor.id());
            let valore = match opzione.valore {
                ValoreAmmesso::Enumerato(ammessi) => {
                    (*ammessi.first().expect("enumerazione senza valori")).to_owned()
                }
                ValoreAmmesso::Booleano => "true".to_owned(),
                ValoreAmmesso::Carattere => ";".to_owned(),
                ValoreAmmesso::Intero { minimo, .. } => minimo.to_string(),
                ValoreAmmesso::Testo => "x".to_owned(),
            };
            opts.format_options
                .insert(opzione.chiave.to_owned(), valore);

            let esito = driver.open(Source::Path(directory.path().join("qualunque")), opts);
            let Err(errore) = esito else {
                panic!(
                    "{}: '{}' vale in scrittura ma la lettura l'ha accettata",
                    descriptor.id(),
                    opzione.chiave
                )
            };
            assert_eq!(
                errore.category,
                plenora_io_model::ErrorCategory::InvalidConfiguration,
                "{}: '{}' rifiutata con la categoria sbagliata: {errore}",
                descriptor.id(),
                opzione.chiave
            );
            // L'errore deve dire *perche'*: la fase, non un generico rifiuto.
            assert!(
                errore.to_string().contains("scrittura"),
                "{}: '{}' rifiutata senza nominare la fase: {errore}",
                descriptor.id(),
                opzione.chiave
            );
        }
    }
    assert!(
        provati > 0,
        "nessuna opzione di sola scrittura: il test non ha verificato niente"
    );
}

#[test]
fn ogni_valore_ammesso_dallo_schema_e_accettato_dal_driver() {
    // Copre le enumerazioni di **scrittura** con una scrittura vera: e' li'
    // che lo schema puo' promettere piu' di quanto il driver mantenga, perche'
    // il driver traduce il valore in una scelta (un codec, un encoding, una
    // forma di pubblicazione) e la traduzione puo' non avere il caso.
    //
    // Le enumerazioni di **lettura** — oggi solo `row_diagnostics.key_policy`
    // — pretendono un dataset con una chiave gia' presente, cioe' un fixture
    // che vive nel driver: `emit` e `redact` sono esercitate entrambe dai test
    // di `driver-shp`. Questo test non le duplica e non finge di coprirle.
    let directory = tempfile::tempdir().unwrap();
    let mut provati = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities().is_none() {
            continue;
        }
        for opzione in descriptor.format_options().opzioni {
            let ValoreAmmesso::Enumerato(ammessi) = opzione.valore else {
                continue;
            };
            if !opzione.fase.copre(FaseOpzione::Scrittura) {
                continue;
            }
            for valore in ammessi {
                provati += 1;
                let suffisso = destinazione_richiesta(opzione.chiave, valore)
                    .unwrap_or_else(|| extension(descriptor.id()));
                let destinazione = directory.path().join(format!(
                    "{}-{}-{valore}.{suffisso}",
                    descriptor.id(),
                    opzione.chiave
                ));
                if let Err(errore) = scrive_con(
                    &*driver,
                    destinazione,
                    &opzioni_scrittura_con(opzione.chiave, valore),
                ) {
                    panic!(
                        "{}: lo schema promette '{valore}' per '{}', il driver lo rifiuta: {errore}",
                        descriptor.id(), opzione.chiave
                    );
                }
            }
        }
    }
    assert!(
        provati > 0,
        "nessun valore enumerato di scrittura: il test non ha verificato niente"
    );
}

/// Il contratto del primo layer, riletto dal file appena scritto.
///
/// E' la forma di uguaglianza che `DeterminismLevel::Semantic` promette: due
/// scritture della stessa cosa possono differire nei byte, non in cio' che si
/// rilegge.
fn contratto_riletto(driver: &dyn FormatDriver, path: &std::path::Path) -> String {
    let dataset = driver
        .open(
            Source::Path(path.to_path_buf()),
            read_options(driver.descriptor().id()),
        )
        .unwrap_or_else(|error| panic!("rilettura di {}: {error}", path.display()));
    let layer = &dataset.layers()[0];
    let campi: Vec<String> = layer
        .contract
        .schema
        .fields()
        .iter()
        .map(|campo| {
            format!(
                "{}:{:?}:{}",
                campo.name(),
                campo.data_type(),
                campo.is_nullable()
            )
        })
        .collect();
    let geometria = layer.contract.geometry.as_ref().map(|g| {
        format!(
            "{}/{:?}/{:?}/{:?}",
            g.name, g.encoding, g.dimensions, g.spatial_semantics
        )
    });
    format!("{campi:?}|{geometria:?}")
}

#[test]
fn il_default_dichiarato_e_quello_applicato() {
    // Un default dichiarato che il driver non applica e' peggio di un default
    // non dichiarato: promette un comportamento e ne produce un altro.
    //
    // La prima stesura confrontava i **byte** dei due file. Era sbagliato per
    // contratto: tutti e dieci i driver dichiarano
    // `write_determinism: Semantic`, non `ByteForByte`, quindi nessuno promette
    // che due scritture della stessa cosa diano gli stessi byte. Passava per
    // fortuna — finche' due scritture XLSX non sono cadute a cavallo di un
    // secondo, e i timestamp dentro il contenitore ZIP le hanno rese diverse.
    // Un test che pretende una garanzia che nessuno ha fatto non e' severo:
    // e' rumoroso, e prima o poi viene messo a tacere.
    //
    // Il confronto e' ora sul **contratto riletto**: si riaprono i due file e
    // si confrontano schema e geometria del layer. E' cio' che il livello
    // `Semantic` promette, ed e' discriminante per le opzioni che governano la
    // forma dell'uscita — `geometry_encoding` fra `wkt` e `xy` cambia le
    // colonne, `delimiter` cambia come si rileggono.
    //
    // Cosa **non** copre: un'opzione che governi una proprieta' invisibile al
    // contratto. Oggi e' `compression` di GeoParquet, coperta a parte dai test
    // del driver, che leggono il codec dal footer. Detto qui perche' un test
    // trasversale che tace su cio' che non vede e' peggio di uno che lo dice.
    let directory = tempfile::tempdir().unwrap();
    let mut provati = 0;
    for driver in drivers() {
        let descriptor = driver.descriptor();
        if descriptor.write_capabilities().is_none() {
            continue;
        }
        for opzione in descriptor.format_options().opzioni {
            let Some(predefinito) = opzione.predefinito else {
                continue;
            };
            if !opzione.fase.copre(FaseOpzione::Scrittura) {
                continue;
            }
            provati += 1;
            let estensione = extension(descriptor.id());
            let omesso = directory.path().join(format!(
                "{}-{}-omesso.{estensione}",
                descriptor.id(),
                opzione.chiave
            ));
            let esplicito = directory.path().join(format!(
                "{}-{}-esplicito.{estensione}",
                descriptor.id(),
                opzione.chiave
            ));

            scrive_con(&*driver, omesso.clone(), &opzioni_scrittura()).unwrap_or_else(|error| {
                panic!(
                    "{}: scrittura senza '{}': {error}",
                    descriptor.id(),
                    opzione.chiave
                )
            });
            scrive_con(
                &*driver,
                esplicito.clone(),
                &opzioni_scrittura_con(opzione.chiave, predefinito),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{}: scrittura con '{}' = '{predefinito}': {error}",
                    descriptor.id(),
                    opzione.chiave
                )
            });

            assert_eq!(
                contratto_riletto(&*driver, &omesso),
                contratto_riletto(&*driver, &esplicito),
                "{}: omettere '{}' non equivale a dichiararlo '{predefinito}'",
                descriptor.id(),
                opzione.chiave
            );
        }
    }
    assert!(
        provati > 0,
        "nessun default di scrittura dichiarato: il test non ha verificato niente"
    );
}
