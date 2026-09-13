//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// Opzioni di lettura sul modello unificato.
///
/// Da S4.d il percorso di lettura vive interamente li': la memoria dei
/// batch e' una `InternalMemoryLease`, che esiste solo dentro un
/// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
/// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
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
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
use plenora_io_core::WriteLayer;
use plenora_io_model::contract::GeometryType;
use plenora_io_model::crs::CrsResolution;

#[test]
fn bounded_output_fails_before_exceeding_the_limit() {
    let mut output = BoundedOutput::new(Vec::new(), 3);
    output.write_all(b"ab").unwrap();
    assert!(output.write_all(b"cd").is_err());
    assert!(output.exceeded());
    assert_eq!(output.into_inner(), b"ab");
}

#[test]
fn spool_spills_to_file_without_changing_the_row() {
    let mut spool = DxfSpoolWriter::with_memory_limit(4096, 1);
    spool
        .push(DxfSpoolRow {
            geometry: Some(WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            }),
            layer: Some("layer".to_owned()),
            entity_type: Some("POINT".to_owned()),
            text: None,
        })
        .unwrap();
    let storage = spool.finish().unwrap();
    assert!(matches!(storage, DxfSpoolStorage::File(_)));
    let mut reader = storage.reader().unwrap();
    let row = reader
        .next_row(CoordinateDimensions::Xy, &WkbLimits::default())
        .unwrap();
    assert_eq!(row.layer.as_deref(), Some("layer"));
    assert_eq!(row.entity_type.as_deref(), Some("POINT"));
    let geometry = decode_wkb(row.geometry.as_deref().unwrap(), &WkbLimits::default()).unwrap();
    assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
    assert!(matches!(geometry.value, WkbValue::Point(_)));
}

fn resolved_wgs84() -> ResolvedCrs {
    ResolvedCrs::new(
        Some("EPSG:4326".to_owned()),
        CrsKind::Geographic,
        Some(WGS84_ESRI_WKT.to_owned()),
    )
}

fn wkb(value: WkbValue, dimensions: CoordinateDimensions) -> Vec<u8> {
    encode_wkb(
        &WkbGeometry {
            value,
            dimensions,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap()
}

fn request() -> ReadRequest {
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

// Un unico round-trip copre scrittura XYZ, CRS GEODATA embedded e
// rilettura: separarlo duplicherebbe la fixture e ne perderebbe la catena.
#[allow(clippy::too_many_lines)]
#[test]
fn write_then_read_xyz_and_embedded_crs_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.dxf");
    let wkb1 = wkb(
        WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(7.5),
            m: None,
        }),
        CoordinateDimensions::Xyz,
    );
    let wkb2 = wkb(
        WkbValue::LineString(vec![
            WkbCoordinate {
                x: 3.0,
                y: 4.0,
                z: Some(8.0),
                m: None,
            },
            WkbCoordinate {
                x: 5.0,
                y: 6.0,
                z: Some(9.0),
                m: None,
            },
        ]),
        CoordinateDimensions::Xyz,
    );
    let mut geometry_contract = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        CrsResolution::resolved(resolved_wgs84()),
        true,
    );
    geometry_contract.dimensions = CoordinateDimensions::Xyz;
    geometry_contract.set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
    let field =
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, "EPSG:4326"), &geometry_contract);
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        field,
        Field::new("val", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![
                Some(wkb1.as_slice()),
                Some(wkb2.as_slice()),
            ])),
            Arc::new(arrow_array::Int64Array::from(vec![1i64, 2])),
        ],
    )
    .unwrap();

    let driver = DxfDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry_contract),
            },
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    let planned_fidelity = w.fidelity_assessment();
    assert_eq!(
        planned_fidelity.level,
        plenora_io_core::Fidelity::Approximating
    );
    assert!(planned_fidelity
        .ragioni_v1()
        .iter()
        .any(|reason| { reason.code == plenora_io_core::FidelityReasonCode::AttributeLoss }));
    w.write(&batch).unwrap();
    let published = w.finish().unwrap();
    // "val" non è rappresentabile in DXF -> dichiarato come perdita (Approximating).
    assert!(!published.loss.is_empty());
    assert_eq!(
        published.fidelity.level,
        plenora_io_core::Fidelity::Approximating
    );
    assert!(published
        .fidelity
        .ragioni_v1()
        .iter()
        .any(|reason| reason.detail.contains("occorrenze")));

    let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    assert_eq!(
        ds.fidelity_assessment().level,
        plenora_io_core::Fidelity::Approximating
    );
    let output_geometry = ds.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xyz);
    assert_eq!(output_geometry.crs.id(), Some("EPSG:4326"));
    let mut r = ds.open_layer_reader(&request()).unwrap();
    let rb = r.next_batch().unwrap().unwrap();
    assert_eq!(rb.num_rows(), 2);
    let geometry = rb.column(0).as_any().downcast_ref::<BinaryArray>().unwrap();
    let decoded: Vec<WkbGeometry> = (0..geometry.len())
        .map(|index| decode_wkb(geometry.value(index), &WkbLimits::default()).unwrap())
        .collect();
    assert!(decoded
        .iter()
        .all(|value| value.dimensions == CoordinateDimensions::Xyz));
    assert!(decoded.iter().any(|value| matches!(
        &value.value,
        WkbValue::Point(point) if point.z == Some(7.5)
    )));
    assert!(decoded.iter().any(|value| matches!(
        &value.value,
        WkbValue::LineString(points)
            if points.first().and_then(|point| point.z) == Some(8.0)
                && points.last().and_then(|point| point.z) == Some(9.0)
    )));
}

#[test]
fn insert_block_is_exploded_and_translated() {
    // Blocco BOX = una LINE (0,0)-(1,0) su layer "0"; un INSERT lo colloca a
    // (10,5) su layer MYLAYER. L'esplosione deve tradurre la LINE a
    // (10,5)-(11,5) ed ereditare il layer dell'INSERT.
    let dxf = "\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nBOX\n10\n0.0\n20\n0.0\n30\n0.0\n\
0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n1.0\n21\n0.0\n31\n0.0\n\
0\nENDBLK\n\
0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n8\nMYLAYER\n2\nBOX\n10\n10.0\n20\n5.0\n30\n0.0\n\
0\nENDSEC\n0\nEOF\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blk.dxf");
    std::fs::write(&path, dxf).unwrap();
    let drawing = Drawing::load_file(&path).unwrap();
    let (batch, _loss, _contract) =
        build_batch(&drawing, resolved_wgs84(), DxfQuote::predefinite()).unwrap();

    let geom = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let layers = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let types = batch
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let mut found = false;
    for i in 0..batch.num_rows() {
        if types.value(i) == "LINE" {
            if let WkbValue::LineString(points) = decode_wkb(geom.value(i), &WkbLimits::default())
                .unwrap()
                .value
            {
                assert!((points[0].x - 10.0).abs() < 1e-6 && (points[0].y - 5.0).abs() < 1e-6);
                assert!((points[1].x - 11.0).abs() < 1e-6 && (points[1].y - 5.0).abs() < 1e-6);
                assert_eq!(layers.value(i), "MYLAYER");
                found = true;
            }
        }
    }
    assert!(found, "la LINE del blocco esploso non è stata trovata");
}

#[test]
fn missing_geodata_requires_explicit_assumption() {
    let drawing = Drawing::new();
    let error = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap_err();
    assert!(error.to_string().contains("assume-crs"));

    let resolved =
        resolve_dxf_crs(&drawing, &opzioni_lettura().with_assume_crs("EPSG:3857")).unwrap();
    assert_eq!(resolved.id.as_deref(), Some("EPSG:3857"));
}

#[test]
fn geodata_epsg_is_resolved_without_fallback() {
    let mut drawing = Drawing::new();
    drawing.add_object(Object::new(ObjectType::GeoData(GeoData {
        coordinate_system_definition: WGS84_ESRI_WKT.to_owned(),
        ..Default::default()
    })));
    let resolved = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap();
    assert_eq!(resolved.id.as_deref(), Some("EPSG:4326"));
    assert_eq!(resolved.kind, CrsKind::Geographic);
    assert_eq!(resolved.definition.as_deref(), Some(WGS84_ESRI_WKT));
}

#[test]
fn unresolved_geodata_is_preserved_in_typed_error() {
    let definition = "LOCAL_CS[\"survey-grid-secret\"]";
    let mut drawing = Drawing::new();
    drawing.add_object(Object::new(ObjectType::GeoData(GeoData {
        coordinate_system_definition: definition.to_owned(),
        ..Default::default()
    })));

    let error = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap_err();
    assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
    assert_eq!(error.driver.as_deref(), Some("dxf"));
    assert!(!error.to_string().contains("survey-grid-secret"));
}

#[test]
fn fuzz_entrypoint_accepts_minimal_ascii_dxf() {
    let dxf = b"0\nSECTION\n2\nENTITIES\n0\nPOINT\n10\n1\n20\n2\n30\n3\n0\nENDSEC\n0\nEOF\n";
    assert_eq!(__fuzz_read_dxf(dxf).unwrap(), 1);
}

/// Un `BLOCK` che non arriva mai a `ENDBLK` finisce, invece di non finire.
///
/// Undici righe tenevano il lettore occupato per sempre. `Entity::read`
/// restituisce `Ok(None)` senza consumare niente quando trova `0/ENDSEC`,
/// e il ciclo di `read_block` riprendeva la stessa coppia all'infinito,
/// allocando a ogni giro: non lentezza, lavoro senza fine, su un ingresso
/// che chiunque puo' fabbricare.
///
/// L'ha trovato la fuzz smoke -- e l'ha trovato solo dopo che il job ha
/// ricominciato a costruire tutti i target. Qui la stessa cosa e' una prova
/// che costa millisecondi e non dipende da quale input il fuzzer peschi.
///
/// La prova e' che **ritorni**: `assert` sull'esito verrebbe dopo, e se il
/// difetto tornasse questa prova non fallirebbe -- resterebbe appesa. E' il
/// motivo per cui il seme versionato in `fuzz/seeds/dxf_reader/` le sta
/// accanto: li' il tetto di libFuzzer trasforma l'attesa in un rosso.
#[test]
fn un_blocco_senza_endblk_non_gira_a_vuoto() {
    let dxf = b"0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nsenza-endblk\n0\nENDSEC\n0\nEOF\n";
    assert!(
        __fuzz_read_dxf(dxf).is_err(),
        "un BLOCK non terminato non e' un documento leggibile"
    );
}

#[test]
fn row_level_dxf_failure_reports_the_top_level_entity_index() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid-row.dxf");
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::ModelPoint(ModelPoint {
        location: DxfPoint::new(1.0, 2.0, 3.0),
        ..Default::default()
    })));
    drawing.add_entity(Entity::new(EntityType::Circle(
        dxf::entities::Circle::default(),
    )));
    let mut file = File::create(&path).unwrap();
    drawing.save(&mut file).unwrap();

    let error = DxfDriver
        .open(
            Source::Path(path),
            opzioni_lettura().with_assume_crs("EPSG:4326"),
        )
        .err()
        .expect("il CIRCLE degenere deve essere rifiutato");
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 1);
    assert_eq!(diagnostics.counts["dxf.entity_not_representable"], 1);
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn read_limits_are_enforced_before_batch_creation() {
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::ModelPoint(ModelPoint {
        location: DxfPoint::new(1.0, 2.0, 3.0),
        ..Default::default()
    })));
    let row_error = build_batch(
        &drawing,
        resolved_wgs84(),
        DxfQuote {
            righe: 0,
            ..DxfQuote::predefinite()
        },
    )
    .unwrap_err();
    assert_eq!(row_error.code, plenora_io_model::IoErrorCode::LimitExceeded);

    let column_error = build_batch(
        &drawing,
        resolved_wgs84(),
        DxfQuote {
            colonne: 3,
            ..DxfQuote::predefinite()
        },
    )
    .unwrap_err();
    assert_eq!(
        column_error.code,
        plenora_io_model::IoErrorCode::LimitExceeded
    );
}

#[test]
fn unsupported_m_is_rejected_before_output_creation() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("m.dxf");
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        CrsResolution::resolved(resolved_wgs84()),
        true,
    );
    geometry.dimensions = CoordinateDimensions::Xym;
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, "EPSG:4326"),
        &geometry,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "m".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };

    assert!(DxfDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .is_err());
    assert!(!output.exists());
}

#[test]
fn dxf_conversion_rejects_non_finite_coordinates() {
    for coordinate in [
        WkbCoordinate {
            x: f64::NAN,
            y: 0.0,
            z: None,
            m: None,
        },
        WkbCoordinate {
            x: 0.0,
            y: f64::INFINITY,
            z: None,
            m: None,
        },
        WkbCoordinate {
            x: 0.0,
            y: 0.0,
            z: Some(f64::NEG_INFINITY),
            m: None,
        },
    ] {
        let error = point_entity(&coordinate).unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert!(error.message.contains("coordinate non finite"));
    }
}

#[test]
fn declared_input_total_enables_dxf_specific_row_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("null.dxf");
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        CrsResolution::resolved(resolved_wgs84()),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, "EPSG:4326"),
        &geometry,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "null".to_owned(),
            contract: DataContract {
                schema: schema.clone(),
                geometry: Some(geometry),
            },
        }],
    };
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![None::<&[u8]>]))],
    )
    .unwrap();
    let mut writer = DxfDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.declare_input_total(LayerId(0), 1).unwrap();

    let error = writer.write(&batch).unwrap_err();
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.input_total, Some(1));
    assert_eq!(diagnostics.observed_total, 1);
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(
        diagnostics.counts.get("dxf.null_geometry_unsupported"),
        Some(&1)
    );
    assert!(diagnostics.validate().is_ok());
    assert!(!output.exists());
}

#[test]
fn writer_adapter_attributes_non_finite_dxf_row_and_prevents_publish() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("nan.dxf");
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        CrsResolution::resolved(resolved_wgs84()),
        false,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, "EPSG:4326"),
        &geometry,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "nan".to_owned(),
            contract: DataContract {
                schema: schema.clone(),
                geometry: Some(geometry),
            },
        }],
    };
    let bytes = encode_wkb(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: f64::NAN,
                y: 1.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]))],
    )
    .unwrap();
    let mut writer = DxfDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.declare_input_total(LayerId(0), 1).unwrap();

    let error = writer.write(&batch).unwrap_err();
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(diagnostics.counts["dxf.non_finite_coordinate"], 1);
    assert!(diagnostics.validate().is_ok());
    assert!(writer.finish().is_err());
    assert!(!output.exists());
}

fn dxf_writer_for_geometry_type(
    output: &std::path::Path,
    geometry_type: GeometryType,
    input_total: u64,
) -> (Box<dyn FormatWriter>, SchemaRef) {
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        CrsResolution::resolved(resolved_wgs84()),
        false,
    );
    geometry.set_exact_geometry_types(vec![geometry_type]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, "EPSG:4326"),
        &geometry,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "geometry".to_owned(),
            contract: DataContract {
                schema: schema.clone(),
                geometry: Some(geometry),
            },
        }],
    };
    let mut writer = DxfDriver
        .create(
            Sink::Path(output.to_path_buf()),
            &plan,
            &opzioni_scrittura(),
        )
        .unwrap();
    writer.declare_input_total(LayerId(0), input_total).unwrap();
    (writer, schema)
}

#[test]
fn writer_adapter_rejects_empty_multipart_and_collection_without_publish() {
    let point = || WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    for (name, geometry_type, valid_value, empty_value) in [
        (
            "multipoint",
            GeometryType::MultiPoint,
            WkbValue::MultiPoint(vec![point()]),
            WkbValue::MultiPoint(Vec::new()),
        ),
        (
            "collection",
            GeometryType::GeometryCollection,
            WkbValue::GeometryCollection(vec![point()]),
            WkbValue::GeometryCollection(Vec::new()),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join(format!("{name}.dxf"));
        let (mut writer, schema) = dxf_writer_for_geometry_type(&output, geometry_type, 2);
        let valid = wkb(valid_value, CoordinateDimensions::Xy);
        let valid_batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(valid.as_slice())]))],
        )
        .unwrap();
        writer.write(&valid_batch).unwrap();
        let empty = wkb(empty_value, CoordinateDimensions::Xy);
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![Some(empty.as_slice())]))],
        )
        .unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.examples.len(), 1);
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(diagnostics.counts["dxf.empty_geometry_unsupported"], 1);
        assert_eq!(diagnostics.input_total, Some(2));
        assert_eq!(
            diagnostics
                .write_outcome
                .as_ref()
                .unwrap()
                .certainly_rejected,
            plenora_io_model::KnownOrUnknownCount::Known { value: 1 }
        );
        assert!(diagnostics.validate().is_ok());
        assert!(writer.write(&batch).is_err(), "poison deve essere sticky");
        assert!(writer.finish().is_err());
        assert!(!output.exists());
    }
}

#[test]
fn empty_supported_collections_and_nested_empty_use_the_stable_cause() {
    let empty = "dxf.empty_geometry_unsupported";
    assert_eq!(
        dxf_geometry_rejection_cause(&WkbValue::MultiLineString(Vec::new())),
        Some(empty)
    );
    assert_eq!(
        dxf_geometry_rejection_cause(&WkbValue::MultiPolygon(Vec::new())),
        Some(empty)
    );
    assert_eq!(
        dxf_geometry_rejection_cause(&WkbValue::GeometryCollection(vec![WkbGeometry {
            value: WkbValue::MultiPoint(Vec::new()),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        }])),
        Some(empty)
    );
    assert_eq!(
        dxf_geometry_rejection_cause(&WkbValue::CompoundCurve(Vec::new())),
        Some("dxf.geometry_type_unsupported")
    );
}

#[test]
fn non_empty_multipoint_preserves_explosion_fidelity_and_entities() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("multipoint.dxf");
    let (mut writer, schema) = dxf_writer_for_geometry_type(&output, GeometryType::MultiPoint, 1);
    let bytes = wkb(
        WkbValue::MultiPoint(vec![
            WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 3.0,
                    y: 4.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
        ]),
        CoordinateDimensions::Xy,
    );
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]))],
    )
    .unwrap();
    writer.write(&batch).unwrap();
    let published = writer.finish().unwrap();
    assert_eq!(published.loss.counts["MultiPoint esploso in entità DXF"], 2);

    let dataset = DxfDriver
        .open(Source::Path(output), opzioni_lettura())
        .unwrap();
    let mut reader = dataset.open_layer_reader(&request()).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 2);
}

#[test]
fn missing_geometry_contract_is_rejected_before_output_creation() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("missing-crs.dxf");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "empty".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::empty()),
                geometry: None,
            },
        }],
    };
    assert!(DxfDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .is_err());
    assert!(!output.exists());
}
