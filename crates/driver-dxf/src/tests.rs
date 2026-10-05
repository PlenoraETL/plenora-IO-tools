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

/// Il caso del soak di `dxf_reader` del 2026-10-05, byte per byte.
///
/// 235 byte che sulla 4.0.0 e sulla 4.1.0 finivano in un'allocazione da sei
/// gigabyte e in un abort senza busta. `SectionTypeSettings::read`, su un
/// codice che non e' suo, rimetteva la coppia nell'iteratore e restituiva
/// `Some` di un valore vuoto senza consumare niente; il ciclo di
/// `apply_custom_reader_sectionsettings` la richiamava sulla stessa coppia
/// all'infinito, spingendo un elemento a ogni giro.
///
/// Come per `un_blocco_senza_endblk_non_gira_a_vuoto`, la prova e' che la
/// chiamata **ritorni**; il seme versionato in `fuzz/seeds/dxf_reader/` le sta
/// accanto perche' li' il tetto di libFuzzer trasforma un ritorno mancato in un
/// rosso, e la prova della CLI in `crates/plenora-io-tools/tests/` la ripete
/// sul binario con un tempo massimo.
#[test]
fn il_caso_del_soak_sectionsettings_ritorna_con_un_errore() {
    let caso = include_bytes!("../../../fuzz/seeds/dxf_reader/sectionsettings-senza-progresso.dxf");
    assert_eq!(
        caso.len(),
        235,
        "il seme e' il caso originale, non una riduzione"
    );
    assert!(__fuzz_read_dxf(caso).is_err());
}

/// La forma minima della stessa classe: un `SectionTypeSettings` aperto da
/// `1` e seguito da un codice che nessuno dei suoi campi conosce.
///
/// Prova la guardia sul ciclo di `apply_custom_reader_sectionsettings` senza
/// dipendere dai byte del fuzzer, e su **entrambi** i lettori: quello completo
/// (`Drawing::load`) e quello progressivo che usa la CLI, dove la sezione
/// `OBJECTS` viene letta prima di `ENTITIES`.
#[test]
fn un_section_type_settings_che_non_consuma_si_rifiuta() {
    let dxf = b"0\nSECTION\n2\nOBJECTS\n0\nSECTIONSETTINGS\n1\nSectionTypeSettings\n72\n0\n0\nENDSEC\n0\nEOF\n";
    assert!(
        __fuzz_read_dxf(dxf).is_err(),
        "lettore completo: un giro senza progresso e' un errore"
    );
    let progressivo = dxf::DrawingEntityReader::load(std::io::Cursor::new(dxf.to_vec()));
    assert!(
        progressivo.is_err(),
        "lettore progressivo: un giro senza progresso e' un errore"
    );
}

/// Le guardie dei due cicli di `SectionSettings` non scattano su un documento
/// legittimo: un `SectionTypeSettings` con due `SectionGeometrySettings`,
/// scritto dal fork stesso e riletto.
///
/// `SectionGeometrySettings::read` non restituisce mai `Some` senza consumare
/// -- lo apre la coppia `90`, che legge -- e una prova ostile per lei non si
/// puo' costruire. Si prova invece che la guardia preventiva sul suo ciclo non
/// rifiuti cio' che va letto, che e' il modo in cui una guardia sbaglia.
///
/// Un solo `SectionTypeSettings` e non due: il lettore upstream legge la `1`
/// che apre il secondo come nome del file del primo, e li fonde. E' un difetto
/// di semantica del lettore, indipendente dalle guardie -- misurato sul fork
/// senza la correzione, con lo stesso esito -- e questa prova non lo fissa.
#[test]
fn le_guardie_di_sectionsettings_lasciano_passare_un_documento_valido() {
    use dxf::objects::SectionSettings;
    use dxf::{SectionGeometrySettings, SectionTypeSettings};

    let geometria = |tipo: i32| SectionGeometrySettings {
        section_type: tipo,
        geometry_count: 1,
        ..Default::default()
    };
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2018;
    drawing.add_object(Object::new(ObjectType::SectionSettings(SectionSettings {
        section_type: 1,
        geometry_settings: vec![SectionTypeSettings {
            section_type: 1,
            geometry_settings: vec![geometria(1), geometria(2)],
            ..Default::default()
        }],
        ..Default::default()
    })));
    let mut byte = Vec::new();
    drawing.save(&mut byte).unwrap();

    let riletto = Drawing::load(&mut std::io::Cursor::new(byte)).unwrap();
    let lette: Vec<_> = riletto
        .objects()
        .filter_map(|o| match o.specific {
            ObjectType::SectionSettings(ref ss) => Some(ss.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(lette.len(), 1);
    assert_eq!(lette[0].geometry_settings.len(), 1);
    let tipi: Vec<i32> = lette[0].geometry_settings[0]
        .geometry_settings
        .iter()
        .map(|g| g.section_type)
        .collect();
    assert_eq!(tipi, [1, 2]);
}

/// `TableCellStyle::read` consuma sempre la coppia `7` da cui parte, ma il
/// ciclo di `apply_custom_reader_tablestyle` che la rimette indietro e la
/// richiama non ha modo di saperlo: la guardia lo pretende. Anche qui la prova
/// ostile non esiste, e si prova che la guardia non rifiuti tre stili validi.
///
/// Scritto a mano e non dal fork: `TableCellStyle::read` consuma la coppia `0`
/// che chiude l'oggetto invece di rimetterla indietro, e un `TABLESTYLE` il cui
/// ultimo stile arriva fino alla `0` non si rilegge -- anche senza la
/// correzione. Qui una coppia `40` chiude l'ultimo stile prima della `0`.
#[test]
fn la_guardia_di_tablestyle_lascia_passare_un_documento_valido() {
    let dxf = b"0\nSECTION\n2\nOBJECTS\n0\nTABLESTYLE\n7\nA\n140\n2.5\n7\nB\n140\n2.5\n7\nC\n140\n2.5\n40\n1.0\n0\nENDSEC\n0\nEOF\n";
    let riletto = Drawing::load(&mut std::io::Cursor::new(dxf.to_vec())).unwrap();
    let nomi: Vec<String> = riletto
        .objects()
        .filter_map(|o| match o.specific {
            dxf::objects::ObjectType::TableStyle(ref ts) => Some(ts.cell_styles.clone()),
            _ => None,
        })
        .flatten()
        .map(|s| s.name)
        .collect();
    assert_eq!(nomi, ["A", "B", "C"]);
}

/// Una `LINE` con `livelli` gruppi annidati: `102/{...}` oppure, in XDATA,
/// `1002/{`.
fn linea_con_gruppi_annidati(livelli: usize, xdata: bool) -> Vec<u8> {
    let mut testo = String::from("0\nSECTION\n2\nENTITIES\n0\nLINE\n");
    if xdata {
        testo.push_str("10\n0\n20\n0\n11\n1\n21\n1\n1001\nAPP\n");
        testo.push_str(&"1002\n{\n".repeat(livelli));
        testo.push_str(&"1002\n}\n".repeat(livelli));
    } else {
        testo.push_str(&"102\n{a\n".repeat(livelli));
        testo.push_str(&"102\n}\n".repeat(livelli));
        testo.push_str("10\n0\n20\n0\n11\n1\n21\n1\n");
    }
    testo.push_str("0\nENDSEC\n0\nEOF\n");
    testo.into_bytes()
}

/// Lo stack del thread principale di Windows: il piu' piccolo su cui la CLI
/// legge un DXF. Le prove sui gruppi annidati girano in un thread di questa
/// misura, cosi' che un'eventuale crescita del consumo per livello si veda qui
/// e non in produzione.
const STACK_DEL_THREAD_PRINCIPALE_WINDOWS: usize = 1024 * 1024;

fn in_uno_stack_da_un_mebibyte<T: Send + 'static>(prova: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK_DEL_THREAD_PRINCIPALE_WINDOWS)
        .spawn(prova)
        .expect("il thread di prova parte")
        .join()
        .expect("la prova non trabocca lo stack")
}

/// I gruppi annidati si leggono per ricorsione, e senza tetto un ingresso di
/// pochi megabyte esauriva lo stack: un abort senza busta, della stessa
/// famiglia del caso del soak -- un ingresso ostile che esaurisce una risorsa
/// invece di essere rifiutato. Sessantamila livelli stanno sotto il tetto di
/// byte dell'entry point di fuzz, e senza la correzione bastano a far
/// traboccare lo stack.
#[test]
fn i_gruppi_annidati_oltre_il_tetto_si_rifiutano_invece_di_esaurire_lo_stack() {
    in_uno_stack_da_un_mebibyte(|| {
        for xdata in [false, true] {
            let dxf = linea_con_gruppi_annidati(60_000, xdata);
            assert!(
                dxf.len() < 1_048_576,
                "sotto il tetto dell'entry point di fuzz"
            );
            assert!(__fuzz_read_dxf(&dxf).is_err(), "xdata={xdata}");
        }
    });
}

/// Il tetto e' un confine, non un divieto: 256 livelli si leggono, 257 no. E si
/// leggono in un MiB di stack, quello del thread principale di Windows, anche
/// in una build non ottimizzata.
#[test]
fn il_tetto_dei_gruppi_annidati_e_esattamente_duecentocinquantasei() {
    in_uno_stack_da_un_mebibyte(|| {
        for xdata in [false, true] {
            assert_eq!(
                __fuzz_read_dxf(&linea_con_gruppi_annidati(256, xdata)).unwrap(),
                1,
                "xdata={xdata}: 256 livelli sono leggibili"
            );
            assert!(
                __fuzz_read_dxf(&linea_con_gruppi_annidati(257, xdata)).is_err(),
                "xdata={xdata}: 257 livelli si rifiutano"
            );
        }
    });
}

/// Un BLOCK la cui LINE porta un difetto, e un INSERT che lo usa.
fn blocco_con_linea_difettosa(difetto: &str) -> Vec<u8> {
    let mut testo = String::from("0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n0\nLINE\n");
    testo.push_str(difetto);
    testo.push_str(
        "0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB\n10\n0\n20\n0\n0\nENDSEC\n0\nEOF\n",
    );
    testo.into_bytes()
}

/// Rilevato in revisione: dentro BLOCKS, `EntityIter` trasformava l'errore
/// della LINE in fine sequenza, la guardia di progresso di `read_block`
/// passava -- qualcosa era stato consumato --, e il BLOCK veniva accettato
/// **senza la LINE**: l'INSERT esplodeva in zero righe e la lettura riusciva.
/// Perdita silenziosa, e non solo per la profondita': un `10/abc` faceva lo
/// stesso, gia' sulla 4.1.0.
///
/// Ora ogni errore che `EntityIter` o `ObjectIter` inghiottono ferma
/// l'iteratore, e il documento si rifiuta. La prova e' su entrambi i lettori:
/// quello completo e quello progressivo della CLI.
#[test]
fn una_linea_illeggibile_dentro_un_blocco_rifiuta_il_documento() {
    let difetti = [
        (
            "profondita",
            format!(
                "10\n0\n20\n0\n11\n1\n21\n1\n1001\nAPP\n{}{}",
                "1002\n{\n".repeat(257),
                "1002\n}\n".repeat(257)
            ),
        ),
        ("coordinata", String::from("10\nabc\n20\n0\n11\n1\n21\n1\n")),
    ];
    for (nome, difetto) in difetti {
        let dxf = blocco_con_linea_difettosa(&difetto);
        let completo = in_uno_stack_da_un_mebibyte({
            let dxf = dxf.clone();
            move || __fuzz_read_dxf(&dxf).is_err()
        });
        assert!(
            completo,
            "{nome}: lettore completo, il BLOCK non si accetta senza la LINE"
        );

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("blocco.dxf");
        std::fs::write(&path, &dxf).unwrap();
        let aperto = in_uno_stack_da_un_mebibyte(move || {
            DxfDriver
                .open(
                    Source::Path(path),
                    opzioni_lettura().with_assume_crs("EPSG:4326"),
                )
                .is_err()
        });
        assert!(
            aperto,
            "{nome}: lettore progressivo, il BLOCK non si accetta senza la LINE"
        );
    }
}

/// Lo stesso nella sezione OBJECTS: un oggetto illeggibile interrompeva
/// `ObjectIter`, e se dopo veniva `0/ENDSEC` il documento passava senza gli
/// oggetti che seguivano -- un `GEODATA` fra loro avrebbe cambiato il CRS.
#[test]
fn un_oggetto_illeggibile_rifiuta_il_documento() {
    let dxf = b"0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nnon-esadecimale\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n0\nEOF\n";
    assert!(__fuzz_read_dxf(dxf).is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oggetto.dxf");
    std::fs::write(&path, dxf).unwrap();
    assert!(DxfDriver
        .open(
            Source::Path(path),
            opzioni_lettura().with_assume_crs("EPSG:4326"),
        )
        .is_err());
}

// --- documenti accettati con dati mancanti o alterati -------------------------
//
// La classe trovata nella seconda revisione di #23: un documento che il lettore
// accettava senza errore, con una parte persa o sostituita da un default. Ogni
// prova ha il suo controllo -- lo stesso documento senza il difetto, che si
// legge -- perche' un rifiuto per un'altra ragione la renderebbe vuota.

/// L'esito dei due lettori: quello completo (`Drawing::load`, l'entry point di
/// fuzz) e quello progressivo della CLI (`DxfDriver::open`). `Ok(righe)` o
/// `Err(())`.
fn esito_dei_due_lettori(
    dxf: &[u8],
) -> (
    std::result::Result<usize, ()>,
    std::result::Result<usize, ()>,
) {
    let completo = __fuzz_read_dxf(dxf).map_err(|_| ());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prova.dxf");
    std::fs::write(&path, dxf).unwrap();
    let progressivo = DxfDriver
        .open(
            Source::Path(path),
            opzioni_lettura().with_assume_crs("EPSG:4326"),
        )
        .map_err(|_| ())
        .and_then(|dataset| {
            let mut reader = dataset
                .open_layer_reader(&plenora_io_core::request::ReadRequest {
                    layer: LayerId(0),
                    projected_fields: None,
                    projection_mode: ProjectionMode::BestEffort,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::Complete,
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .map_err(|_| ())?;
            let mut righe = 0;
            while let Some(batch) = reader.next_batch().map_err(|_| ())? {
                righe += batch.num_rows();
            }
            Ok(righe)
        });
    (completo, progressivo)
}

fn rifiutato(nome: &str, dxf: &[u8]) {
    let (completo, progressivo) = esito_dei_due_lettori(dxf);
    assert!(
        completo.is_err(),
        "{nome}: il lettore completo accetta {completo:?}"
    );
    assert!(
        progressivo.is_err(),
        "{nome}: il lettore progressivo accetta {progressivo:?}"
    );
}

fn letto(nome: &str, dxf: &[u8], righe: usize) {
    let (completo, progressivo) = esito_dei_due_lettori(dxf);
    assert_eq!(completo, Ok(righe), "{nome}: controllo, lettore completo");
    assert_eq!(
        progressivo,
        Ok(righe),
        "{nome}: controllo, lettore progressivo"
    );
}

fn entita(corpo: &str) -> Vec<u8> {
    format!("0\nSECTION\n2\nENTITIES\n{corpo}0\nENDSEC\n0\nEOF\n").into_bytes()
}

const LINEA: &str = "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n";

/// Un tipo di entita' che il lettore non conosce era consumato e scartato, e
/// il rifiuto delle entita' non gestite del driver non veniva raggiunto: il
/// documento si leggeva senza di lei. Lo stesso per una DIMENSION senza un
/// sottotipo riconosciuto, saltata con `continue`.
#[test]
fn un_entita_di_tipo_sconosciuto_rifiuta_il_documento() {
    letto("controllo", &entita(LINEA), 1);
    rifiutato(
        "tipo sconosciuto",
        &entita(&format!("{LINEA}0\nLINEE\n8\n0\n10\n0\n20\n0\n")),
    );
    rifiutato(
        "DIMENSION senza sottotipo",
        &entita(&format!(
            "{LINEA}0\nDIMENSION\n8\n0\n100\nAcDbDimension\n70\n0\n"
        )),
    );
}

/// Dentro un BLOCK l'entita' sconosciuta conta se il blocco viene esploso: un
/// INSERT che lo usa si rifiuta, un blocco che nessuno inserisce non produce
/// righe e non cambia l'esito.
#[test]
fn un_blocco_con_un_entita_sconosciuta_si_rifiuta_quando_e_inserito() {
    let blocco = |corpo_del_blocco: &str, modello: &str| {
        format!(
            "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n{corpo_del_blocco}0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{modello}0\nENDSEC\n0\nEOF\n"
        )
        .into_bytes()
    };
    let insert = "0\nINSERT\n2\nB\n10\n0\n20\n0\n";
    letto("controllo", &blocco(LINEA, insert), 1);
    rifiutato(
        "blocco inserito",
        &blocco(&format!("{LINEA}0\nLINEE\n10\n0\n"), insert),
    );
    letto(
        "blocco non inserito",
        &blocco(&format!("{LINEA}0\nLINEE\n10\n0\n"), LINEA),
        1,
    );
}

/// Una riga di codice vuota era la fine dell'ingresso: un DXF con una riga
/// vuota in testa si leggeva come un documento vuoto. Un codice senza la riga
/// del valore era un valore vuoto.
#[test]
fn una_riga_vuota_non_e_la_fine_del_documento() {
    let documento = entita(LINEA);
    letto("controllo", &documento, 1);
    let mut con_riga_vuota = b"\n".to_vec();
    con_riga_vuota.extend_from_slice(&documento);
    rifiutato("riga vuota in testa", &con_riga_vuota);
    let mut in_mezzo = b"0\nSECTION\n2\nENTITIES\n".to_vec();
    in_mezzo.extend_from_slice(b"\n");
    in_mezzo.extend_from_slice(LINEA.as_bytes());
    in_mezzo.extend_from_slice(b"0\nENDSEC\n0\nEOF\n");
    rifiutato("riga vuota in mezzo", &in_mezzo);
    rifiutato(
        "codice senza valore",
        b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n21",
    );
}

/// `\U+ZZZZ` e un surrogato diventavano `?`; una barra negli ultimi sei
/// caratteri della riga spariva con cio' che la seguiva. Un nome di layer e un
/// testo arrivano entrambi all'uscita.
#[test]
fn le_sequenze_unicode_non_valide_rifiutano_e_le_barre_restano() {
    let linea_su_layer = |layer: &str| {
        entita(&format!(
            "0\nLINE\n8\n{layer}\n10\n0\n20\n0\n11\n1\n21\n1\n"
        ))
    };
    letto("controllo", &linea_su_layer("Rep\\U+00E8re"), 1);
    rifiutato("cifre non esadecimali", &linea_su_layer("A\\U+ZZZZ"));
    rifiutato("surrogato", &linea_su_layer("A\\U+D800"));
    rifiutato("sequenza troncata", &linea_su_layer("A\\U+00"));

    let testo = |valore: &str| {
        let dxf = entita(&format!(
            "0\nTEXT\n8\n0\n10\n0\n20\n0\n40\n1\n1\n{valore}\n"
        ));
        let drawing = Drawing::load(&mut std::io::Cursor::new(dxf)).unwrap();
        let letti: Vec<String> = drawing
            .entities()
            .filter_map(|e| match e.specific {
                EntityType::Text(ref t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        letti
    };
    assert_eq!(testo("riga\\P"), ["riga\\P"], "la barra finale resta");
    assert_eq!(
        testo("\\Pab\\U+00E8"),
        ["\\Pab\u{e8}"],
        "e non sposta la sequenza dopo"
    );
}

/// Tre X, tre Y e due Z davano due punti: il terzo spariva.
#[test]
fn le_coordinate_disallineate_di_una_spline_rifiutano_il_documento() {
    let spline = |z: &str| {
        entita(&format!(
            "0\nSPLINE\n8\n0\n70\n8\n71\n2\n72\n6\n73\n3\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n{z}10\n1\n20\n1\n10\n2\n20\n0\n"
        ))
    };
    // I 30 vanno dopo ogni 10/20: qui si scrivono tutti in coda alla prima
    // coppia, il lettore li accoda nell'ordine in cui arrivano.
    letto("controllo", &spline("30\n0\n30\n0\n30\n0\n"), 1);
    rifiutato("due Z per tre punti", &spline("30\n0\n30\n0\n"));
}

/// `NaN` e `inf` passavano il parse dei reali; una normale non finita, o
/// nulla, diventava l'asse Z.
#[test]
fn i_reali_non_finiti_e_le_normali_nulle_rifiutano_il_documento() {
    let cerchio =
        |normale: &str| entita(&format!("0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n1\n{normale}"));
    letto("controllo", &cerchio("210\n0\n220\n0\n230\n1\n"), 1);
    rifiutato("normale NaN", &cerchio("210\nNaN\n220\n0\n230\n1\n"));
    rifiutato("normale inf", &cerchio("210\ninf\n220\n0\n230\n1\n"));
    rifiutato("normale nulla", &cerchio("210\n0\n220\n0\n230\n0\n"));
    rifiutato(
        "coordinata infinita",
        &entita("0\nLINE\n10\n0\n20\n0\n11\ninfinity\n21\n1\n"),
    );
}

/// Pesi non finiti, nulli o negativi diventavano 1.0, e un vettore dei nodi
/// sbagliato faceva disegnare la poligonale di controllo al posto della curva.
#[test]
fn i_pesi_e_i_nodi_non_validi_di_una_spline_rifiutano_il_documento() {
    let spline = |nodi: &str, pesi: &str| {
        entita(&format!(
            "0\nSPLINE\n8\n0\n70\n8\n71\n2\n72\n6\n73\n3\n{nodi}{pesi}10\n0\n20\n0\n30\n0\n10\n1\n20\n1\n30\n0\n10\n2\n20\n0\n30\n0\n"
        ))
    };
    let nodi = "40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n";
    letto("controllo", &spline(nodi, ""), 1);
    letto(
        "controllo con pesi",
        &spline(nodi, "41\n1\n41\n2\n41\n1\n"),
        1,
    );
    rifiutato("peso nullo", &spline(nodi, "41\n1\n41\n0\n41\n1\n"));
    rifiutato("peso negativo", &spline(nodi, "41\n1\n41\n-2\n41\n1\n"));
    rifiutato("pesi mancanti", &spline(nodi, "41\n1\n41\n1\n"));
    rifiutato(
        "nodi mancanti",
        &spline("40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n", ""),
    );
}

/// Un fattore di scala nullo diventava 1, e un INSERT con attributi e senza
/// blocco passava con il solo punto timbro.
#[test]
fn un_insert_con_scala_nulla_o_senza_blocco_rifiuta_il_documento() {
    let documento = |scala: &str, nome: &str| {
        format!(
            "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n{LINEA}0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\n{nome}\n10\n0\n20\n0\n{scala}0\nENDSEC\n0\nEOF\n"
        )
        .into_bytes()
    };
    letto("controllo", &documento("41\n2\n", "B"), 1);
    rifiutato("scala X nulla", &documento("41\n0\n", "B"));
    let attributo = "66\n1\n0\nATTRIB\n8\n0\n10\n0\n20\n0\n40\n1\n1\nv\n2\nTAG\n0\nSEQEND\n";
    rifiutato("blocco assente con attributi", &{
        let mut d = documento("", "ASSENTE");
        let testo = String::from_utf8(d.clone()).unwrap();
        d = testo
            .replace(
                "10\n0\n20\n0\n0\nENDSEC\n0\nEOF",
                &format!("10\n0\n20\n0\n{attributo}0\nENDSEC\n0\nEOF"),
            )
            .into_bytes();
        d
    });
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

/// Il `decode_wkb` del writer DXF dichiara `Write`, e nient'altro cambia.
///
/// # Perche' lo stato del writer si costruisce a mano
///
/// Perche' `create` lo avvolge con `with_write_validation`, che ispeziona la
/// geometria e registra una **violazione di riga** invece di propagare: per la
/// via normale un WKB illeggibile viene respinto li'. Davanti a questa
/// decodifica ci sono due strati -- la validazione in lettura e quella in
/// scrittura -- e il ramo e' percio' **difensivo**.
///
/// Difensivo non vuol dire esente: se ci si arriva, la fase dev'essere quella
/// in corso. Costruire lo stato come fa `create`, ma senza la guardia, e' il
/// modo di raggiungere il ramo davvero invece di dichiararlo corretto e basta.
#[test]
fn la_decodifica_difensiva_del_writer_dichiara_la_fase_di_scrittura() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("guasto.dxf");
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
    // Byte order valido, tipo 99 che non esiste.
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(
            &[0x01_u8, 0x63, 0x00, 0x00, 0x00][..],
        )]))],
    )
    .unwrap();

    let mut sotto_prova = DxfWriterState {
        drawing: Drawing::new(),
        path: output,
        durable: false,
        loss: LossReport::default(),
        dropped_cols: Vec::new(),
        rows: 0,
        input_total: None,
        first: true,
        wkb_limits: WkbLimits::default(),
        max_output_bytes: u64::MAX,
    };

    let errore = sotto_prova
        .write(&batch)
        .expect_err("un WKB illeggibile non si scrive");

    assert_eq!(
        errore.phase,
        plenora_io_model::ErrorPhase::Write,
        "la scrittura e' in corso: valeva `validate` finche' la fase la          dichiarava l'analizzatore condiviso: {errore}"
    );
    // La correzione riguarda la sola fase: gli altri assi non si muovono.
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Wkb, "{errore}");
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "{errore}"
    );
    assert_eq!(
        errore.retry,
        plenora_io_model::RetryDisposition::Never,
        "{errore}"
    );
    assert_eq!(
        errore.remote_effect,
        plenora_io_model::RemoteEffect::None,
        "{errore}"
    );
}

/// Lo spool su file rifiuta una cella oltre il tetto, e dichiara la lettura.
///
/// # Quale delle due righe e' in esame
///
/// L'**ispezione**, non la decodifica sotto di essa. `next_row` chiama
/// `inspect_wkb` con gli stessi limiti e solo dopo, se le dimensioni non
/// coincidono, decodifica: un tetto superato ferma percio' l'ispezione, e la
/// decodifica non gira. Correggere la sola decodifica avrebbe lasciato scoperta
/// l'unica delle due che si raggiunge.
///
/// # Perche' la prova e' interna
///
/// Perche' il ramo `File` dello spool si apre oltre i 64 MB, che e' una costante
/// del driver e non un'opzione della riga di comando: dal percorso pubblico
/// servirebbe una fixture di quella misura. `with_memory_limit` esiste per
/// questo, ed e' gia' usata dalla prova sullo spill.
#[test]
fn lo_spool_su_file_dichiara_la_fase_di_lettura_sul_tetto_per_cella() {
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
    assert!(
        matches!(storage, DxfSpoolStorage::File(_)),
        "la premessa: il ramo in esame e' quello su file"
    );
    let mut reader = storage.reader().unwrap();

    // Un WKB di Point misura ventuno byte: cinque non bastano.
    let stretti = WkbLimits {
        max_cell_bytes: 5,
        ..WkbLimits::default()
    };
    let Err(errore) = reader.next_row(CoordinateDimensions::Xy, &stretti) else {
        unreachable!("il tetto per cella deve rifiutare");
    };

    assert_eq!(
        errore.phase,
        plenora_io_model::ErrorPhase::Read,
        "lo spool si sta scorrendo: valeva `validate` finche' la fase la          dichiarava l'analizzatore condiviso: {errore}"
    );
    // La correzione riguarda la sola fase: gli altri assi non si muovono.
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Wkb, "{errore}");
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "{errore}"
    );
    assert_eq!(
        errore.retry,
        plenora_io_model::RetryDisposition::Never,
        "{errore}"
    );
    assert_eq!(
        errore.remote_effect,
        plenora_io_model::RemoteEffect::None,
        "{errore}"
    );
}
