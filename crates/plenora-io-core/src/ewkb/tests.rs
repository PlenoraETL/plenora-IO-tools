//! Le prove di [`super`]: la conversione e' esatta, e il SRID non si perde in
//! silenzio.

use super::*;

use arrow_array::BinaryArray;
use plenora_io_model::contract::{CoordinateDimensions, FieldId};
use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{encode_wkb, WkbCoordinate};

use crate::descriptor::FormatDescriptor;

fn limiti() -> WkbLimits {
    plenora_io_model::budget::PipelineLimits::default().wkb_limits()
}

fn coordinata(x: f64, y: f64) -> WkbCoordinate {
    WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    }
}

/// Un poligono con buco e un multipunto in un collection, con il SRID su ogni
/// livello come lo scrive `PostGIS` quando lo ripete.
fn geometria(srid: Option<i32>) -> WkbGeometry {
    let poligono = WkbGeometry {
        value: WkbValue::Polygon(vec![
            vec![
                coordinata(0.1, 0.2),
                coordinata(10.000_000_000_000_002, 0.2),
                coordinata(10.0, 10.0),
                coordinata(0.1, 0.2),
            ],
            vec![
                coordinata(1.0, 1.0),
                coordinata(2.0, 1.0),
                coordinata(2.0, 2.0),
                coordinata(1.0, 1.0),
            ],
        ]),
        dimensions: CoordinateDimensions::Xy,
        srid,
    };
    let punti = WkbGeometry {
        value: WkbValue::MultiPoint(vec![WkbGeometry {
            value: WkbValue::Point(coordinata(f64::MIN_POSITIVE, -0.0)),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        }]),
        dimensions: CoordinateDimensions::Xy,
        srid,
    };
    WkbGeometry {
        value: WkbValue::GeometryCollection(vec![poligono, punti]),
        dimensions: CoordinateDimensions::Xy,
        srid,
    }
}

fn contratto(crs_id: Option<&str>, srid: Option<i32>) -> GeometryColumnContract {
    let crs = crs_id.map_or(CrsResolution::Missing, |id| {
        CrsResolution::Resolved(ResolvedCrs::new(
            Some(id.to_owned()),
            CrsKind::Geographic,
            None,
        ))
    });
    let mut contratto = GeometryColumnContract::wkb_xy(FieldId(0), "geom", crs, true);
    contratto.encoding = GeometryEncoding::Ewkb;
    contratto.srid = srid;
    contratto
}

fn schema() -> SchemaRef {
    let metadati: HashMap<String, String> = [
        (PLENORA_ENCODING_KEY.to_owned(), "ewkb".to_owned()),
        (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
        ("plenora.geometry.crs_id".to_owned(), "EPSG:4326".to_owned()),
    ]
    .into_iter()
    .collect();
    Arc::new(Schema::new(vec![Field::new(
        "geom",
        DataType::Binary,
        true,
    )
    .with_metadata(metadati)]))
}

/// Un sink di prova: scrive la geometria come gli dice `geometry`.
fn descrittore(geometry: crate::descriptor::GeometryWriteSupport) -> FormatDescriptor {
    FormatDescriptor::const_new(
        "test",
        crate::descriptor::Direction::Bidirectional,
        crate::descriptor::ReadMode::StreamingSequential,
        // I tre assi di INV-7: il descrittore di prova dichiara la
        // combinazione che tutti i driver reali dichiarano.
        crate::descriptor::NativeReadMode::StreamingSequential,
        crate::descriptor::DeliverySemantics::OperationAtomic,
        crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
        crate::descriptor::DeterminismLevel::Semantic,
        Some(crate::descriptor::WriteMode::Streaming),
        Some(crate::descriptor::DeterminismLevel::Semantic),
        false,
        false,
        crate::descriptor::ReaderConcurrency::MultipleIndependentReaders,
        crate::descriptor::ProjectionSupport::None,
        crate::descriptor::PredicatePruningSupport::None,
        crate::descriptor::SpatialPruningSupport::None,
        crate::descriptor::CrsHandling::Embedded,
        crate::descriptor::Fidelity::Conditional,
        crate::descriptor::Runtime::PureRust,
        // `hostile_input_hardened`: un descrittore di prova non parla di
        // input ostile: dichiara il valore che non afferma niente.
        false,
        // `spec_version_supported`: un descrittore di prova non parla di
        // nessun formato reale, quindi non ne dichiara la versione.
        None,
        Some(crate::descriptor::FormatWriteCapabilities {
            field_names: crate::descriptor::DBF_FIELD_NAMES,
            allowed_types: crate::descriptor::SCALAR_TYPES,
            type_coercion: crate::descriptor::TypeCoercionPolicy::Reject,
            attributes: crate::descriptor::AttributeWriteSupport::All,
            geometry,
            crs: crate::descriptor::CrsWriteSupport::Embedded,
            crs_representations: crate::descriptor::CrsRepresentationCapabilities::new(
                crate::descriptor::CrsRepresentationState::Preserved,
                crate::descriptor::CrsRepresentationState::Preserved,
                crate::descriptor::CrsRepresentationState::Preserved,
            ),
            nullability: crate::descriptor::NullabilitySupport::Preserve,
            multi_layer: false,
            sink_path: crate::SinkPathConstraint::Free,
        }),
        plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
        &["test"],
        1,
        1,
        1,
    )
}

fn gpkg() -> FormatDescriptor {
    descrittore(crate::descriptor::WKB_PASSTHROUGH_GEOMETRY)
}

#[test]
fn la_conversione_e_esatta_bit_per_bit() {
    let (schema_wkb, contratto_wkb, conversione) = pianifica(
        &gpkg(),
        &schema(),
        Some(&contratto(Some("EPSG:4326"), Some(4326))),
        limiti(),
    )
    .expect("pianificabile")
    .expect("da convertire");
    assert_eq!(contratto_wkb.encoding, GeometryEncoding::Wkb);
    assert_eq!(contratto_wkb.srid, None);
    let campo = schema_wkb.field(0);
    assert_eq!(
        campo
            .metadata()
            .get(PLENORA_ENCODING_KEY)
            .map(String::as_str),
        Some("wkb")
    );
    assert!(campo.metadata().get(PLENORA_SRID_KEY).is_none());
    assert_eq!(
        campo
            .metadata()
            .get("plenora.geometry.crs_id")
            .map(String::as_str),
        Some("EPSG:4326"),
        "le altre chiavi restano"
    );

    let ewkb = encode_wkb(&geometria(Some(4326)), WkbFlavor::Ewkb).expect("ewkb");
    let batch = RecordBatch::try_new(
        schema(),
        vec![Arc::new(BinaryArray::from(vec![
            Some(ewkb.as_slice()),
            None,
        ]))],
    )
    .expect("batch");
    let convertito = conversione.converti(&batch).expect("convertito");
    let colonna = convertito
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .expect("binaria");
    let atteso = encode_wkb(&geometria(None), WkbFlavor::Iso).expect("iso");
    assert_eq!(
        colonna.value(0),
        atteso.as_slice(),
        "WKB ISO delle stesse coordinate"
    );
    assert!(colonna.is_null(1), "un null resta null");
    // Il giro inverso: decodificato, e' la stessa geometria senza SRID, con
    // -0.0 e il subnormale intatti.
    assert_eq!(
        decode_wkb(colonna.value(0), &limiti()).expect("iso"),
        geometria(None)
    );
}

#[test]
fn un_srid_diverso_dal_crs_e_un_errore() {
    let errore = pianifica(
        &gpkg(),
        &schema(),
        Some(&contratto(Some("EPSG:3857"), Some(4326))),
        limiti(),
    )
    .expect_err("SRID e CRS discordi");
    assert!(
        errore.to_string().contains("SRID EWKB diverso dal CRS"),
        "{errore}"
    );
}

#[test]
fn un_srid_senza_crs_che_lo_rappresenti_e_un_errore() {
    let mut senza = contratto(None, Some(4326));
    senza.crs = CrsResolution::DeclaredButUnresolved(plenora_io_model::crs::RawCrs {
        definition: None,
        authority_hint: None,
        definition_format: None,
        axis_order: plenora_io_model::crs::AxisOrder::Unknown,
    });
    let errore = pianifica(&gpkg(), &schema(), Some(&senza), limiti()).expect_err("andrebbe perso");
    assert!(
        errore
            .to_string()
            .contains("senza un CRS che lo rappresenti"),
        "{errore}"
    );
}

#[test]
fn senza_srid_si_converte_anche_senza_crs() {
    assert!(
        pianifica(&gpkg(), &schema(), Some(&contratto(None, None)), limiti())
            .expect("pianificabile")
            .is_some()
    );
}

#[test]
fn un_payload_con_un_altro_srid_e_un_errore() {
    let (_, _, conversione) = pianifica(
        &gpkg(),
        &schema(),
        Some(&contratto(Some("EPSG:4326"), Some(4326))),
        limiti(),
    )
    .expect("pianificabile")
    .expect("da convertire");
    let altro = encode_wkb(&geometria(Some(3003)), WkbFlavor::Ewkb).expect("ewkb");
    let batch = RecordBatch::try_new(
        schema(),
        vec![Arc::new(BinaryArray::from(vec![Some(altro.as_slice())]))],
    )
    .expect("batch");
    let errore = conversione.converti(&batch).expect_err("SRID del payload");
    assert!(
        errore.to_string().contains("SRID del payload EWKB"),
        "{errore}"
    );
}

#[test]
fn un_sink_che_scrive_ewkb_non_converte() {
    let ipc = descrittore(crate::descriptor::WKB_EWKB_PASSTHROUGH_GEOMETRY);
    assert!(pianifica(
        &ipc,
        &schema(),
        Some(&contratto(Some("EPSG:4326"), Some(4326))),
        limiti()
    )
    .expect("pianificabile")
    .is_none());
}

#[test]
fn un_contratto_wkb_non_converte() {
    let mut wkb = contratto(Some("EPSG:4326"), None);
    wkb.encoding = GeometryEncoding::Wkb;
    assert!(pianifica(&gpkg(), &schema(), Some(&wkb), limiti())
        .expect("pianificabile")
        .is_none());
}
